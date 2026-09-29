// SPDX-License-Identifier: GPL-3.0-only

//! The backup device's disk usage for `NativeListWithUsage`: `statvfs` on
//! [`crate::native::MOUNT_POINT`] while the list has the device mounted there.

use std::path::Path;

use apsis_core::DiskUsage;
use rustix::fs::statvfs;

/// `statvfs` on `path`, which the caller knows is the device's mount point.
pub fn of_mount_point(path: &Path) -> Option<DiskUsage> {
    let vfs = statvfs(path).ok()?;
    DiskUsage::from_statvfs(vfs.f_blocks, vfs.f_bfree, vfs.f_bavail, vfs.f_frsize)
}
