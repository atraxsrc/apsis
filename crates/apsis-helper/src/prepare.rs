// SPDX-License-Identifier: GPL-3.0-only

//! `Restore`'s preparation (PLAN 6b.4, 6b.9, 6b.13 step 3 item 5): the checks again, both dry
//! runs and the space checks, the safety snapshot, the plan files in the state folder and the
//! recovery note on the backup disk. Then the plan is ready ([`crate::state::Running::ready`]).

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use apsis_core::native::{Cancel, QuietRunner, TIMESHIFT_DIR};
use apsis_core::restore::filter::{self, Home};
use apsis_core::restore::plan::{Plan, SeparateHome};
use apsis_core::restore::refusal::Refusal;
use apsis_core::restore::{argv, esp, file, recover, space, state as restore_state};
use apsis_core::usage::fstype_at;
use apsis_core::{Backend, DiskUsage, Error, Result, Runner, parse_snapshot_name};

use crate::check::{self, Live, SnapshotFiles};
use crate::native::{self, FINDMNT_ROOT_UUID, MOUNT_POINT};
use crate::runner::{DirectRunner, SAFE_PATH};
use crate::state::State;
use crate::usage;

/// What `Restore(snapshot, restore_home, safety_snapshot)` asked for, and who asked.
#[derive(Debug, Clone)]
pub struct Request {
    pub snapshot: String,
    pub home: Home,
    pub safety_snapshot: bool,
    pub starter_uid: u32,
}

/// The safety snapshot's comment: "Before restoring <the snapshot's date as the list shows
/// it>" (PLAN 6b.4).
#[must_use]
pub fn safety_comment(snapshot: &str) -> String {
    let date = parse_snapshot_name(snapshot).map_or_else(
        || snapshot.to_owned(),
        |t| t.strftime("%Y-%m-%d %H:%M").to_string(),
    );
    format!("Before restoring {date}")
}

/// What each destination partition must have free, margin included (`request.json`'s
/// `root_needs` and the separate home's `needs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    pub root: u64,
    /// A separate `/home` being restored.
    pub home: Option<u64>,
}

/// The space checks of PLAN 6b.4 for the partitions the restore writes to: `transfer` is the
/// restore's dry-run size, `under_home` the part of it under `/home` when `/home` is its own
/// mount and restored (`home` is then its `statvfs`). `/` is checked first.
///
/// # Errors
///
/// [`Refusal::SystemSpace`] for the first short partition, with its numbers.
pub fn needs(
    transfer: u64,
    under_home: Option<u64>,
    root: &DiskUsage,
    home: Option<&DiskUsage>,
) -> Result<Needs, Refusal> {
    let (on_root, on_home) = space::split(transfer, under_home.filter(|_| home.is_some()));
    let root_needs = space::system_needs(on_root, root.total);
    space::check_system(root_needs, root.free)?;
    let home_needs = match (on_home, home) {
        (Some(part), Some(usage)) => {
            let needs = space::system_needs(part, usage.total);
            space::check_system(needs, usage.free)?;
            Some(needs)
        }
        _ => None,
    };
    Ok(Needs {
        root: root_needs,
        home: home_needs,
    })
}

/// The state folder before a preparation: made root-only if it's missing, and cleared of
/// what an earlier preparation or arm left (the plan, the filter, rsync's log, `state.json`,
/// the ESP backup, a helper copy). The last `result.json` stays: `RestoreResult` reads it.
///
/// # Errors
///
/// The folder can't be made or a leftover can't be removed.
pub fn clear_leftovers(dir: &Path) -> io::Result<()> {
    if fs::symlink_metadata(dir).is_err() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    for name in [
        apsis_core::restore::plan::FILE,
        filter::FILE,
        HOME_FILTER_FILE,
        argv::LOG_FILE,
        restore_state::STATE_FILE,
        "apsis-helper",
    ] {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    match fs::remove_dir_all(dir.join(esp::BACKUP_DIR)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The second dry run's filter, written next to the real one while it runs and removed after.
const HOME_FILTER_FILE: &str = "restore-home.filter";

/// The whole preparation (PLAN 6b.4's order: the refusals, both dry runs and space, the
/// safety snapshot, the plan). Runs under the write lock as the `restore` job; `cancel` stops
/// it between steps and inside rsync. On `Ok`, `request.json`, `restore.filter` and the
/// recovery note are on disk and the plan is ready for `RestartToRestore`.
///
/// # Errors
///
/// [`Error::RestoreRefused`] with the refusal's word; [`Error::Stopped`]; what the mount,
/// rsync or a file write reported.
pub fn prepare(request: &Request, state: &Arc<State>, cancel: Arc<Cancel>) -> Result<Plan> {
    let dir = Path::new(file::DIR);
    let refused = |refusal: Refusal| Error::RestoreRefused(refusal.to_wire());
    let stopped = || {
        if cancel.is_stopping() {
            Err(Error::Stopped)
        } else {
            Ok(())
        }
    };
    clear_leftovers(dir)?;
    let (backend, _mounted) =
        native::open_for_restore(&DirectRunner, log, request.home == Home::Restore)?;
    let list = backend.list()?;
    if !list.snapshots.iter().any(|s| s.name == request.snapshot) {
        return Err(Error::NoSuchSnapshot(request.snapshot.clone()));
    }
    let backup_uuid = backend
        .config()
        .device_uuid
        .clone()
        .ok_or_else(|| Error::Helper("the backup device has no UUID".to_owned()))?;
    // The checks again (the dialog's were a while ago).
    let snapshot_dir = check::snapshot_dir(&backend.config().repo, &request.snapshot);
    let files = SnapshotFiles::read(&snapshot_dir);
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")?;
    let live = Live::read(Path::new("/"), &DirectRunner, &mountinfo)?;
    let dialog = check::dialog(&live, &files);
    if let Some(refusal) = dialog.refusal {
        return Err(refused(refusal));
    }
    let info = files.info.as_ref().ok_or_else(|| {
        refused(Refusal::Unreadable(
            apsis_core::restore::refusal::Unreadable::NoInfo,
        ))
    })?;
    let excludes = files.excludes.as_deref().unwrap_or_default();
    let running_kernel = fs::read_to_string("/proc/sys/kernel/osrelease")?
        .trim()
        .to_owned();
    stopped()?;

    // The filter, and the restore's dry run.
    let rules = filter::rules(&filter::Request {
        mountinfo: &mountinfo,
        home: request.home,
        protected_kernel: Some(&running_kernel),
        snapshot_excludes: excludes,
        restore_root: dialog.has_root,
    })?;
    filter::save(dir, &rules).map_err(file_error)?;
    let localhost = snapshot_dir.join("localhost");
    let runner = QuietRunner::new(SAFE_PATH).low_priority();
    let transfer = dry_run(
        &runner,
        &argv::rsync_dry_run(
            &localhost,
            Path::new("/"),
            &dir.join(filter::FILE),
            dialog.old_format,
        ),
        &cancel,
    )?
    .map_err(refused)?;
    stopped()?;

    // A separate /home being restored: what lands there, and its UUID.
    let home_is_separate =
        request.home == Home::Restore && fstype_at(&mountinfo, Path::new("/home")).is_some();
    let (under_home, home_uuid, home_usage) = if home_is_separate {
        let home_filter = dir.join(HOME_FILTER_FILE);
        fs::write(&home_filter, filter::to_text(&filter::home_only(&rules)))?;
        let measured = dry_run(
            &runner,
            &argv::rsync_dry_run(&localhost, Path::new("/"), &home_filter, dialog.old_format),
            &cancel,
        );
        let _ = fs::remove_file(&home_filter);
        let under_home = measured?.map_err(refused)?;
        let uuid = mount_uuid(&DirectRunner, "/home")?;
        let usage = usage::of_mount_point(Path::new("/home"))
            .ok_or_else(|| Error::Helper("statvfs of /home failed".to_owned()))?;
        (Some(under_home), Some(uuid), Some(usage))
    } else {
        (None, None, None)
    };
    stopped()?;
    let root_usage = usage::of_mount_point(Path::new("/"))
        .ok_or_else(|| Error::Helper("statvfs of / failed".to_owned()))?;
    let needs = needs(transfer, under_home, &root_usage, home_usage.as_ref()).map_err(refused)?;

    let repo = backend.config().repo.clone();
    // The safety snapshot: its own dry run and space check first, then the create.
    let safety_snapshot = if request.safety_snapshot {
        let comment = safety_comment(&request.snapshot);
        let create = backend.plan(&comment)?;
        let mut dry = create.argv.clone();
        dry.extend(["--dry-run", "--no-human-readable"].map(OsString::from));
        let size = dry_run(&runner, &dry, &cancel)?.map_err(refused)?;
        let backup_usage = usage::of_mount_point(Path::new(MOUNT_POINT))
            .ok_or_else(|| Error::Helper("statvfs of the backup disk failed".to_owned()))?;
        space::check_backup(space::backup_needs(size), backup_usage.free).map_err(refused)?;
        stopped()?;
        let name = Arc::new(Mutex::new(None::<String>));
        let (named, progress) = (Arc::clone(&name), Arc::clone(state));
        let backend = backend
            .with_cancel(Arc::clone(&cancel))
            .with_named(move |n| {
                *named
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(n.to_owned());
            })
            .with_progress(move |p| progress.progress(&p));
        backend.create(&comment)?;
        let name = name
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| Error::Helper("the safety snapshot has no name".to_owned()))?;
        Some(name)
    } else {
        None
    };
    stopped()?;

    // The recovery note on the backup disk, then the plan.
    let esp_uuid = mount_uuid(&DirectRunner, "/boot/efi")?;
    let note = recover::text(
        &live.root_uuid,
        &esp_uuid,
        &backup_uuid,
        &request.snapshot,
        dialog.old_format,
    );
    fs::write(repo.join(TIMESHIFT_DIR).join(recover::FILE), note)?;
    let prepared_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let plan = Plan {
        snapshot: request.snapshot.clone(),
        snapshot_created: info.created,
        backup_uuid,
        home: request.home,
        old_format: dialog.old_format,
        safety_snapshot,
        root_uuid: live.root_uuid.clone(),
        running_kernel,
        root_needs: needs.root,
        separate_home: match (home_uuid, needs.home) {
            (Some(uuid), Some(needs)) => Some(SeparateHome { uuid, needs }),
            _ => None,
        },
        starter_uid: request.starter_uid,
        prepared_at,
    };
    plan.save(dir).map_err(file_error)?;
    Ok(plan)
}

/// Runs an rsync dry run (`--stats`) and reads its size: `Ok(Err(SizeUnknown))` when the
/// output has no readable size. The output is collected from the stream: the helper's runner
/// hands it over piece by piece and keeps none of it in `RunOutput::stdout`.
fn dry_run(
    runner: &impl Runner,
    argv: &[OsString],
    cancel: &Arc<Cancel>,
) -> Result<Result<u64, Refusal>> {
    log(&format!(
        "running {}",
        apsis_core::native::shell_words(argv)
    ));
    let mut stdout = String::new();
    let streamed = runner.run_cancellable(
        argv,
        &mut |segment| {
            stdout.push_str(segment);
            stdout.push('\n');
            false
        },
        cancel,
    );
    let output = match streamed {
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(Error::Stopped),
        other => other?,
    };
    if cancel.is_stopping() {
        return Err(Error::Stopped);
    }
    // 23 and 24: some files couldn't be read or vanished; the count is still a count.
    if !output.success && !matches!(output.code, Some(23 | 24)) {
        return Err(Error::Native(format!(
            "rsync dry run exited with code {}: {}",
            output.code.map_or("none".to_owned(), |c| c.to_string()),
            output.stderr.trim()
        )));
    }
    Ok(space::dry_run_size(&stdout))
}

/// The filesystem UUID of what's mounted at `point` (`findmnt`).
fn mount_uuid(runner: &impl Runner, point: &str) -> Result<String> {
    let argv: Vec<OsString> = [&FINDMNT_ROOT_UUID[..], &[point]]
        .concat()
        .iter()
        .map(Into::into)
        .collect();
    let output = runner.run(&argv)?;
    let uuid = output.stdout.trim();
    if !output.success || uuid.is_empty() {
        return Err(Error::Helper(format!(
            "findmnt gave no UUID for {point}: {}",
            output.stderr.trim()
        )));
    }
    Ok(uuid.to_owned())
}

fn file_error(error: file::FileError) -> Error {
    Error::Helper(error.to_string())
}

fn log(line: &str) {
    eprintln!("apsis-helper: {line}");
}

#[cfg(test)]
mod tests {
    use std::fs;

    use apsis_core::DiskUsage;
    use apsis_core::restore::refusal::Refusal;

    use super::*;

    const GIB: u64 = 1 << 30;

    /// Behaves like the helper's `QuietRunner`: rsync's standard output is handed to the
    /// callback piece by piece and `RunOutput::stdout` stays empty.
    struct Streaming(&'static str);

    impl apsis_core::Runner for Streaming {
        fn run(&self, _argv: &[std::ffi::OsString]) -> std::io::Result<apsis_core::RunOutput> {
            unreachable!("a dry run streams")
        }

        fn run_streaming(
            &self,
            _argv: &[std::ffi::OsString],
            on_segment: &mut dyn FnMut(&str) -> bool,
        ) -> std::io::Result<apsis_core::RunOutput> {
            for line in self.0.lines() {
                on_segment(line);
            }
            Ok(apsis_core::RunOutput {
                success: true,
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    /// Found in check 1 (2026-10-02): the quiet runner keeps no stdout, so the size must be
    /// read from the stream, or every restore is refused as `size-unknown`.
    #[test]
    fn the_dry_run_reads_the_size_from_the_stream() {
        let stats = "Number of files: 5 (reg: 2, dir: 3)\n\
Total file size: 1228800 bytes\n\
Total transferred file size: 1228800 bytes\n\
sent 1,228,900 bytes  received 50 bytes\n";
        let runner = Streaming(stats);
        let argv: Vec<std::ffi::OsString> = vec!["rsync".into(), "--dry-run".into()];
        let cancel = apsis_core::native::Cancel::new();
        assert_eq!(dry_run(&runner, &argv, &cancel).unwrap(), Ok(1_228_800));
        let none = Streaming("Number of files: 5\n");
        assert_eq!(
            dry_run(&none, &argv, &cancel).unwrap(),
            Err(Refusal::SizeUnknown)
        );
    }

    #[test]
    fn the_safety_snapshots_comment_names_the_date() {
        assert_eq!(
            safety_comment("2026-09-25_11-28-53"),
            "Before restoring 2026-09-25 11:28"
        );
    }

    fn usage(total: u64, free: u64) -> DiskUsage {
        DiskUsage {
            total,
            used: total - free,
            free,
        }
    }

    /// `/` alone: the transfer plus the margin (1 GiB, or 2% of the disk); short refuses with
    /// the system-disk line.
    #[test]
    fn the_root_partition_needs_the_transfer_and_the_margin() {
        let root = usage(100 * GIB, 10 * GIB);
        let alone = needs(5 * GIB, None, &root, None).unwrap();
        assert_eq!(alone.root, 5 * GIB + 2 * GIB, "2% of 100 GiB beats 1 GiB");
        assert_eq!(alone.home, None);
        let short = usage(100 * GIB, 6 * GIB);
        assert_eq!(
            needs(5 * GIB, None, &short, None),
            Err(Refusal::SystemSpace {
                needs: 7 * GIB,
                free: 6 * GIB
            })
        );
    }

    /// A separate `/home` being restored: the part under `/home` is checked against it, the
    /// rest against `/`, and `/` is checked first.
    #[test]
    fn a_separate_home_is_checked_on_its_own_partition() {
        let root = usage(40 * GIB, 10 * GIB);
        let home = usage(200 * GIB, 12 * GIB);
        let both = needs(8 * GIB, Some(6 * GIB), &root, Some(&home)).unwrap();
        assert_eq!(both.root, 2 * GIB + GIB);
        assert_eq!(both.home, Some(6 * GIB + 4 * GIB));
        // Short /home refuses with the system-disk line and its own numbers.
        let short_home = usage(200 * GIB, 9 * GIB);
        assert_eq!(
            needs(8 * GIB, Some(6 * GIB), &root, Some(&short_home)),
            Err(Refusal::SystemSpace {
                needs: 10 * GIB,
                free: 9 * GIB
            })
        );
        // Short / is refused first.
        let short_root = usage(40 * GIB, GIB);
        assert_eq!(
            needs(8 * GIB, Some(6 * GIB), &short_root, Some(&short_home)),
            Err(Refusal::SystemSpace {
                needs: 3 * GIB,
                free: GIB
            })
        );
        // A home part larger than the whole (a file changed between the dry runs) is capped.
        let capped = needs(8 * GIB, Some(9 * GIB), &root, Some(&home)).unwrap();
        assert_eq!(capped.root, GIB);
    }

    /// A preparation starts from a clean state folder: a plan, filter, log, state or ESP
    /// backup left by an earlier one goes; the last result stays for `RestoreResult`.
    #[test]
    fn leftovers_of_an_earlier_preparation_are_cleared() {
        let dir = std::env::temp_dir().join(format!("apsis-prepare-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("esp-backup")).unwrap();
        for name in [
            "request.json",
            "restore.filter",
            "rsync-log",
            "state.json",
            "result.json",
            "apsis-helper",
            "esp-backup/vmlinuz.efi",
        ] {
            fs::write(dir.join(name), "x").unwrap();
        }
        clear_leftovers(&dir).unwrap();
        let left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["result.json"]);
        // A missing folder is made, root-only.
        fs::remove_dir_all(&dir).unwrap();
        clear_leftovers(&dir).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
