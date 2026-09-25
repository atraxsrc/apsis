// SPDX-License-Identifier: GPL-3.0-only

//! The native rsync backend as the helper runs it: the backup device named in Timeshift's
//! settings, mounted at [`MOUNT_POINT`] for the length of one call, and the rest of what a
//! snapshot is taken with (this system's `/` UUID, distribution, `/etc/fstab`, the filters).
//!
//! Timeshift's settings are only read here. List and dry run mount the device read-only.

use std::fs;
use std::path::{Path, PathBuf};

use apsis_core::native::exclude::{self, HomeUser};
use apsis_core::native::{self, NativeConfig, NativeRsync, QuietRunner};
use apsis_core::settings::{self, Config, Device};
use apsis_core::{Error, Result, Runner};

use crate::runner::SAFE_PATH;
use crate::settings::{Files, lsblk};

/// Where the helper mounts the backup device. Under `/run`, which Timeshift's own filters
/// exclude (`/run/*`), like its `/run/timeshift/<pid>/backup`.
pub const MOUNT_POINT: &str = "/run/apsis/backup";

/// `findmnt` for the UUID of the filesystem mounted at `/`: Timeshift's `sys_root` is the
/// device lsblk shows mounted at `/` (`Main.vala:3523-3545`), and `sys-uuid` its UUID.
pub const FINDMNT_ROOT_UUID: [&str; 5] = [
    "findmnt",
    "--noheadings",
    "--output",
    "UUID",
    "--mountpoint",
];

/// Read-only (list, dry run) or read-write (create).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

/// Everything the native backend needs, from the texts the helper reads. Kept apart from the
/// reading so it can be tested.
///
/// # Errors
///
/// btrfs mode, no backup device, the device not connected, or one Apsis doesn't mount
/// (encrypted, not a Linux filesystem).
pub fn config(
    timeshift_json: &str,
    lsblk_json: &str,
    root_uuid: &str,
    distro: String,
    fstab: &str,
    users: &[HomeUser],
    dry_run: bool,
) -> Result<(NativeConfig, Device)> {
    let settings = Config::parse(timeshift_json)?.settings();
    if settings.btrfs_mode {
        return Err(Error::Native(
            "Timeshift is in btrfs mode; the native backend is rsync only for now".to_owned(),
        ));
    }
    let uuid = settings.backup_device_uuid;
    if uuid.is_empty() {
        return Err(Error::NoSnapshotDevice);
    }
    let devices = settings::parse_lsblk(lsblk_json)?;
    let device = devices
        .into_iter()
        .find(|d| d.uuid == uuid)
        .ok_or_else(|| Error::DeviceNotFound {
            device: uuid.clone(),
        })?;
    if !device.selectable() {
        return Err(Error::Native(format!(
            "the backup device ({}, {}) is encrypted or not a Linux filesystem; the native \
             backend doesn't unlock or mount those, Timeshift does",
            device.path(),
            if device.fstype.is_empty() {
                "no filesystem"
            } else {
                &device.fstype
            }
        )));
    }
    let config = NativeConfig {
        repo: PathBuf::from(MOUNT_POINT),
        device: Some(device.path()),
        device_uuid: Some(uuid),
        source: PathBuf::from("/"),
        sys_uuid: root_uuid.trim().to_owned(),
        sys_distro: distro,
        exclude: exclude::for_backup(&settings.exclude, fstab, users),
        dry_run,
    };
    Ok((config, device))
}

/// `mount -o <ro|rw>,nosuid,nodev /dev/disk/by-uuid/<uuid> <MOUNT_POINT>`. Timeshift mounts
/// by UUID too (`Device.mount`, `Device.vala:1571-1666`), with no options.
pub fn mount_argv(uuid: &str, access: Access) -> Vec<String> {
    let mode = match access {
        Access::ReadOnly => "ro",
        Access::ReadWrite => "rw",
    };
    vec![
        "mount".to_owned(),
        "-o".to_owned(),
        format!("{mode},nosuid,nodev"),
        format!("/dev/disk/by-uuid/{uuid}"),
        MOUNT_POINT.to_owned(),
    ]
}

/// The backup device mounted at [`MOUNT_POINT`] until this drops.
pub struct Mounted<R: Runner> {
    runner: R,
}

impl<R: Runner> Drop for Mounted<R> {
    fn drop(&mut self) {
        if let Err(error) = run(&self.runner, &["umount", MOUNT_POINT]) {
            eprintln!("apsis-helper: couldn't unmount {MOUNT_POINT}: {error}");
        }
    }
}

/// The native backend on the backup device from Timeshift's settings, mounted for `access`.
/// The device stays mounted while the returned guard lives; drop the backend first.
///
/// # Errors
///
/// See [`config`]; also a failed `lsblk`, `findmnt` or `mount`.
pub fn open<R: Runner + Clone>(
    runner: &R,
    access: Access,
    dry_run: bool,
    log: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(NativeRsync<QuietRunner>, Mounted<R>)> {
    let text = Files::system().read()?;
    let devices = lsblk(runner)?;
    let root_uuid = run(runner, &[&FINDMNT_ROOT_UUID[..], &["/"]].concat())?;
    let distro = native::distro::full_name(Path::new("/"));
    // Timeshift reads a missing fstab or passwd as empty.
    let fstab = fs::read_to_string("/etc/fstab").unwrap_or_default();
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let users = exclude::home_users(&passwd, Path::new("/"));
    let (config, device) = config(&text, &devices, &root_uuid, distro, &fstab, &users, dry_run)?;

    fs::create_dir_all(MOUNT_POINT)?;
    // Timeshift unmounts whatever is at its mount point first; one left by a crashed helper
    // would be ours. Not mounted is fine.
    let _ = run(runner, &["umount", MOUNT_POINT]);
    let argv = mount_argv(&device.uuid, access);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    run(runner, &argv)?;
    let mounted = Mounted {
        runner: runner.clone(),
    };
    let backend = NativeRsync::new(config, QuietRunner::new(SAFE_PATH)).with_log(log);
    Ok((backend, mounted))
}

/// Timeshift's lock file (`AppLock.create("timeshift", ...)`, `AppLock.vala:33-37`,
/// `Main.vala:263`): `<pid>;<mode>`, held while any Timeshift (command line or window) runs.
pub const TIMESHIFT_LOCK: &str = "/var/run/lock/timeshift/lock";

/// The PID in Timeshift's lock file, if that process is a Timeshift: a `/proc/<pid>/exe` whose
/// name contains `timeshift`, 26.09.0's test. 24.01.1 (`AppLock.vala:39-50`,
/// `TeeJee.Process.vala:294-313`) counts any running process with that PID; this is stricter
/// about what counts, so a stale lock whose PID was reused doesn't block a native create. A
/// native create doesn't start while a Timeshift runs.
pub fn timeshift_running() -> Option<u32> {
    let text = fs::read_to_string(TIMESHIFT_LOCK).ok()?;
    let pid: u32 = text.split(';').next()?.trim().parse().ok()?;
    let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    exe.file_name()?
        .to_string_lossy()
        .contains("timeshift")
        .then_some(pid)
}

/// Runs a fixed argv; its stdout, or an error with its stderr.
fn run(runner: &impl Runner, argv: &[&str]) -> Result<String> {
    let argv: Vec<_> = argv.iter().map(Into::into).collect();
    let output = runner.run(&argv)?;
    if !output.success {
        return Err(Error::Native(format!(
            "{} failed: {}",
            argv[0].to_string_lossy(),
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");
    const ROOT_UUID: &str = "22222222-2222-2222-2222-222222222222\n";

    fn configured() -> Result<(NativeConfig, Device)> {
        config(
            CONFIG,
            LSBLK,
            ROOT_UUID,
            "Pop 24.04 (noble)".to_owned(),
            "",
            &[],
            true,
        )
    }

    #[test]
    fn config_follows_timeshifts_settings() {
        let settings = Config::parse(CONFIG).unwrap().settings();
        let (config, device) = configured().unwrap();
        assert_eq!(config.repo, Path::new(MOUNT_POINT));
        assert_eq!(config.source, Path::new("/"));
        assert_eq!(
            config.device_uuid.as_deref(),
            Some(settings.backup_device_uuid.as_str())
        );
        assert_eq!(device.uuid, settings.backup_device_uuid);
        assert_eq!(config.sys_uuid, "22222222-2222-2222-2222-222222222222");
        assert!(config.dry_run);
        assert_eq!(
            config.exclude,
            exclude::for_backup(&settings.exclude, "", &[])
        );
    }

    #[test]
    fn btrfs_mode_and_missing_devices_are_refused() {
        let btrfs = CONFIG.replace("\"btrfs_mode\" : \"false\"", "\"btrfs_mode\" : \"true\"");
        assert_ne!(btrfs, CONFIG);
        let result = config(&btrfs, LSBLK, ROOT_UUID, String::new(), "", &[], true);
        assert!(
            matches!(result, Err(Error::Native(ref m)) if m.contains("btrfs")),
            "{result:?}"
        );

        let settings = Config::parse(CONFIG).unwrap().settings();
        let unplugged = LSBLK.replace(
            &settings.backup_device_uuid,
            "33333333-0000-0000-0000-000000000000",
        );
        let result = config(CONFIG, &unplugged, ROOT_UUID, String::new(), "", &[], true);
        assert!(
            matches!(result, Err(Error::DeviceNotFound { .. })),
            "{result:?}"
        );

        let unset = CONFIG.replace(&settings.backup_device_uuid, "");
        let result = config(&unset, LSBLK, ROOT_UUID, String::new(), "", &[], true);
        assert!(matches!(result, Err(Error::NoSnapshotDevice)), "{result:?}");
    }

    #[test]
    fn mount_is_by_uuid_and_read_only_unless_creating() {
        assert_eq!(
            mount_argv("abcd", Access::ReadOnly),
            [
                "mount",
                "-o",
                "ro,nosuid,nodev",
                "/dev/disk/by-uuid/abcd",
                MOUNT_POINT
            ]
        );
        assert_eq!(mount_argv("abcd", Access::ReadWrite)[2], "rw,nosuid,nodev");
    }
}
