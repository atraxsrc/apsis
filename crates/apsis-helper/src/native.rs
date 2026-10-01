// SPDX-License-Identifier: GPL-3.0-only

//! The native rsync backend as the helper runs it: the backup device named in Apsis's config,
//! mounted at [`MOUNT_POINT`], and the rest of what a snapshot is taken with (this system's
//! `/` UUID, distribution, `/etc/fstab`, the filters).
//!
//! Lists share one read-only mount ([`SharedMount`]: the first reader mounts, the last one
//! unmounts); a create, delete or delete-many mounts read-write for itself, once the readers
//! are gone (`state.rs` sees to the order).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use apsis_core::config::Config;
use apsis_core::native::exclude::{self, HomeUser};
use apsis_core::native::{self, NativeConfig, NativeRsync, QuietRunner};
use apsis_core::settings::{self, Device};
use apsis_core::{Error, Result, Runner};

use crate::runner::SAFE_PATH;
use crate::settings::{Files, lsblk, system};

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

/// Read-only (list) or read-write (create, delete).
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
        exclude: exclude::for_backup(
            &apsis.filters,
            apsis.include_root,
            apsis.include_home,
            fstab,
            users,
        ),
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
            // A disk pulled out mid-job: detach it lazily, so the next plug-in mounts cleanly.
            eprintln!("apsis-helper: couldn't unmount {MOUNT_POINT}: {error}; unmounting lazily");
            if let Err(error) = run(&self.runner, &["umount", "--lazy", MOUNT_POINT]) {
                eprintln!("apsis-helper: couldn't unmount {MOUNT_POINT} lazily: {error}");
            }
        }
    }
}

/// The read-only mount the readers share. The first reader mounts the device, the others
/// find it mounted, the last one out unmounts; a reader is refused if the device named in
/// the config isn't the one mounted (the config changed under them, which only a hand edit
/// does: `WriteConfig` is a write and waits for the readers).
pub struct SharedMount<R: Runner> {
    held: Mutex<Option<Held<R>>>,
}

struct Held<R: Runner> {
    uuid: String,
    readers: usize,
    /// Held for its drop: the unmount, when the last reader leaves.
    _mounted: Mounted<R>,
}

impl<R: Runner + Clone> Default for SharedMount<R> {
    fn default() -> Self {
        Self {
            held: Mutex::new(None),
        }
    }
}

impl<R: Runner + Clone> SharedMount<R> {
    fn lock(&self) -> MutexGuard<'_, Option<Held<R>>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Readers holding the mount.
    #[cfg(test)]
    pub fn readers(&self) -> usize {
        self.lock().as_ref().map_or(0, |h| h.readers)
    }

    /// Mounts `device` read-only, or joins the readers that have it mounted, until the guard
    /// drops.
    ///
    /// # Errors
    ///
    /// `mount` failed, or another device is mounted for the readers in.
    pub fn acquire(self: &Arc<Self>, runner: &R, device: &Device) -> Result<Shared<R>> {
        let mut held = self.lock();
        match held.as_mut() {
            Some(held) if held.uuid == device.uuid => held.readers += 1,
            Some(_) => {
                return Err(Error::Native(
                    "the backup device changed while it was being read; try again".to_owned(),
                ));
            }
            None => {
                let mounted = mount(runner, device, Access::ReadOnly)?;
                *held = Some(Held {
                    uuid: device.uuid.clone(),
                    readers: 1,
                    _mounted: mounted,
                });
            }
        }
        Ok(Shared(Arc::clone(self)))
    }
}

/// A reader's share of the mount (see [`SharedMount::acquire`]).
pub struct Shared<R: Runner + Clone>(Arc<SharedMount<R>>);

impl<R: Runner + Clone> Drop for Shared<R> {
    fn drop(&mut self) {
        let mut held = self.0.lock();
        let last = match held.as_mut() {
            Some(h) => {
                h.readers = h.readers.saturating_sub(1);
                h.readers == 0
            }
            None => false,
        };
        if last {
            // Drops the `Mounted`: the unmount.
            *held = None;
        }
    }
}

/// What the native backend needs from this system and Apsis's config (or, before it's saved,
/// the import).
fn prepare<R: Runner>(runner: &R) -> Result<(NativeConfig, Device)> {
    let devices = lsblk(runner)?;
    let (apsis, _) = Files::system().effective(&settings::parse_lsblk(&devices)?, &system()?)?;
    let root_uuid = run(runner, &[&FINDMNT_ROOT_UUID[..], &["/"]].concat())?;
    let distro = native::distro::full_name(Path::new("/"));
    // Timeshift reads a missing fstab or passwd as empty.
    let fstab = fs::read_to_string("/etc/fstab").unwrap_or_default();
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let users = exclude::home_users(&passwd, Path::new("/"));
    config(&apsis, &devices, &root_uuid, distro, &fstab, &users)
}

fn backend(
    config: NativeConfig,
    log: impl Fn(&str) + Send + Sync + 'static,
) -> NativeRsync<QuietRunner> {
    NativeRsync::new(config, QuietRunner::new(SAFE_PATH).low_priority()).with_log(log)
}

/// The native backend on the backup device, mounted read-write for this one write. The device
/// stays mounted while the returned guard lives; drop the backend first.
///
/// # Errors
///
/// See [`config`]; also a failed `lsblk`, `findmnt` or `mount`, or a `config.toml` that
/// can't be read.
pub fn open<R: Runner + Clone>(
    runner: &R,
    log: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(NativeRsync<QuietRunner>, Mounted<R>)> {
    let (config, device) = prepare(runner)?;
    let mounted = mount(runner, &device, Access::ReadWrite)?;
    Ok((backend(config, log), mounted))
}

/// The native backend on the backup device, on the read-only mount the readers share (see
/// [`SharedMount`]). The share lasts while the returned guard lives.
///
/// # Errors
///
/// As [`open`].
pub fn open_shared<R: Runner + Clone>(
    runner: &R,
    shared: &Arc<SharedMount<R>>,
    log: impl Fn(&str) + Send + Sync + 'static,
) -> Result<(NativeRsync<QuietRunner>, Shared<R>)> {
    let (config, device) = prepare(runner)?;
    let share = shared.acquire(runner, &device)?;
    Ok((backend(config, log), share))
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
            include_root: true,
            include_home: false,
            filters: vec!["+ /home/user1/**".to_owned(), "- *.iso".to_owned()],
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
            exclude::for_backup(&apsis(BACKUP_UUID).filters, true, false, "", &[])
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

    /// Records every argv; `mount` and `umount` succeed, nothing else is asked of it.
    #[derive(Clone, Default)]
    struct FakeRunner(Arc<Mutex<Vec<String>>>);

    impl FakeRunner {
        fn lines(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, argv: &[std::ffi::OsString]) -> std::io::Result<apsis_core::RunOutput> {
            let line = argv
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            self.0.lock().unwrap().push(line);
            Ok(apsis_core::RunOutput {
                success: true,
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn device(uuid: &str) -> Device {
        settings::parse_lsblk(LSBLK)
            .unwrap()
            .into_iter()
            .find(|d| d.uuid == uuid)
            .unwrap()
    }

    #[test]
    fn five_overlapping_readers_mount_once_and_the_writer_mounts_after_the_unmount() {
        let runner = FakeRunner::default();
        let shared = Arc::new(SharedMount::<FakeRunner>::default());
        let device = device(BACKUP_UUID);
        let mut readers: Vec<_> = (0..5)
            .map(|_| shared.acquire(&runner, &device).unwrap())
            .collect();
        assert_eq!(shared.readers(), 5);
        let ro = format!(
            "mount -o ro,nosuid,nodev,noexec /dev/disk/by-uuid/{BACKUP_UUID} {MOUNT_POINT}"
        );
        let umount = format!("umount {MOUNT_POINT}");
        // The first reader's: a precautionary umount, then the one read-only mount.
        assert_eq!(runner.lines(), [umount.clone(), ro.clone()]);
        // Four leave: still mounted.
        readers.truncate(1);
        assert_eq!(shared.readers(), 1);
        assert_eq!(runner.lines().len(), 2, "no umount while a reader is in");
        // The last one out unmounts.
        readers.clear();
        assert_eq!(shared.readers(), 0);
        assert_eq!(runner.lines(), [umount.clone(), ro.clone(), umount.clone()]);
        // The writer's read-write mount comes only after that umount.
        let _writer = mount(&runner, &device, Access::ReadWrite).unwrap();
        let rw = format!("mount -o rw,nosuid,nodev /dev/disk/by-uuid/{BACKUP_UUID} {MOUNT_POINT}");
        assert_eq!(runner.lines()[3..], [umount.clone(), rw]);
        // After the writer, a reader mounts again.
        drop(_writer);
        let _again = shared.acquire(&runner, &device).unwrap();
        assert_eq!(runner.lines().iter().filter(|l| **l == ro).count(), 2);
    }

    #[test]
    fn a_reader_for_another_device_is_refused_while_one_is_mounted() {
        let runner = FakeRunner::default();
        let shared = Arc::new(SharedMount::<FakeRunner>::default());
        let first = shared.acquire(&runner, &device(BACKUP_UUID)).unwrap();
        let other = device("11111111-1111-1111-1111-111111111111");
        assert!(matches!(
            shared.acquire(&runner, &other),
            Err(Error::Native(ref m)) if m.contains("changed")
        ));
        drop(first);
        assert!(
            shared.acquire(&runner, &other).is_ok(),
            "free once they left"
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
