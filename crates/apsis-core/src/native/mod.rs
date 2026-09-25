// SPDX-License-Identifier: GPL-3.0-only

//! The native rsync backend: Timeshift's rsync snapshots without running `timeshift`.
//!
//! It reads and writes the same layout Timeshift does, so either tool can list, use and delete
//! the other's snapshots. Everything copied from Timeshift's source is referenced in
//! `docs/DECISIONS.md` (Phase 5) and at each place below. Under the backup device's mount:
//!
//! ```text
//! timeshift/
//!   snapshots/
//!     2026-09-25_11-28-53/          local time the snapshot was started
//!       localhost/                  the copy of `/`
//!       exclude.list                rsync filters it was taken with
//!       rsync-log                   rsync's --log-file
//!       info.json                   see `info`
//!   snapshots-ondemand/2026-09-25_11-28-53 -> ../snapshots/2026-09-25_11-28-53
//!   snapshots-{boot,hourly,daily,weekly,monthly}/  the same, per tag
//!   apsis-staging/                  Apsis only: a native create in progress
//! ```
//!
//! One deliberate difference: Timeshift builds a new snapshot in place, protected by its own
//! lock. Timeshift's lock ignores any process that isn't called `timeshift`, and a scheduled
//! Timeshift run removes every snapshot folder without a readable `info.json` or
//! `exclude.list` as incomplete. So Apsis builds in `timeshift/apsis-staging/<name>/` and
//! renames the finished folder into `snapshots/`, where it appears complete in one step.

pub mod distro;
pub mod exclude;
pub mod info;
mod runner;

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use jiff::Zoned;

pub use self::info::{INFO_FILE, Info};
pub use self::runner::QuietRunner;
use crate::backend::Backend;
use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
use crate::timeshift::{Runner, validate_comment};

/// Timeshift's folder on the backup device, in rsync mode (`SnapshotRepo.vala:159-168`).
pub const TIMESHIFT_DIR: &str = "timeshift";
/// Snapshots, inside [`TIMESHIFT_DIR`] (`SnapshotRepo.vala:170-174`).
pub const SNAPSHOTS_DIR: &str = "snapshots";
/// Where a native create builds a snapshot before it's complete, inside [`TIMESHIFT_DIR`].
/// Apsis's own; Timeshift doesn't look here.
pub const STAGING_DIR: &str = "apsis-staging";
/// Skipped when Timeshift lists `snapshots/` (`SnapshotRepo.vala:310`).
const SYNC_DIR: &str = ".sync";
/// The copy of `/` inside a snapshot (`Main.vala:1473-1474`).
pub const LOCALHOST_DIR: &str = "localhost";
/// The filters a snapshot was taken with (`Main.vala:896`). Timeshift counts a rsync
/// snapshot without one as incomplete (`Snapshot.vala:291-315`).
pub const EXCLUDE_FILE: &str = "exclude.list";
/// rsync's `--log-file` (`Main.vala:1538`).
pub const RSYNC_LOG_FILE: &str = "rsync-log";
/// What `app-version` says in a snapshot Apsis made (Timeshift writes its own version there).
pub const APP_VERSION: &str = concat!("apsis ", env!("CARGO_PKG_VERSION"));
/// The tags `create_symlinks` keeps a folder for, in its order (`SnapshotRepo.vala:929-940`).
const SYMLINK_TAGS: [Tag; 6] = [
    Tag::Boot,
    Tag::Hourly,
    Tag::Daily,
    Tag::Weekly,
    Tag::Monthly,
    Tag::OnDemand,
];
/// rsync exit codes a snapshot survives: 23 (some files couldn't be read) and 24 (files
/// vanished while copying) are normal on a running system. Timeshift doesn't check the exit
/// code at all, only that rsync reported a total size (`Main.vala:1577-1581`).
const RSYNC_OK_CODES: [i32; 3] = [0, 23, 24];

/// What the native backend works with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeConfig {
    /// Where the backup device is mounted. Snapshots are in `<repo>/timeshift/snapshots/`.
    pub repo: PathBuf,
    /// The backup device, for [`SnapshotList::device`].
    pub device: Option<String>,
    /// Its filesystem UUID, for [`SnapshotList::uuid`].
    pub device_uuid: Option<String>,
    /// What's backed up: `/` for real.
    pub source: PathBuf,
    /// Filesystem UUID of the system's `/`, for `sys-uuid`.
    pub sys_uuid: String,
    /// For `sys-distro` (see [`distro::full_name`]).
    pub sys_distro: String,
    /// The rsync filters (see [`exclude::for_backup`]).
    pub exclude: Vec<String>,
    /// [`Backend::create`] only logs its [`CreatePlan`], and touches nothing.
    pub dry_run: bool,
}

/// [`Backend`] that reads and writes Timeshift's rsync snapshots itself.
pub struct NativeRsync<R> {
    config: NativeConfig,
    runner: R,
    clock: Box<dyn Fn() -> Zoned + Send + Sync>,
    log: Box<dyn Fn(&str) + Send + Sync>,
}

impl<R: Runner> NativeRsync<R> {
    /// `runner` runs rsync. The clock is the system's local time; the log goes nowhere.
    pub fn new(config: NativeConfig, runner: R) -> Self {
        Self {
            config,
            runner,
            clock: Box::new(Zoned::now),
            log: Box::new(|_| {}),
        }
    }

    /// Takes the time from `clock` instead (a snapshot's name is its local start time).
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> Zoned + Send + Sync + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Sends what it does (and a dry run's plan) to `log`, one message at a time.
    #[must_use]
    pub fn with_log(mut self, log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.log = Box::new(log);
        self
    }

    pub fn config(&self) -> &NativeConfig {
        &self.config
    }

    fn timeshift_dir(&self) -> PathBuf {
        self.config.repo.join(TIMESHIFT_DIR)
    }

    fn snapshots_dir(&self) -> PathBuf {
        self.timeshift_dir().join(SNAPSHOTS_DIR)
    }

    fn staging_dir(&self) -> PathBuf {
        self.timeshift_dir().join(STAGING_DIR)
    }

    /// Refuses to write through a symlink: the folders a create or delete writes in must be
    /// real folders on the backup device (or not there yet), so nothing lands elsewhere.
    fn check_folders(&self) -> Result<()> {
        for dir in [
            self.timeshift_dir(),
            self.snapshots_dir(),
            self.staging_dir(),
        ] {
            match fs::symlink_metadata(&dir) {
                Ok(meta) if !meta.is_dir() => {
                    return Err(Error::Native(format!(
                        "{} is not a folder (a symlink?); not writing through it",
                        dir.display()
                    )));
                }
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
        Ok(())
    }

    /// Every folder in `snapshots/`, read as Timeshift reads it.
    fn scan(&self) -> Result<Vec<Found>> {
        let dir = self.snapshots_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            // `load_snapshots`: no folder, no snapshots.
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            // Timeshift asks GIO for the type, following symlinks.
            if name == SYNC_DIR || !fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            found.push(Found::read(
                &entry.path(),
                name.to_string_lossy().into_owned(),
            ));
        }
        // `load_snapshots` sorts by the date in info.json; the name breaks ties.
        found.sort_by(|a, b| {
            let created = |f: &Found| f.info.as_ref().map_or(0, |i| i.created);
            created(a)
                .cmp(&created(b))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(found)
    }

    /// What a create would do now, without doing any of it.
    ///
    /// # Errors
    ///
    /// An invalid comment, a snapshot folder that's already there, or the repository can't be
    /// read.
    pub fn plan(&self, comment: &str) -> Result<CreatePlan> {
        let comment = validate_comment(comment)?;
        self.check_folders()?;
        let now = (self.clock)();
        let name = now.strftime("%Y-%m-%d_%H-%M-%S").to_string();
        let target = self.snapshots_dir().join(&name);
        let staging = self.staging_dir().join(&name);
        for path in [&target, &staging] {
            if fs::symlink_metadata(path).is_ok() {
                return Err(Error::Native(format!(
                    "{} already exists; try again in a second",
                    path.display()
                )));
            }
        }
        // `get_latest_snapshot("", sys_uuid)`: the newest valid snapshot of this system
        // (`SnapshotRepo.vala:372-402`, `Main.vala:1510-1520`).
        let link_from = self
            .scan()?
            .into_iter()
            .rev()
            .find(|f| {
                f.is_valid()
                    && f.info
                        .as_ref()
                        .is_some_and(|i| i.sys_uuid == self.config.sys_uuid)
            })
            .map(|f| f.path.join(LOCALHOST_DIR));
        let exclude = exclude::to_text(&self.config.exclude);
        let info = Info {
            created: now.timestamp().as_second(),
            sys_uuid: self.config.sys_uuid.clone(),
            sys_distro: self.config.sys_distro.clone(),
            app_version: APP_VERSION.to_owned(),
            file_count: 0,
            tags: vec![Tag::OnDemand.word().to_owned()],
            comments: comment.to_owned(),
            live: false,
            kind: "rsync".to_owned(),
        };
        let argv = rsync_argv(&self.config.source, &staging, link_from.as_deref());
        Ok(CreatePlan {
            name,
            staging,
            target,
            link_from,
            exclude,
            argv,
            info,
        })
    }

    /// Carries out `plan`: builds the snapshot in the staging folder, then moves it into
    /// `snapshots/` and updates the tag folders.
    fn execute(&self, plan: &CreatePlan) -> Result<()> {
        fs::create_dir_all(self.snapshots_dir())?;
        fs::create_dir_all(plan.staging.join(LOCALHOST_DIR))?;
        let built = self.build(plan);
        if let Err(error) = built {
            (self.log)(&format!(
                "removing the unfinished {}",
                plan.staging.display()
            ));
            if let Err(cleanup) = fs::remove_dir_all(&plan.staging) {
                (self.log)(&format!("couldn't remove it: {cleanup}"));
            }
            let _ = fs::remove_dir(self.staging_dir());
            return Err(error);
        }
        // `rename` would replace an empty folder; `plan` checked, this checks again.
        if fs::symlink_metadata(&plan.target).is_ok() {
            return Err(Error::Native(format!(
                "{} appeared while copying; the snapshot is left in {}",
                plan.target.display(),
                plan.staging.display()
            )));
        }
        fs::rename(&plan.staging, &plan.target)?;
        File::open(self.snapshots_dir())?.sync_all()?;
        let _ = fs::remove_dir(self.staging_dir());
        (self.log)(&format!("created {}", plan.target.display()));
        self.update_symlinks()
    }

    /// Everything that happens inside the staging folder.
    fn build(&self, plan: &CreatePlan) -> Result<()> {
        write_synced(&plan.staging.join(EXCLUDE_FILE), &plan.exclude)?;
        (self.log)(&format!("running {}", shell_words(&plan.argv)));
        let output = self.runner.run(&plan.argv)?;
        let code = output.code.unwrap_or(-1);
        let stderr = output.stderr.trim();
        if !RSYNC_OK_CODES.contains(&code) {
            return Err(Error::Native(format!(
                "rsync exited with {}: {stderr}",
                output
                    .code
                    .map_or("a signal".to_owned(), |c| format!("code {c}"))
            )));
        }
        if code != 0 {
            (self.log)(&format!("rsync exited with code {code} (kept): {stderr}"));
        }
        let log = fs::read(plan.staging.join(RSYNC_LOG_FILE))?;
        // `file_line_count`: bytes that are `\n` (`TeeJee.FileSystem.vala:91-97`).
        #[allow(clippy::naive_bytecount, reason = "once per snapshot; no crate for it")]
        let lines = log.iter().filter(|&&b| b == b'\n').count();
        if total_size(&String::from_utf8_lossy(&log)).unwrap_or(0) == 0 {
            // Timeshift's own check (`Main.vala:1577-1581`).
            return Err(Error::Native(format!(
                "rsync reported no total size (exit code {code}): {stderr}"
            )));
        }
        let info = Info {
            file_count: lines as u64,
            ..plan.info.clone()
        };
        write_synced(&plan.staging.join(INFO_FILE), &info.to_text())
    }

    /// `create_symlinks` (`SnapshotRepo.vala:929-991`): each tag folder is emptied and
    /// re-created, then every valid snapshot gets `snapshots-<tag>/<name> ->
    /// ../snapshots/<name>` for each of its tags.
    fn update_symlinks(&self) -> Result<()> {
        let timeshift = self.timeshift_dir();
        let tag_dir = |tag: Tag| timeshift.join(format!("{SNAPSHOTS_DIR}-{}", tag.word()));
        for tag in SYMLINK_TAGS {
            let dir = tag_dir(tag);
            match fs::remove_dir_all(&dir) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
            fs::create_dir_all(&dir)?;
        }
        for found in self.scan()?.iter().filter(|f| f.is_valid()) {
            for tag in found.info.iter().flat_map(Info::known_tags) {
                let link = tag_dir(tag).join(&found.name);
                symlink(format!("../{SNAPSHOTS_DIR}/{}", found.name), link)?;
            }
        }
        Ok(())
    }
}

impl<R: Runner> Backend for NativeRsync<R> {
    /// Reads every `info.json` directly. Folders Timeshift would count as incomplete, and a
    /// native create's leftovers, are warnings.
    fn list(&self) -> Result<SnapshotList> {
        let mut warnings = Vec::new();
        let mut snapshots = Vec::new();
        for found in self.scan()? {
            if let Some(problem) = found.problem() {
                warnings.push(format!("{}: {problem}", found.name));
                continue;
            }
            let (Some(created), Some(info)) = (parse_snapshot_name(&found.name), found.info) else {
                warnings.push(format!("{}: not a snapshot name", found.name));
                continue;
            };
            snapshots.push(Snapshot {
                name: found.name,
                created,
                tags: info.known_tags(),
                comment: Some(info.comments).filter(|c| !c.is_empty()),
            });
        }
        if let Ok(entries) = fs::read_dir(self.staging_dir()) {
            for entry in entries.flatten() {
                warnings.push(format!(
                    "{}: left over from an interrupted native create, in {}",
                    entry.file_name().to_string_lossy(),
                    self.staging_dir().display()
                ));
            }
        }
        Ok(SnapshotList {
            device: self.config.device.clone(),
            uuid: self.config.device_uuid.clone(),
            mode: Some(Mode::Rsync),
            snapshots,
            warnings,
        })
    }

    /// In dry-run mode, logs the [`CreatePlan`] and changes nothing.
    fn create(&self, comment: &str) -> Result<()> {
        let plan = self.plan(comment)?;
        if self.config.dry_run {
            (self.log)(&plan.to_string());
            return Ok(());
        }
        self.execute(&plan)
    }

    /// Removes the snapshot's folder, then updates the tag folders.
    fn delete(&self, name: &str) -> Result<()> {
        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.to_owned()));
        }
        self.check_folders()?;
        let path = self.snapshots_dir().join(name);
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            return Err(Error::NoSuchSnapshot(name.to_owned()));
        }
        fs::remove_dir_all(&path)?;
        (self.log)(&format!("deleted {}", path.display()));
        self.update_symlinks()
    }
}

/// A folder in `snapshots/`, and what Timeshift would make of it.
struct Found {
    name: String,
    path: PathBuf,
    /// `None`: no `info.json`, or one Timeshift can't read.
    info: Option<Info>,
    info_exists: bool,
    has_exclude: bool,
}

impl Found {
    fn read(path: &Path, name: String) -> Self {
        let text = fs::read_to_string(path.join(INFO_FILE));
        Self {
            name,
            path: path.to_owned(),
            info_exists: text.is_ok(),
            info: text.ok().as_deref().and_then(Info::parse),
            has_exclude: path.join(EXCLUDE_FILE).exists(),
        }
    }

    /// Why Timeshift would count it as incomplete (`Snapshot.vala:190-213`, `:282-287`, `:291-315`).
    fn problem(&self) -> Option<&'static str> {
        if !self.info_exists {
            Some("incomplete: no info.json")
        } else if self.info.is_none() {
            Some("incomplete: info.json can't be read")
        } else if !self.has_exclude {
            Some("incomplete: no exclude.list")
        } else {
            None
        }
    }

    fn is_valid(&self) -> bool {
        self.problem().is_none()
    }
}

/// Everything a native create will do, worked out before any of it happens. Its text is the
/// dry run's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    /// `YYYY-MM-DD_HH-MM-SS`, local time.
    pub name: String,
    /// Where it's built: `timeshift/apsis-staging/<name>`.
    pub staging: PathBuf,
    /// Where it ends up: `timeshift/snapshots/<name>`.
    pub target: PathBuf,
    /// `--link-dest`: the newest snapshot of this system's `localhost/`, if there is one.
    pub link_from: Option<PathBuf>,
    /// `exclude.list`'s text.
    pub exclude: String,
    /// rsync's argv, run without a shell.
    pub argv: Vec<OsString>,
    /// `info.json`, with `file_count` still 0: it's the line count of `rsync-log`, known once
    /// rsync is done.
    pub info: Info,
}

impl fmt::Display for CreatePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exclude_lines = self.exclude.lines().count();
        writeln!(f, "dry run: native create {}, nothing written", self.name)?;
        writeln!(f, "mkdir -p {}", self.staging.join(LOCALHOST_DIR).display())?;
        writeln!(
            f,
            "write {} ({exclude_lines} lines):",
            self.staging.join(EXCLUDE_FILE).display()
        )?;
        for line in self.exclude.lines() {
            writeln!(f, "  {line}")?;
        }
        match &self.link_from {
            Some(from) => writeln!(f, "link unchanged files to {}", from.display())?,
            None => writeln!(f, "no earlier snapshot of this system: full copy")?,
        }
        writeln!(f, "run (argv, no shell): {}", shell_words(&self.argv))?;
        writeln!(
            f,
            "write {} (file_count: lines in rsync-log once rsync is done):",
            self.staging.join(INFO_FILE).display()
        )?;
        for line in self.info.to_text().lines() {
            writeln!(f, "  {line}")?;
        }
        writeln!(
            f,
            "rename {} -> {}",
            self.staging.display(),
            self.target.display()
        )?;
        write!(
            f,
            "rebuild snapshots-{{boot,hourly,daily,weekly,monthly,ondemand}}/, adding \
             snapshots-ondemand/{0} -> ../snapshots/{0}",
            self.name
        )
    }
}

/// Timeshift's rsync command for a new snapshot (`RsyncTask.build_script`,
/// `RsyncTask.vala:175-255`, with the options `create_snapshot_with_rsync` sets,
/// `Main.vala:1538-1559`), as an argv instead of a shell script. `--delete-excluded` is there
/// twice, as in Timeshift's. The source is `source` with a trailing `/` (Timeshift: `/`), the
/// destination `<snapshot>/localhost/`.
#[must_use]
pub fn rsync_argv(source: &Path, snapshot: &Path, link_from: Option<&Path>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = [
        "rsync",
        "-aii",
        "--recursive",
        "--verbose",
        "--delete",
        "--force",
        "--stats",
        "--sparse",
        "--delete-excluded",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    let with_slash = |path: &Path| {
        let mut text = path.as_os_str().to_owned();
        if !text.as_encoded_bytes().ends_with(b"/") {
            text.push("/");
        }
        text
    };
    let option = |name: &str, value: OsString| {
        let mut arg = OsString::from(name);
        arg.push(value);
        arg
    };
    if let Some(from) = link_from {
        argv.push(option("--link-dest=", with_slash(from)));
    }
    argv.push(option(
        "--log-file=",
        snapshot.join(RSYNC_LOG_FILE).into_os_string(),
    ));
    argv.push(option(
        "--exclude-from=",
        snapshot.join(EXCLUDE_FILE).into_os_string(),
    ));
    argv.push("--delete-excluded".into());
    argv.push(with_slash(source));
    argv.push(with_slash(&snapshot.join(LOCALHOST_DIR)));
    argv
}

/// The last `total size is N  speedup is X` in rsync's output (Timeshift's `TotalSize` regex,
/// `RsyncTask.vala:140-141`, `:534-535`), commas removed.
fn total_size(log: &str) -> Option<u64> {
    log.lines().rev().find_map(|line| {
        let (_, rest) = line.split_once("total size is ")?;
        let (number, rest) = rest.split_once([' ', '\t'])?;
        rest.trim_start().starts_with("speedup is ").then_some(())?;
        let digits: String = number.chars().filter(|&c| c != ',').collect();
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    })
}

/// Writes `text` to a new file at `path` and flushes it to disk.
fn write_synced(path: &Path, text: &str) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// The argv as a shell would need it typed, for logs: words with anything unusual in single
/// quotes.
#[must_use]
pub fn shell_words(argv: &[OsString]) -> String {
    argv.iter()
        .map(|arg| {
            let arg = arg.to_string_lossy();
            let plain = !arg.is_empty()
                && arg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_=/.,:+@%".contains(c));
            if plain {
                arg.into_owned()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_matches_timeshifts_script() {
        let argv = rsync_argv(
            Path::new("/"),
            Path::new("/mnt/timeshift/snapshots/2026-09-25_11-28-53"),
            Some(Path::new(
                "/mnt/timeshift/snapshots/2026-09-24_10-00-00/localhost",
            )),
        );
        let s = "/mnt/timeshift/snapshots/2026-09-25_11-28-53";
        assert_eq!(
            argv,
            [
                "rsync",
                "-aii",
                "--recursive",
                "--verbose",
                "--delete",
                "--force",
                "--stats",
                "--sparse",
                "--delete-excluded",
                "--link-dest=/mnt/timeshift/snapshots/2026-09-24_10-00-00/localhost/",
                &format!("--log-file={s}/rsync-log"),
                &format!("--exclude-from={s}/exclude.list"),
                "--delete-excluded",
                "/",
                &format!("{s}/localhost/"),
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn first_snapshot_has_no_link_dest() {
        let argv = rsync_argv(Path::new("/src"), Path::new("/s/x"), None);
        assert!(
            !argv
                .iter()
                .any(|a| a.to_string_lossy().starts_with("--link-dest"))
        );
        assert_eq!(argv[argv.len() - 2], "/src/");
    }

    #[test]
    fn total_size_like_timeshifts_regex() {
        let log = "2026/09/25 19:47:13 [6] sent 174 bytes  received 57 bytes  462.00 bytes/sec\n\
            2026/09/25 19:47:13 [6] total size is 1,234,567  speedup is 0.01\n";
        assert_eq!(total_size(log), Some(1_234_567));
        assert_eq!(total_size("total size is 0  speedup is 0.00"), Some(0));
        assert_eq!(total_size("building file list\n"), None);
    }

    #[test]
    fn shell_words_quote_what_needs_it() {
        let argv = ["rsync", "--link-dest=/a b/", "it's"].map(OsString::from);
        assert_eq!(shell_words(&argv), r"rsync '--link-dest=/a b/' 'it'\''s'");
    }
}
