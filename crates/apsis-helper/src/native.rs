// SPDX-License-Identifier: GPL-3.0-only

//! The native rsync backend as the helper runs it: the backup device named in Apsis's config,
//! mounted at [`MOUNT_POINT`] for the length of one call, and the rest of what a snapshot is
//! taken with (this system's `/` UUID, distribution, `/etc/fstab`, the filters).
//!
//! List, browse and restore mount the device read-only; create and delete read-write.

use std::fs;
use std::path::{Path, PathBuf};

use apsis_core::config::Config;
use apsis_core::native::exclude::{self, HomeUser};
use apsis_core::native::{self, NativeConfig, NativeRsync, QuietRunner};
use apsis_core::settings::{self, Device};
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

/// Read-only (list, browse, restore) or read-write (create, delete).
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
/// No backup device, the device not connected, or one Apsis doesn't mount (encrypted, not a
/// Linux filesystem).
pub fn config(
    apsis: &Config,
    lsblk_json: &str,
    root_uuid: &str,
    distro: String,
    fstab: &str,
    users: &[HomeUser],
) -> Result<(NativeConfig, Device)> {
    let device = backup_device(apsis, lsblk_json)?;
    let config = NativeConfig {
        repo: PathBuf::from(MOUNT_POINT),
        device: Some(device.path()),
        device_uuid: Some(device.uuid.clone()),
        source: PathBuf::from("/"),
        sys_uuid: root_uuid.trim().to_owned(),
        sys_distro: distro,
        exclude: exclude::for_backup(&apsis.filters, fstab, users),
        dry_run: false,
    };
    Ok((config, device))
}

/// The backup device Apsis's config names, as lsblk shows it: connected, and one Apsis mounts
/// itself (unencrypted, a Linux filesystem).
///
/// # Errors
///
/// No backup device, the device not connected, encrypted or not a Linux filesystem.
pub fn backup_device(apsis: &Config, lsblk_json: &str) -> Result<Device> {
    let uuid = apsis.backup_device_uuid.clone();
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
            "the backup device ({}, {}) is encrypted or not a Linux filesystem; encrypted \
             backup disks aren't supported yet",
            device.path(),
            if device.fstype.is_empty() {
                "no filesystem"
            } else {
                &device.fstype
            }
        )));
    }
    Ok(device)
}

/// `mount -o <ro,noexec|rw>,nosuid,nodev /dev/disk/by-uuid/<uuid> <MOUNT_POINT>`. Timeshift mounts
/// by UUID too (`Device.mount`, `Device.vala:1571-1666`), with no options.
pub fn mount_argv(uuid: &str, access: Access) -> Vec<String> {
    let options = match access {
        Access::ReadOnly => "ro,nosuid,nodev,noexec",
        Access::ReadWrite => "rw,nosuid,nodev",
    };
    vec![
        "mount".to_owned(),
        "-o".to_owned(),
        options.to_owned(),
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

/// The native backend on the backup device from Apsis's config (or, before it's saved, the
/// import), mounted for `access`. The device stays mounted while the returned guard lives;
/// drop the backend first.
///
/// # Errors
///
/// See [`config`]; also a failed `lsblk`, `findmnt` or `mount`, or a `config.toml` that
/// can't be read.
pub fn open<R: Runner + Clone>(
    runner: &R,
    access: Access,
    log: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(NativeRsync<QuietRunner>, Mounted<R>)> {
    let devices = lsblk(runner)?;
    let (apsis, _) = Files::system().effective(&settings::parse_lsblk(&devices)?, &[])?;
    let root_uuid = run(runner, &[&FINDMNT_ROOT_UUID[..], &["/"]].concat())?;
    let distro = native::distro::full_name(Path::new("/"));
    // Timeshift reads a missing fstab or passwd as empty.
    let fstab = fs::read_to_string("/etc/fstab").unwrap_or_default();
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let users = exclude::home_users(&passwd, Path::new("/"));
    let (config, device) = config(&apsis, &devices, &root_uuid, distro, &fstab, &users)?;
    let mounted = mount(runner, &device, access)?;
    let backend =
        NativeRsync::new(config, QuietRunner::new(SAFE_PATH).low_priority()).with_log(log);
    Ok((backend, mounted))
}

/// Mounts `device` at [`MOUNT_POINT`] until the guard drops.
fn mount<R: Runner + Clone>(runner: &R, device: &Device, access: Access) -> Result<Mounted<R>> {
    fs::create_dir_all(MOUNT_POINT)?;
    // Timeshift unmounts whatever is at its mount point first; one left by a crashed helper
    // would be ours. Not mounted is fine.
    let _ = run(runner, &["umount", MOUNT_POINT]);
    let argv = mount_argv(&device.uuid, access);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    run(runner, &argv)?;
    Ok(Mounted {
        runner: runner.clone(),
    })
}

/// The backup device from Apsis's config, mounted read-only (and `noexec`) at
/// [`MOUNT_POINT`] while the guard lives: for browsing and restoring.
///
/// # Errors
///
/// See [`backup_device`]; also a failed `lsblk` or `mount`.
pub fn mount_backup<R: Runner + Clone>(runner: &R) -> Result<Mounted<R>> {
    let devices = lsblk(runner)?;
    let (apsis, _) = Files::system().effective(&settings::parse_lsblk(&devices)?, &[])?;
    let device = backup_device(&apsis, &devices)?;
    mount(runner, &device, Access::ReadOnly)
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

    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");
    const ROOT_UUID: &str = "22222222-2222-2222-2222-222222222222\n";
    /// `sdb1` in the lsblk fixture.
    const BACKUP_UUID: &str = "00000000-0000-0000-0000-000000000000";

    fn apsis(uuid: &str) -> Config {
        Config {
            backup_device_uuid: uuid.to_owned(),
            filters: vec!["+ /home/user1/**".to_owned(), "*.iso".to_owned()],
        }
    }

    #[test]
    fn config_follows_apsiss_config() {
        let (config, device) = config(
            &apsis(BACKUP_UUID),
            LSBLK,
            ROOT_UUID,
            "Pop 24.04 (noble)".to_owned(),
            "",
            &[],
        )
        .unwrap();
        assert_eq!(config.repo, Path::new(MOUNT_POINT));
        assert_eq!(config.source, Path::new("/"));
        assert_eq!(config.device_uuid.as_deref(), Some(BACKUP_UUID));
        assert_eq!(device.uuid, BACKUP_UUID);
        assert_eq!(config.sys_uuid, "22222222-2222-2222-2222-222222222222");
        assert!(!config.dry_run);
        assert_eq!(
            config.exclude,
            exclude::for_backup(&apsis(BACKUP_UUID).filters, "", &[])
        );
    }

    #[test]
    fn missing_encrypted_and_unset_devices_are_refused() {
        let open = |uuid: &str| config(&apsis(uuid), LSBLK, ROOT_UUID, String::new(), "", &[]);
        assert!(matches!(
            open("33333333-0000-0000-0000-000000000000"),
            Err(Error::DeviceNotFound { .. })
        ));
        assert!(matches!(open(""), Err(Error::NoSnapshotDevice)));
        // The unlocked LUKS filesystem.
        let result = open("33333333-3333-3333-3333-333333333333");
        assert!(
            matches!(result, Err(Error::Native(ref m)) if m.contains("aren't supported yet")),
            "{result:?}"
        );
    }

    #[test]
    fn mount_is_by_uuid_and_read_only_unless_writing() {
        assert_eq!(
            mount_argv("abcd", Access::ReadOnly),
            [
                "mount",
                "-o",
                "ro,nosuid,nodev,noexec",
                "/dev/disk/by-uuid/abcd",
                MOUNT_POINT
            ]
        );
        assert_eq!(mount_argv("abcd", Access::ReadWrite)[2], "rw,nosuid,nodev");
    }
}
