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
//!   apsis-staging/                  Apsis only: a native create or delete in progress
//! ```
//!
//! One deliberate difference: Timeshift builds a new snapshot in place, protected by its own
//! lock. Timeshift's lock ignores any process that isn't called `timeshift`, and a scheduled
//! Timeshift run removes every snapshot folder without a readable `info.json` or
//! `exclude.list` as incomplete. So Apsis builds in `timeshift/apsis-staging/<name>/` and
//! renames the finished folder into `snapshots/`, where it appears complete in one step.

mod cancel;
pub mod distro;
pub mod exclude;
pub mod info;
pub mod prune;
pub(crate) mod runner;

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Zoned;

pub use self::cancel::{Cancel, GRACE, TooLate};
pub use self::info::{INFO_FILE, Info, RSYNC_FLAGS, RSYNC_FLAGS_KEY};
pub use self::runner::QuietRunner;
use crate::backend::Backend;
use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
use crate::progress::{Progress, parse_rsync};
use crate::runner::{Runner, validate_comment};
use crate::usage::mounts_under;

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
    progress: Box<dyn Fn(Progress) + Send + Sync>,
    named: Box<dyn Fn(&str) + Send + Sync>,
    mountinfo: Box<dyn Fn() -> io::Result<String> + Send + Sync>,
    cancel: Arc<Cancel>,
}

/// The mounts a delete checks for, as the kernel lists them for this process.
const MOUNTINFO: &str = "/proc/self/mountinfo";

impl<R: Runner> NativeRsync<R> {
    /// `runner` runs rsync. The clock is the system's local time; the log goes nowhere.
    pub fn new(config: NativeConfig, runner: R) -> Self {
        Self {
            config,
            runner,
            clock: Box::new(Zoned::now),
            log: Box::new(|_| {}),
            progress: Box::new(|_| {}),
            named: Box::new(|_| {}),
            mountinfo: Box::new(|| fs::read_to_string(MOUNTINFO)),
            cancel: Cancel::new(),
        }
    }

    /// A create stops when `cancel` is asked to (see [`Cancel`]).
    #[must_use]
    pub fn with_cancel(mut self, cancel: Arc<Cancel>) -> Self {
        self.cancel = cancel;
        self
    }

    /// Tells `named` a create's snapshot name as soon as it's planned.
    #[must_use]
    pub fn with_named(mut self, named: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.named = Box::new(named);
        self
    }

    /// Reads the mount table from `mountinfo` instead of [`MOUNTINFO`] (for the tests).
    #[must_use]
    pub fn with_mountinfo(
        mut self,
        mountinfo: impl Fn() -> io::Result<String> + Send + Sync + 'static,
    ) -> Self {
        self.mountinfo = Box::new(mountinfo);
        self
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

    /// Sends rsync's `--info=progress2` progress to `progress` while a create copies (only
    /// with a runner that streams, like [`QuietRunner`]).
    #[must_use]
    pub fn with_progress(mut self, progress: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        self.progress = Box::new(progress);
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
            rsync_flags: Some(RSYNC_FLAGS.to_owned()),
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
        (self.named)(&plan.name);
        fs::create_dir_all(self.snapshots_dir())?;
        // Space comes back before the new copy starts.
        self.remove_leftovers();
        fs::create_dir_all(plan.staging.join(LOCALHOST_DIR))?;
        let built = self.build(plan).and_then(|()| {
            if self.cancel.commit() {
                Ok(())
            } else {
                Err(Error::Stopped)
            }
        });
        if let Err(error) = built {
            (self.log)(&format!(
                "removing the unfinished {}",
                plan.staging.display()
            ));
            if let Err(cleanup) = self.remove_staging(&plan.name) {
                (self.log)(&format!("couldn't remove it: {cleanup}"));
            }
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
        let output = self.runner.run_cancellable(
            &plan.argv,
            &mut |segment| {
                parse_rsync(segment)
                    .map(|progress| (self.progress)(progress))
                    .is_some()
            },
            &self.cancel,
        );
        let output = match output {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                return Err(Error::Stopped);
            }
            other => other?,
        };
        // Stopped while rsync ran: whatever it managed goes.
        if self.cancel.is_stopping() {
            return Err(Error::Stopped);
        }
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

    /// Deletes the snapshot `name`, as root, and only it:
    ///
    /// - `name` must be a snapshot name (`YYYY-MM-DD_HH-MM-SS`), so it can't be `snapshots/`
    ///   itself, `..` or a path;
    /// - `timeshift/`, `snapshots/` and `<name>/` are each opened with `O_NOFOLLOW`: a symlink
    ///   anywhere on the way is refused;
    /// - `<name>/info.json` must be a regular file: a folder that isn't a snapshot is refused;
    /// - nothing may be mounted at or below `<name>/` (`/proc/self/mountinfo`): a bind mount
    ///   has the same device number, so the walk alone couldn't tell;
    /// - `apsis-staging/` is opened with `O_NOFOLLOW` too (made if it isn't there), and the
    ///   folder is moved there with `RENAME_NOREPLACE` before anything is removed. A name
    ///   already taken there, or a filesystem without `RENAME_NOREPLACE`, refuses the delete:
    ///   there is no plain rename to fall back to. Both folders are flushed after the move;
    /// - then its links in `snapshots-<tag>/` go, as Timeshift's delete leaves no dangling link;
    ///   nothing else there is touched;
    /// - then the folder is removed with [`prune::remove_at`] (never follows a symlink, never
    ///   leaves the filesystem), and `apsis-staging/` if that's empty now.
    ///
    /// Cut anywhere after the move, what's left is a leftover in `apsis-staging/` (listed, and
    /// removed by Delete or the next create), never a half-removed snapshot in `snapshots/`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidSnapshotName`], [`Error::NoSuchSnapshot`], [`Error::InvalidInput`] (the
    /// refusals above, before anything is moved or deleted), or the delete failed part-way.
    pub fn delete_snapshot(&self, name: &str) -> Result<()> {
        use rustix::fs::{
            AtFlags, FileType, Mode, OFlags, RenameFlags, fsync, mkdirat, open, openat,
            renameat_with, statat, unlinkat,
        };
        use rustix::io::Errno;

        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.to_owned()));
        }
        let refuse = |why: &str| Error::InvalidInput(format!("not deleting {name}: {why}"));
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let folder = |errno: Errno, what: &str| match errno {
            Errno::LOOP | Errno::NOTDIR => refuse(&format!("{what} is a symlink or not a folder")),
            Errno::NOENT => refuse(&format!("there is no {what}")),
            other => Error::Io(io::Error::from(other)),
        };
        let repo = open(&self.config.repo, flags, Mode::empty())
            .map_err(|e| folder(e, "backup device folder"))?;
        let timeshift = openat(&repo, TIMESHIFT_DIR, flags, Mode::empty())
            .map_err(|e| folder(e, "timeshift/"))?;
        let snapshots = openat(&timeshift, SNAPSHOTS_DIR, flags, Mode::empty())
            .map_err(|e| folder(e, "timeshift/snapshots/"))?;
        let snapshot = match openat(&snapshots, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Err(Error::NoSuchSnapshot(name.to_owned())),
            Err(errno) => return Err(folder(errno, "the snapshot folder")),
        };
        let info = statat(&snapshot, INFO_FILE, AtFlags::SYMLINK_NOFOLLOW);
        if !info.is_ok_and(|st| FileType::from_raw_mode(st.st_mode) == FileType::RegularFile) {
            return Err(refuse("it has no info.json, so it isn't a snapshot folder"));
        }
        drop(snapshot);
        let path = fs::canonicalize(self.snapshots_dir().join(name))?;
        let mounts = mounts_under(&(self.mountinfo)()?, &path);
        if !mounts.is_empty() {
            let list: Vec<String> = mounts.iter().map(|m| m.display().to_string()).collect();
            return Err(refuse(&format!(
                "something is mounted inside it ({}); unmount it first",
                list.join(", ")
            )));
        }
        match mkdirat(&timeshift, STAGING_DIR, Mode::from_raw_mode(0o755)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(errno) => return Err(Error::Io(io::Error::from(errno))),
        }
        let staging = openat(&timeshift, STAGING_DIR, flags, Mode::empty())
            .map_err(|e| folder(e, "timeshift/apsis-staging/"))?;
        // Any error, `EINVAL` (no `RENAME_NOREPLACE` here) included, refuses: nothing moved.
        if let Err(errno) = renameat_with(&snapshots, name, &staging, name, RenameFlags::NOREPLACE)
        {
            drop(staging);
            let _ = unlinkat(&timeshift, STAGING_DIR, AtFlags::REMOVEDIR);
            return Err(refuse(&match errno {
                Errno::EXIST => format!("apsis-staging/{name} is already there"),
                other => format!("can't move it into apsis-staging/ ({other})"),
            }));
        }
        (self.log)(&format!("moved {name} to {STAGING_DIR}/"));
        // The move is on disk before the first removal.
        fsync(&snapshots).map_err(|e| Error::Io(io::Error::from(e)))?;
        fsync(&staging).map_err(|e| Error::Io(io::Error::from(e)))?;
        self.remove_tag_links(name)?;
        prune::remove_at(&staging, &self.staging_dir(), name)?;
        drop(staging);
        // Only if it's empty now; anything else there stays.
        let _ = unlinkat(&timeshift, STAGING_DIR, AtFlags::REMOVEDIR);
        (self.log)(&format!(
            "deleted {}",
            self.snapshots_dir().join(name).display()
        ));
        Ok(())
    }

    /// Removes the staging folder `apsis-staging/<name>`, as root, with the delete's rules:
    ///
    /// - `name` must be a snapshot name;
    /// - `timeshift/`, `apsis-staging/` and `<name>/` are opened with `O_NOFOLLOW`: a symlink
    ///   anywhere on the way is refused;
    /// - nothing may be mounted at or below it (`/proc/self/mountinfo`);
    /// - it's removed with [`prune::remove_at`] (never follows a symlink, never leaves the
    ///   filesystem), then `apsis-staging/` itself if that's empty now.
    ///
    /// Not there (never made, or gone already) is fine.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidSnapshotName`], [`Error::InvalidInput`] (the refusals above, before
    /// anything is removed), or the removal failed part-way.
    pub fn remove_staging(&self, name: &str) -> Result<()> {
        use rustix::fs::{AtFlags, FileType, Mode, OFlags, open, openat, statat, unlinkat};
        use rustix::io::Errno;

        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.to_owned()));
        }
        let refuse =
            |why: &str| Error::InvalidInput(format!("not removing the unfinished {name}: {why}"));
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let folder = |errno: Errno, what: &str| match errno {
            Errno::LOOP | Errno::NOTDIR => refuse(&format!("{what} is a symlink or not a folder")),
            other => Error::Io(io::Error::from(other)),
        };
        let repo = open(&self.config.repo, flags, Mode::empty())
            .map_err(|e| folder(e, "backup device folder"))?;
        let timeshift = match openat(&repo, TIMESHIFT_DIR, flags, Mode::empty()) {
            Err(Errno::NOENT) => return Ok(()),
            other => other.map_err(|e| folder(e, "timeshift/"))?,
        };
        let staging = match openat(&timeshift, STAGING_DIR, flags, Mode::empty()) {
            Err(Errno::NOENT) => return Ok(()),
            other => other.map_err(|e| folder(e, "timeshift/apsis-staging/"))?,
        };
        match statat(&staging, name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(Errno::NOENT) => {}
            Err(errno) => return Err(Error::Io(io::Error::from(errno))),
            Ok(st) if FileType::from_raw_mode(st.st_mode) != FileType::Directory => {
                return Err(refuse("it is a symlink or not a folder"));
            }
            Ok(_) => {
                let path = fs::canonicalize(self.staging_dir().join(name))?;
                let mounts = mounts_under(&(self.mountinfo)()?, &path);
                if !mounts.is_empty() {
                    let list: Vec<String> =
                        mounts.iter().map(|m| m.display().to_string()).collect();
                    return Err(refuse(&format!(
                        "something is mounted inside it ({}); unmount it first",
                        list.join(", ")
                    )));
                }
                prune::remove_at(&staging, &self.staging_dir(), name)?;
                (self.log)(&format!(
                    "removed {}",
                    self.staging_dir().join(name).display()
                ));
            }
        }
        drop(staging);
        // Only if it's empty now; anything else there stays.
        let _ = unlinkat(&timeshift, STAGING_DIR, AtFlags::REMOVEDIR);
        Ok(())
    }

    /// Interrupted creates' and deletes' folders in `apsis-staging/`: real folders with a
    /// snapshot name, oldest first. Anything else there is a warning (and is never removed).
    fn leftovers(&self) -> (Vec<String>, Vec<String>) {
        let mut names = Vec::new();
        let mut warnings = Vec::new();
        let dir = self.staging_dir();
        if !fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            return (names, warnings);
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            return (names, warnings);
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir());
            if is_dir && parse_snapshot_name(&name).is_some() {
                names.push(name);
            } else {
                eprintln!("apsis: not Apsis's, left alone: {}", entry.path().display());
                warnings.push(format!(
                    "{name:?} in the unfinished snapshots folder isn't Apsis's; left alone"
                ));
            }
        }
        names.sort();
        (names, warnings)
    }

    /// Removes every interrupted create's or delete's folder in `apsis-staging/` (see
    /// [`NativeRsync::remove_staging`]); never anything in `snapshots/`. A refusal is logged
    /// and the rest go on.
    fn remove_leftovers(&self) {
        for name in self.leftovers().0 {
            let started = parse_snapshot_name(&name)
                .map(|t| t.strftime("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();
            (self.log)(&format!(
                "removing leftover {} (an interrupted create or delete, started {started})",
                self.staging_dir().join(&name).display()
            ));
            if let Err(error) = self.remove_staging(&name) {
                (self.log)(&format!("couldn't remove it: {error}"));
            }
        }
    }

    /// Removes `snapshots-<tag>/<name>` for each tag folder, where that is a symlink. A tag
    /// folder that is itself a symlink, and anything that isn't a link, are left alone.
    fn remove_tag_links(&self, name: &str) -> Result<()> {
        for tag in SYMLINK_TAGS {
            let dir = self
                .timeshift_dir()
                .join(format!("{SNAPSHOTS_DIR}-{}", tag.word()));
            if !fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let link = dir.join(name);
            if fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
                fs::remove_file(&link)?;
            }
        }
        Ok(())
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
    /// Reads every `info.json` directly. [`SnapshotList::leftovers`] are the folders in
    /// `apsis-staging/` and the half-deleted ones in `snapshots/` (see `Found::half_deleted`);
    /// any other folder Timeshift would count as incomplete is a warning.
    fn list(&self) -> Result<SnapshotList> {
        let mut warnings = Vec::new();
        let mut snapshots = Vec::new();
        let mut half_deleted = Vec::new();
        for found in self.scan()? {
            if found.half_deleted() {
                half_deleted.push(found.name);
                continue;
            }
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
                rsync_flags: info.rsync_flags,
            });
        }
        let (mut leftovers, odd) = self.leftovers();
        warnings.extend(odd);
        leftovers.extend(half_deleted);
        leftovers.sort();
        leftovers.dedup();
        Ok(SnapshotList {
            device: self.config.device.clone(),
            uuid: self.config.device_uuid.clone(),
            mode: Some(Mode::Rsync),
            snapshots,
            warnings,
            leftovers,
            // The helper adds what `statvfs` says while the device is mounted.
            usage: None,
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

    /// Deletes one snapshot ([`NativeRsync::delete_snapshot`], which also takes a half-deleted
    /// folder in `snapshots/`), or one interrupted create's or delete's folder
    /// ([`NativeRsync::remove_staging`]) when `name` is in `apsis-staging/` only.
    fn delete(&self, name: &str) -> Result<()> {
        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.to_owned()));
        }
        // Everything else goes through the snapshot delete, with all its refusals.
        let is_snapshot = fs::symlink_metadata(self.snapshots_dir().join(name)).is_ok();
        if !is_snapshot && self.leftovers().0.iter().any(|n| n == name) {
            return self.remove_staging(name);
        }
        self.delete_snapshot(name)
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
    /// A real folder (not a symlink) whose `info.json` is a regular file: what
    /// [`NativeRsync::delete_snapshot`] asks for.
    deletable: bool,
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
            deletable: fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
                && fs::symlink_metadata(path.join(INFO_FILE)).is_ok_and(|m| m.is_file()),
        }
    }

    /// What a delete cut part-way in place leaves (an Apsis delete before fix 2, or perhaps
    /// Timeshift's): a snapshot name, a readable `info.json`, no `exclude.list` (the walk goes
    /// in `readdir` order). A leftover row that Delete removes; never the `--link-dest` base,
    /// never removed unasked. Whether Timeshift building a snapshot in place ever looks the
    /// same isn't known; it would be a row too, and Delete needs a click, a confirm and a
    /// password (PLAN 6b.9, left knowingly).
    fn half_deleted(&self) -> bool {
        parse_snapshot_name(&self.name).is_some()
            && self.deletable
            && self.info.is_some()
            && !self.has_exclude
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
/// `Main.vala:1538-1559`), as an argv instead of a shell script. Timeshift passes
/// `--delete-excluded` twice; once is enough (it's a flag, not an ordered rule). The source
/// is `source` with a trailing `/` (Timeshift: `/`), the destination `<snapshot>/localhost/`. Apsis adds `--info=progress2` (whole-transfer percent
/// and time left, on stdout, which Timeshift's log file doesn't get).
///
/// Since 0.4.1 Apsis also adds `-A -X --numeric-ids`: POSIX ACLs, extended attributes (file
/// capabilities among them) and owners by number, so a restore has everything it needs.
/// `-H` stays off, as in Timeshift: with `--link-dest` it costs a table of every
/// multiply-linked file on each run.
#[must_use]
pub fn rsync_argv(source: &Path, snapshot: &Path, link_from: Option<&Path>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = [
        "rsync",
        "-aii",
        "-A",
        "-X",
        "--numeric-ids",
        "--recursive",
        "--verbose",
        "--delete",
        "--force",
        "--stats",
        "--sparse",
        "--delete-excluded",
        "--info=progress2",
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
    argv.push(with_slash(source));
    argv.push(with_slash(&snapshot.join(LOCALHOST_DIR)));
    argv
}

/// `rsync_argv`'s command as a dry run that can go before the staging folder exists: the
/// exclude list is read from `exclude_file` instead of the staging copy, no log file is
/// written, and `--dry-run --no-human-readable` come last. rsync's dry run accepts a
/// destination that isn't there yet and creates nothing; its "Total transferred file size"
/// is what a `--link-dest` copy would add to the backup disk.
#[must_use]
pub fn dry_run_argv(argv: &[OsString], exclude_file: &Path) -> Vec<OsString> {
    let mut dry: Vec<OsString> = argv
        .iter()
        .filter(|arg| !arg.as_encoded_bytes().starts_with(b"--log-file="))
        .map(|arg| {
            if arg.as_encoded_bytes().starts_with(b"--exclude-from=") {
                let mut replaced = OsString::from("--exclude-from=");
                replaced.push(exclude_file);
                replaced
            } else {
                arg.clone()
            }
        })
        .collect();
    dry.extend(["--dry-run", "--no-human-readable"].map(OsString::from));
    dry
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
                "-A",
                "-X",
                "--numeric-ids",
                "--recursive",
                "--verbose",
                "--delete",
                "--force",
                "--stats",
                "--sparse",
                "--delete-excluded",
                "--info=progress2",
                "--link-dest=/mnt/timeshift/snapshots/2026-09-24_10-00-00/localhost/",
                &format!("--log-file={s}/rsync-log"),
                &format!("--exclude-from={s}/exclude.list"),
                "/",
                &format!("{s}/localhost/"),
            ]
            .map(OsString::from)
        );
    }

    /// The safety snapshot's dry run before the staging folder exists: the exclude list is
    /// read from elsewhere, nothing is logged, and nothing is written.
    #[test]
    fn dry_run_argv_reads_the_given_exclude_list_and_writes_no_log() {
        let argv = rsync_argv(Path::new("/"), Path::new("/s/x"), Some(Path::new("/s/w")));
        let dry = dry_run_argv(&argv, Path::new("/var/lib/apsis/restore/safety.exclude"));
        assert_eq!(
            dry,
            [
                "rsync",
                "-aii",
                "-A",
                "-X",
                "--numeric-ids",
                "--recursive",
                "--verbose",
                "--delete",
                "--force",
                "--stats",
                "--sparse",
                "--delete-excluded",
                "--info=progress2",
                "--link-dest=/s/w/",
                "--exclude-from=/var/lib/apsis/restore/safety.exclude",
                "/",
                "/s/x/localhost/",
                "--dry-run",
                "--no-human-readable",
            ]
            .map(OsString::from)
        );
    }

    /// `info.json` says what the argv does.
    #[test]
    fn argv_has_the_flags_info_json_names() {
        let argv = rsync_argv(Path::new("/"), Path::new("/s/x"), None);
        assert_eq!(RSYNC_FLAGS, "-aAX --numeric-ids");
        for flag in ["-A", "-X", "--numeric-ids"] {
            assert!(argv.iter().any(|a| a == flag), "{flag}");
        }
    }

    #[test]
    fn argv_never_preserves_hard_links() {
        let argv = rsync_argv(Path::new("/"), Path::new("/s/x"), Some(Path::new("/s/w")));
        for arg in &argv {
            let arg = arg.to_string_lossy();
            let short_flags = arg.starts_with('-') && !arg.starts_with("--");
            assert!(
                arg != "--hard-links" && !(short_flags && arg.contains('H')),
                "{arg}"
            );
        }
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
