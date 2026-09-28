// SPDX-License-Identifier: GPL-3.0-only

//! The backup device's disk usage for `ListWithUsage` and `NativeListWithUsage`: `statvfs` on
//! a mount of it, or on a brief read-only mount when nothing has it mounted.
//!
//! Timeshift unmounts its `/run/timeshift/<pid>/backup` when it exits (`exit_app`), so after a
//! `timeshift --list` the device is only found if something else has it mounted (the desktop's
//! automount, `/` in btrfs mode). A `statvfs` on a path that isn't the device's mount would
//! describe another filesystem, so the mount is looked up in `/proc/self/mountinfo` first. If
//! there is none, the device is mounted `ro,nosuid,nodev,noexec` at [`MOUNT_POINT`] for the
//! `statvfs` and unmounted again, as for browsing; Timeshift's `X GB free` line is the last
//! resort.

use std::fs;
use std::path::{Path, PathBuf};

use apsis_core::usage::{is_plain_uuid, mount_points};
use apsis_core::{DiskUsage, Runner, SnapshotList};

use crate::native::{self, MOUNT_POINT};
use rustix::fs::{FileType, major, minor, stat, statvfs};

/// `statvfs` on `path`, which the caller knows is the device's mount point.
pub fn of_mount_point(path: &Path) -> Option<DiskUsage> {
    let vfs = statvfs(path).ok()?;
    DiskUsage::from_statvfs(vfs.f_blocks, vfs.f_bfree, vfs.f_bavail, vfs.f_frsize)
}

/// Where the disk usage came from, for the journal.
pub fn of_timeshift_list<R: Runner + Clone>(
    runner: &R,
    list: &SnapshotList,
) -> (Option<DiskUsage>, String) {
    if let Some((point, usage)) = of_listed_device(list) {
        return (
            Some(usage),
            format!("statvfs of {} (already mounted)", point.display()),
        );
    }
    let Some(uuid) = list.uuid.as_deref().filter(|u| is_plain_uuid(u)) else {
        return (
            None,
            "no UUID to mount by: Timeshift's free line only".to_owned(),
        );
    };
    match native::mount_listed(runner, uuid) {
        Ok(mounted) => {
            let usage = of_mount_point(Path::new(MOUNT_POINT));
            drop(mounted);
            let source = if usage.is_some() {
                format!("statvfs of a brief read-only mount at {MOUNT_POINT}")
            } else {
                format!("statvfs of {MOUNT_POINT} failed: Timeshift's free line only")
            };
            (usage, source)
        }
        Err(error) => (
            None,
            format!("brief read-only mount failed ({error}): Timeshift's free line only"),
        ),
    }
}

/// The usage of the device `list` names (by UUID, else by its `/dev` path), and where it's
/// mounted, if it's mounted somewhere now. `None` if it isn't, or can't be told apart from
/// another filesystem.
pub fn of_listed_device(list: &SnapshotList) -> Option<(PathBuf, DiskUsage)> {
    let node = device_node(list)?;
    let node = fs::canonicalize(node).ok()?;
    let st = stat(&node).ok()?;
    if FileType::from_raw_mode(st.st_mode) != FileType::BlockDevice {
        return None;
    }
    let rdev = (major(st.st_rdev), minor(st.st_rdev));
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let is_device = |source: &Path| fs::canonicalize(source).is_ok_and(|s| s == node);
    let point = mount_points(&mountinfo, rdev, is_device)
        .into_iter()
        .next()?;
    let usage = of_mount_point(&point)?;
    Some((point, usage))
}

/// `/dev/disk/by-uuid/<uuid>`, or the listed `/dev/...` path when there's no UUID.
fn device_node(list: &SnapshotList) -> Option<PathBuf> {
    match (&list.uuid, &list.device) {
        (Some(uuid), _) if is_plain_uuid(uuid) => Some(Path::new("/dev/disk/by-uuid").join(uuid)),
        (Some(_), _) => None,
        (None, Some(device)) if device.starts_with("/dev/") => Some(PathBuf::from(device)),
        (None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_nodes_come_from_plain_uuids_or_dev_paths() {
        let list = |uuid: Option<&str>, device: Option<&str>| SnapshotList {
            uuid: uuid.map(str::to_owned),
            device: device.map(str::to_owned),
            ..SnapshotList::default()
        };
        assert_eq!(
            device_node(&list(Some("abcd-1234"), Some("/dev/sdb1"))),
            Some(PathBuf::from("/dev/disk/by-uuid/abcd-1234"))
        );
        assert_eq!(
            device_node(&list(None, Some("/dev/sdb1"))),
            Some(PathBuf::from("/dev/sdb1"))
        );
        // Nothing that could leave /dev/disk/by-uuid, and no guessing past a bad UUID.
        assert_eq!(
            device_node(&list(Some("../../etc"), Some("/dev/sdb1"))),
            None
        );
        assert_eq!(device_node(&list(None, Some("sdb1"))), None);
        assert_eq!(device_node(&list(None, None)), None);
    }

    #[test]
    fn a_device_that_is_not_there_has_no_usage() {
        let list = SnapshotList {
            uuid: Some("00000000-0000-0000-0000-000000000000".to_owned()),
            ..SnapshotList::default()
        };
        assert_eq!(of_listed_device(&list), None);
        // No such device for lsblk either: the free line, and nothing mounted.
        let (usage, source) = of_timeshift_list(&NoLsblk, &list);
        assert_eq!(usage, None);
        assert!(source.contains("free line"), "{source}");
    }

    /// A runner that fails whatever it's asked to run (no `lsblk`, no `mount`).
    #[derive(Clone)]
    struct NoLsblk;

    impl Runner for NoLsblk {
        fn run(&self, _: &[std::ffi::OsString]) -> std::io::Result<apsis_core::RunOutput> {
            Err(std::io::ErrorKind::NotFound.into())
        }
    }

    #[test]
    fn a_mount_point_has_usage() {
        // Whatever filesystem the test runs on has a size.
        let usage = of_mount_point(Path::new("/")).expect("statvfs /");
        assert!(usage.total > 0 && usage.used <= usage.total);
    }
}
