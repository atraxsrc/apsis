// SPDX-License-Identifier: GPL-3.0-only

//! File-level restore as the helper runs it: the backup device mounted read-only (and
//! `noexec`) at [`MOUNT_POINT`] for one call, the running system at `/`, the caller from the
//! bus. The rules are in `apsis_core::restore`.

use std::fs;
use std::io;
use std::path::Path;

use apsis_core::restore::{
    self, Caller, Listing, Names, Plan, Request, Restore, RsyncRunner, SnapPath, device_of_mount,
    open_snapshot,
};
use apsis_core::{Error, Result};

use crate::native::{self, MOUNT_POINT};
use crate::runner::{DirectRunner, SAFE_PATH};

/// Where a folder-mode dry run "copies" to: never made (rsync's `--dry-run` doesn't create
/// it), and removed first if an empty one is left over.
pub const DRY_RUN_TARGET: &str = "/run/apsis/restore-dry-run";

/// Longest part of each path that goes into the journal, and how many paths.
const LOGGED_PATH_CHARS: usize = 60;
const LOGGED_PATHS: usize = 3;

/// One folder of `snapshot`, compared with the running system.
///
/// # Errors
///
/// See `apsis_core::restore::browse`; also mounting the backup device.
pub fn browse(snapshot: &str, path: &str) -> Result<Listing> {
    let path = SnapPath::parse(path)?;
    let _mounted = native::mount_backup(&DirectRunner)?;
    let root = open_snapshot(Path::new(MOUNT_POINT), snapshot)?;
    restore::browse(&root, &path, Path::new("/"), &names())
}

/// Runs `request` for the caller `uid`.
///
/// # Errors
///
/// See `apsis_core::restore::Restore::run`; also mounting the backup device, and a caller
/// without an `/etc/passwd` entry.
pub fn run(request: &Request, uid: u32) -> Result<Plan> {
    let caller = Caller::from_passwd(&fs::read_to_string("/etc/passwd")?, uid)?;
    let _mounted = native::mount_backup(&DirectRunner)?;
    let backup_dev = device_of_mount(Path::new(MOUNT_POINT))?;
    match fs::remove_dir(DRY_RUN_TARGET) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(Error::Restore(format!("{DRY_RUN_TARGET}: {e}")));
        }
        _ => {}
    }
    let runner = RsyncRunner::new(SAFE_PATH);
    Restore {
        repo: Path::new(MOUNT_POINT),
        live_root: Path::new("/"),
        backup_dev: Some(backup_dev),
        dry_run_target: Path::new(DRY_RUN_TARGET),
        runner: &runner,
    }
    .run(request, &caller)
}

/// User and group names of the running system. Missing files mean numbers only.
fn names() -> Names {
    let read = |path| fs::read_to_string(path).unwrap_or_default();
    Names::parse(&read("/etc/passwd"), &read("/etc/group"))
}

/// A path as it goes into the journal: quoted, cut to [`LOGGED_PATH_CHARS`].
pub fn logged_path(path: &str) -> String {
    let mut short: String = path.chars().take(LOGGED_PATH_CHARS).collect();
    if short.len() < path.len() {
        short.push('…');
    }
    format!("{short:?}")
}

/// `["/etc/fstab", "/etc/hosts", +3 more]`
pub fn logged_paths(paths: &[String]) -> String {
    let mut shown: Vec<String> = paths
        .iter()
        .take(LOGGED_PATHS)
        .map(|p| logged_path(p))
        .collect();
    if let Some(more) = paths.len().checked_sub(LOGGED_PATHS).filter(|&n| n > 0) {
        shown.push(format!("+{more} more"));
    }
    format!("[{}]", shown.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_paths_are_short() {
        let paths: Vec<String> = ["/a", "/b", "/c", "/d", "/e"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(logged_paths(&paths), r#"["/a", "/b", "/c", +2 more]"#);
        assert_eq!(logged_paths(&paths[..1]), r#"["/a"]"#);
        let long = format!("/{}", "x".repeat(200));
        let logged = logged_path(&long);
        assert_eq!(
            logged.chars().filter(|&c| c == 'x').count(),
            LOGGED_PATH_CHARS - 1
        );
        assert!(logged.ends_with("…\""));
        // Newlines in names stay on one journal line.
        assert_eq!(logged_path("/a\nb"), "\"/a\\nb\"");
    }

    #[test]
    fn the_dry_run_target_is_under_the_helpers_run_folder() {
        assert!(DRY_RUN_TARGET.starts_with("/run/apsis/"));
        assert!(!Path::new(DRY_RUN_TARGET).starts_with(MOUNT_POINT));
    }
}
