// SPDX-License-Identifier: GPL-3.0-only

//! File-level restore (Phase 6a): browse a snapshot's files and copy chosen ones back. The
//! same for Timeshift's and Apsis's rsync snapshots (`timeshift/snapshots/<name>/localhost/`).
//!
//! - [`path`]: snapshot paths, checked lexically and resolved without following a symlink.
//! - [`browse`]: one folder of a snapshot, compared with the running system.
//! - [`plan`]: what rsync did or would do, from its itemized output.
//! - [`dest`]: folder mode's destination, held open so root never writes through a path the
//!   caller could change.
//! - [`Restore`]: runs it, dry run or for real.
//!
//! Nothing here needs root: the helper passes the backup device's mount, `/` and the caller;
//! the tests pass folders of their own.

pub mod browse;
pub mod dest;
pub mod path;
pub mod plan;
mod runner;

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub use browse::{Entry, Kind, Listing, Live, MAX_ENTRIES, Names, browse};
pub use dest::RESTORED_DIR;
pub use path::{SnapPath, SnapshotRoot, open_snapshot};
pub use plan::{Action, Destination, Item, Plan};
pub use runner::RsyncRunner;

use crate::error::{Error, Result};
use crate::timeshift::{RunOutput, Runner};

/// Most paths one restore takes.
pub const MAX_PATHS: usize = 1000;

/// Original mode keeps the item it replaces as `<name><this><snapshot>`.
pub const BACKUP_INFIX: &str = ".apsis-before-";

/// Top-level folders original mode never writes into: kernel and runtime filesystems, and the
/// boot partition (full-system territory, Phase 6b).
pub const REFUSED_TOP: [&str; 6] = ["proc", "sys", "dev", "run", "tmp", "boot"];

/// Top-level folders original mode warns about but allows.
pub const WARNED_TOP: [&str; 2] = ["etc", "usr"];

/// A restore to run: what, from where, to where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub snapshot: String,
    /// Absolute snapshot paths (`/etc/hosts`).
    pub paths: Vec<String>,
    pub destination: Destination,
    pub dry_run: bool,
}

/// Who asked: folder mode restores into their home, as them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

impl Caller {
    /// The caller `uid`, with their group and home from `/etc/passwd`'s text.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] if `passwd` has no such user.
    pub fn from_passwd(passwd: &str, uid: u32) -> Result<Self> {
        passwd
            .lines()
            .find_map(|line| {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() < 7 || fields[2].parse() != Ok(uid) {
                    return None;
                }
                Some(Caller {
                    uid,
                    gid: fields[3].parse().ok()?,
                    home: PathBuf::from(fields[5]),
                })
            })
            .ok_or_else(|| Error::InvalidInput(format!("no user with uid {uid} in /etc/passwd")))
    }
}

/// Where a restore runs.
#[derive(Debug)]
pub struct Restore<'a, R> {
    /// The backup device's mount (read-only in the helper).
    pub repo: &'a Path,
    /// The running system's `/`.
    pub live_root: &'a Path,
    /// `st_dev` of the backup device's filesystem: original mode refuses to write onto it.
    pub backup_dev: Option<u64>,
    /// A path that doesn't exist, which a folder-mode dry run "copies" into.
    pub dry_run_target: &'a Path,
    pub runner: &'a R,
}

/// The request's paths, checked: 1 to [`MAX_PATHS`], each a valid snapshot path, not `/`.
/// Sorted, duplicates dropped.
///
/// # Errors
///
/// [`Error::InvalidInput`] with the reason.
pub fn check_paths(paths: &[String]) -> Result<Vec<SnapPath>> {
    if paths.is_empty() {
        return Err(Error::InvalidInput("nothing to restore".to_owned()));
    }
    if paths.len() > MAX_PATHS {
        return Err(Error::InvalidInput(format!(
            "at most {MAX_PATHS} paths at a time"
        )));
    }
    let mut checked = paths
        .iter()
        .map(|p| SnapPath::parse(p))
        .collect::<Result<Vec<_>>>()?;
    if checked.iter().any(SnapPath::is_root) {
        return Err(Error::InvalidInput(
            "/ can't be restored file by file; that's a full-system restore".to_owned(),
        ));
    }
    checked.sort();
    checked.dedup();
    Ok(checked)
}

/// Why original mode refuses `path`, if it does (see [`REFUSED_TOP`]).
#[must_use]
pub fn refused_in_original(path: &SnapPath) -> Option<String> {
    let top = path.parts().first()?;
    REFUSED_TOP
        .contains(&top.as_str())
        .then(|| format!("{path}: /{top} is never restored to its original place"))
}

/// The warning original mode gives for `path`, if any (see [`WARNED_TOP`]).
#[must_use]
pub fn warning_in_original(path: &SnapPath) -> Option<String> {
    let top = path.parts().first()?;
    WARNED_TOP
        .contains(&top.as_str())
        .then(|| format!("{path}: /{top} is live system files; check the plan"))
}

/// rsync, common to both modes: archive with hard links, ACLs and xattrs, numeric owners, no
/// `--delete`, and the itemized output the plan is read from.
fn rsync_base() -> Vec<OsString> {
    ["rsync", "-aHAX", "--numeric-ids", plan::OUT_FORMAT]
        .map(OsString::from)
        .to_vec()
}

/// Folder mode's rsync: `--relative` rebuilds each path under the destination, nothing there
/// is overwritten, everything is the caller's, no setuid/setgid bits, no `security.*` xattrs
/// (file capabilities), no device nodes or FIFOs.
#[must_use]
pub fn folder_argv(
    sources: &[PathBuf],
    uid: u32,
    gid: u32,
    destination: &str,
    dry_run: bool,
) -> Vec<OsString> {
    let mut argv = rsync_base();
    argv.extend(
        [
            "--relative",
            "--ignore-existing",
            "--no-devices",
            "--no-specials",
            "--chmod=ug-s",
            "--filter=-x security.*",
        ]
        .map(OsString::from),
    );
    argv.push(format!("--chown={uid}:{gid}").into());
    if dry_run {
        argv.push("--dry-run".into());
    }
    argv.extend(sources.iter().map(|s| s.clone().into_os_string()));
    argv.push(destination.into());
    argv
}

/// Original mode's rsync for one path, into its live parent folder: what it replaces is kept
/// as `<name>.apsis-before-<snapshot>`.
#[must_use]
pub fn original_argv(
    source: &Path,
    live_parent: &Path,
    snapshot: &str,
    dry_run: bool,
) -> Vec<OsString> {
    let mut argv = rsync_base();
    argv.push("--backup".into());
    argv.push(format!("--suffix={BACKUP_INFIX}{snapshot}").into());
    if dry_run {
        argv.push("--dry-run".into());
    }
    argv.push(source.as_os_str().to_owned());
    let mut parent = live_parent.as_os_str().to_owned();
    parent.push("/");
    argv.push(parent);
    argv
}

impl<R: Runner> Restore<'_, R> {
    /// Runs `request` for `caller`: the plan (dry run) or what was done.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] for anything refused before rsync runs; [`Error::Restore`] when
    /// rsync fails.
    pub fn run(&self, request: &Request, caller: &Caller) -> Result<Plan> {
        let paths = check_paths(&request.paths)?;
        let snapshot = open_snapshot(self.repo, &request.snapshot)?;
        for path in &paths {
            path::resolve(&snapshot, path)?;
        }
        match request.destination {
            Destination::Folder => self.folder(&snapshot, &paths, caller, request.dry_run),
            Destination::Original => self.original(&snapshot, &paths, request.dry_run),
        }
    }

    fn folder(
        &self,
        snapshot: &SnapshotRoot,
        paths: &[SnapPath],
        caller: &Caller,
        dry_run: bool,
    ) -> Result<Plan> {
        // `localhost/./etc/hosts`: `--relative` keeps what's after the `/./`.
        let sources: Vec<PathBuf> = paths
            .iter()
            .map(|p| snapshot.localhost.join(".").join(p.relative()))
            .collect();
        let mut plan = Plan {
            snapshot: snapshot.name.clone(),
            destination: Destination::Folder,
            dry_run,
            target: String::new(),
            items: Vec::new(),
            warnings: Vec::new(),
            notes: Vec::new(),
            commands: Vec::new(),
        };
        let output = if dry_run {
            plan.target = dest::would_be(&caller.home, &snapshot.name)
                .display()
                .to_string();
            match fs::symlink_metadata(self.dry_run_target) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                _ => {
                    return Err(Error::Restore(format!(
                        "{} exists; a dry run needs it absent",
                        self.dry_run_target.display()
                    )));
                }
            }
            let target = format!("{}/", self.dry_run_target.display());
            let argv = folder_argv(&sources, caller.uid, caller.gid, &target, true);
            plan.commands.push(shown(&argv));
            self.runner.run(&argv)?
        } else {
            let pinned = dest::prepare(&caller.home, caller.uid, caller.gid, &snapshot.name)?;
            plan.target = pinned.path.display().to_string();
            let argv = folder_argv(&sources, caller.uid, caller.gid, &pinned.proc_path(), false);
            plan.commands.push(shown(&argv));
            let output = {
                let _inherited = pinned.inherited()?;
                self.runner.run(&argv)
            };
            // Handed over even if rsync failed, so the caller can look at or remove what's there.
            let handed = pinned.hand_over(caller.uid, caller.gid);
            let output = output?;
            handed?;
            output
        };
        check_rsync(&output)?;
        (plan.items, plan.notes) = plan::parse_itemized(&output.stdout, "", None);
        Ok(plan)
    }

    fn original(&self, snapshot: &SnapshotRoot, paths: &[SnapPath], dry_run: bool) -> Result<Plan> {
        let suffix = format!("{BACKUP_INFIX}{}", snapshot.name);
        let mut plan = Plan {
            snapshot: snapshot.name.clone(),
            destination: Destination::Original,
            dry_run,
            target: String::new(),
            items: Vec::new(),
            warnings: Vec::new(),
            notes: Vec::new(),
            commands: Vec::new(),
        };
        let mut jobs = Vec::new();
        for path in paths {
            if let Some(reason) = refused_in_original(path) {
                return Err(Error::InvalidInput(reason));
            }
            let parent = path::live_parent(self.live_root, path)?;
            if let Some(backup_dev) = self.backup_dev {
                let live = parent.join(path.name().unwrap_or_default());
                let on_backup = dest::device_of(&parent)? == backup_dev
                    || dest::device_of(&live).is_ok_and(|dev| dev == backup_dev);
                if on_backup {
                    return Err(Error::InvalidInput(format!(
                        "{path} is on the backup device itself; it can't be restored in place"
                    )));
                }
            }
            plan.warnings.extend(warning_in_original(path));
            let shown_parent = path.parent().unwrap_or_default();
            let prefix = if shown_parent.is_root() {
                String::new()
            } else {
                shown_parent.to_string()
            };
            jobs.push((path.under(&snapshot.localhost), parent, prefix));
        }
        // Always a dry run first: the plan, and the backup names the real run would take.
        for (source, parent, prefix) in &jobs {
            let argv = original_argv(source, parent, &snapshot.name, true);
            let output = self.runner.run(&argv)?;
            check_rsync(&output)?;
            let (items, notes) = plan::parse_itemized(&output.stdout, prefix, Some(&suffix));
            plan.items.extend(items);
            plan.notes.extend(notes);
            if dry_run {
                plan.commands.push(shown(&argv));
            }
        }
        let taken: Vec<String> = plan
            .items
            .iter()
            .filter_map(|item| item.backup.as_deref())
            .filter(|backup| {
                let live = SnapPath::parse(backup).map(|p| p.under(self.live_root));
                live.is_ok_and(|live| fs::symlink_metadata(live).is_ok())
            })
            .map(str::to_owned)
            .collect();
        if !taken.is_empty() {
            return Err(Error::InvalidInput(format!(
                "a backup from an earlier restore is in the way (rsync would overwrite it): {}; \
                 move it away first",
                taken.join(", ")
            )));
        }
        if dry_run {
            return Ok(plan);
        }
        plan.items.clear();
        plan.notes.clear();
        for (index, (source, parent, prefix)) in jobs.iter().enumerate() {
            let argv = original_argv(source, parent, &snapshot.name, false);
            plan.commands.push(shown(&argv));
            let output = self.runner.run(&argv)?;
            let (items, notes) = plan::parse_itemized(&output.stdout, prefix, Some(&suffix));
            plan.items.extend(items);
            plan.notes.extend(notes);
            if let Err(error) = check_rsync(&output) {
                let done = if index == 0 {
                    "nothing before it was restored".to_owned()
                } else {
                    format!("the {index} path(s) before it were restored")
                };
                return Err(Error::Restore(format!("{}: {error}; {done}", paths[index])));
            }
        }
        Ok(plan)
    }
}

/// rsync's exit code: 0 is success. 23 and 24 (some files couldn't be read, or vanished) are
/// failures too, with rsync's last lines.
fn check_rsync(output: &RunOutput) -> Result<()> {
    if output.success {
        return Ok(());
    }
    let what = match output.code {
        Some(23) => "rsync finished with errors (some files couldn't be copied)".to_owned(),
        Some(24) => "rsync finished with errors (some files vanished)".to_owned(),
        Some(code) => format!("rsync failed with exit code {code}"),
        None => "rsync was killed by a signal".to_owned(),
    };
    let tail = output.stderr.trim();
    Err(Error::Restore(if tail.is_empty() {
        what
    } else {
        format!("{what}: {}", tail.lines().collect::<Vec<_>>().join(" | "))
    }))
}

fn shown(argv: &[OsString]) -> Vec<String> {
    argv.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// `st_dev` of `path` (following symlinks: for the mount point).
///
/// # Errors
///
/// `stat` failed.
pub fn device_of_mount(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)?.dev())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(argv: &[OsString]) -> Vec<String> {
        shown(argv)
    }

    #[test]
    fn folder_mode_argv_is_safe_for_the_caller() {
        let argv = folder_argv(
            &[PathBuf::from(
                "/run/apsis/backup/timeshift/snapshots/S/localhost/./etc/hosts",
            )],
            1000,
            1000,
            "/proc/self/fd/7/",
            false,
        );
        assert_eq!(
            strings(&argv),
            [
                "rsync",
                "-aHAX",
                "--numeric-ids",
                "--out-format=APSIS %i %l %n%L",
                "--relative",
                "--ignore-existing",
                "--no-devices",
                "--no-specials",
                "--chmod=ug-s",
                "--filter=-x security.*",
                "--chown=1000:1000",
                "/run/apsis/backup/timeshift/snapshots/S/localhost/./etc/hosts",
                "/proc/self/fd/7/",
            ]
        );
        assert!(!strings(&argv).iter().any(|a| a.contains("delete")));
        let dry = strings(&folder_argv(&[], 1, 1, "/x/", true));
        assert!(dry.contains(&"--dry-run".to_owned()));
        // --no-devices/--no-specials come after -a, which turns them on.
        let pos = |arg: &str| dry.iter().position(|a| a == arg).unwrap();
        assert!(pos("-aHAX") < pos("--no-devices") && pos("-aHAX") < pos("--no-specials"));
    }

    #[test]
    fn original_mode_argv_backs_up_what_it_replaces() {
        let argv = original_argv(
            Path::new("/run/apsis/backup/timeshift/snapshots/S/localhost/etc/hosts"),
            Path::new("/etc"),
            "2026-09-25_03-00-01",
            true,
        );
        assert_eq!(
            strings(&argv),
            [
                "rsync",
                "-aHAX",
                "--numeric-ids",
                "--out-format=APSIS %i %l %n%L",
                "--backup",
                "--suffix=.apsis-before-2026-09-25_03-00-01",
                "--dry-run",
                "/run/apsis/backup/timeshift/snapshots/S/localhost/etc/hosts",
                "/etc/",
            ]
        );
    }

    #[test]
    fn paths_are_checked_sorted_and_deduplicated() {
        let paths = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let checked = check_paths(&paths(&["/etc/hosts", "/etc/fstab", "/etc/hosts"])).unwrap();
        let shown: Vec<String> = checked.iter().map(ToString::to_string).collect();
        assert_eq!(shown, ["/etc/fstab", "/etc/hosts"]);
        for bad in [
            paths(&[]),
            paths(&["/"]),
            paths(&["/etc/../shadow"]),
            paths(&["etc"]),
            vec!["/x".to_owned(); MAX_PATHS + 1],
        ] {
            assert!(
                matches!(check_paths(&bad), Err(Error::InvalidInput(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn original_mode_refuses_runtime_folders_and_warns_for_system_ones() {
        for top in REFUSED_TOP {
            let path = SnapPath::parse(&format!("/{top}/x")).unwrap();
            assert!(refused_in_original(&path).is_some(), "{top}");
        }
        for fine in ["/etc/hosts", "/home/u/x", "/var/lib/x", "/bootstrap"] {
            let path = SnapPath::parse(fine).unwrap();
            assert!(refused_in_original(&path).is_none(), "{fine}");
        }
        let etc = SnapPath::parse("/etc/hosts").unwrap();
        assert!(warning_in_original(&etc).is_some());
        assert!(warning_in_original(&SnapPath::parse("/usr/bin/x").unwrap()).is_some());
        assert!(warning_in_original(&SnapPath::parse("/opt/x").unwrap()).is_none());
    }

    #[test]
    fn callers_come_from_passwd() {
        let passwd =
            "root:x:0:0:root:/root:/bin/bash\nuser1:x:1000:1001:,,,:/home/user1:/bin/bash\n";
        assert_eq!(
            Caller::from_passwd(passwd, 1000).unwrap(),
            Caller {
                uid: 1000,
                gid: 1001,
                home: PathBuf::from("/home/user1")
            }
        );
        assert!(Caller::from_passwd(passwd, 1002).is_err());
    }

    #[test]
    fn rsync_errors_say_what_happened() {
        let output = |code, stderr: &str| RunOutput {
            success: false,
            code,
            stdout: String::new(),
            stderr: stderr.to_owned(),
        };
        let message = |o| match check_rsync(&o) {
            Err(Error::Restore(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(message(output(Some(23), "a\nb")).ends_with("couldn't be copied): a | b"));
        assert!(message(output(Some(24), "")).contains("vanished"));
        assert!(message(output(Some(11), "")).contains("exit code 11"));
        assert!(message(output(None, "")).contains("signal"));
    }
}
