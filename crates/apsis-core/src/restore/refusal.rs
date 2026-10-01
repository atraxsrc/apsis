// SPDX-License-Identifier: GPL-3.0-only

//! Why a snapshot can't be restored on this computer (PLAN 6b.7).
//!
//! Checked for the dialog, again when preparing, and again at apply. Everything here works on
//! what the helper has already read (mountinfo, lsblk, `info.json`, a folder listing), so
//! nothing is opened or run. The space lines (6b.4) need the dry runs, so they're checked after
//! these, in [`super::space`]. Busy isn't here.

use std::path::Path;

use serde_json::Value;

use super::filter::is_kernel_version;
use crate::native::Info;
use crate::settings::Device;
use crate::usage::fstype_at;

/// The first check that failed. The dialog's two lines for each are in PLAN 6b.8's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// There's no `/sys/firmware/efi`.
    NotUefi,
    /// No `/etc/kernelstub/configuration` or no `kernelstub` (GRUB, other distros).
    NotKernelstub,
    /// The ESP isn't mounted at `/boot/efi` as vfat, or has no `EFI/Pop_OS-<root-uuid>/`.
    BootPartition,
    /// `/` isn't ext4.
    RootFilesystem {
        fstype: String,
    },
    /// `/` isn't on a plain partition (dm-crypt, LVM), or lsblk doesn't show its device.
    RootDevice,
    /// `/boot`, `/usr` or `/var` is a separate mount.
    SplitSystem,
    /// The snapshot was made of a system with another root UUID.
    OtherInstallation,
    Unreadable(Unreadable),
    /// The snapshot's kernel has no modules, or its boot tools are missing.
    KernelIncomplete,
    /// `/system-update` exists already: a system update waits for a restart.
    PendingUpdate,
    /// The backup disk is short for the safety snapshot ([`super::space`]). Both in bytes.
    BackupSpace {
        needs: u64,
        free: u64,
    },
    /// `/`, or a separate `/home` being restored, is short for the restore. Both in bytes.
    SystemSpace {
        needs: u64,
        free: u64,
    },
    /// The ESP is short for the boot refresh and a put-back ([`super::esp`]). Both in bytes.
    BootSpace {
        needs: u64,
        free: u64,
    },
}

/// Why a snapshot can't be read as something to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// `info.json` is missing or isn't one Timeshift would read.
    NoInfo,
    /// A btrfs snapshot.
    NotRsync,
    NoLocalhost,
    NoExcludeList,
}

/// The running system.
#[derive(Debug, Clone, Copy)]
pub struct System<'a> {
    /// `/sys/firmware/efi` exists.
    pub uefi: bool,
    /// `/etc/kernelstub/configuration` exists.
    pub kernelstub_config: bool,
    /// The `kernelstub` program is there.
    pub kernelstub: bool,
    /// The text of `/proc/self/mountinfo`.
    pub mountinfo: &'a str,
    /// lsblk's devices ([`crate::settings::parse_lsblk`]).
    pub devices: &'a [Device],
    /// The UUID of the filesystem mounted at `/` (`findmnt`).
    pub root_uuid: &'a str,
    /// The names in `/boot/efi/EFI`.
    pub esp_folders: &'a [String],
    /// `/system-update` exists (not followed).
    pub pending_update: bool,
}

/// The snapshot, as found in its folder on the backup disk.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot<'a> {
    /// Its `info.json`; `None` if it's missing or [`Info::parse`] gave nothing.
    pub info: Option<&'a Info>,
    /// `localhost/` is a folder.
    pub has_localhost: bool,
    /// `exclude.list` is a file.
    pub has_exclude_list: bool,
    /// The text of its `etc/kernelstub/configuration`, if it has one.
    pub kernelstub_config: Option<&'a str>,
    /// Where its `boot/vmlinuz` link points (`vmlinuz-<version>`), if it's a link.
    pub vmlinuz: Option<&'a str>,
    /// The names in its `boot`.
    pub boot_files: &'a [String],
    /// The names in its `usr/lib/modules`.
    pub modules: &'a [String],
    pub has_update_initramfs: bool,
    pub has_kernelstub: bool,
}

/// Runs every check, in the order of PLAN 6b.7's table (the snapshot must be readable before
/// its UUID is).
///
/// # Errors
///
/// The first [`Refusal`].
pub fn check(system: &System<'_>, snapshot: &Snapshot<'_>) -> Result<(), Refusal> {
    let mounted = |point: &str| fstype_at(system.mountinfo, Path::new(point));
    if !system.uefi {
        return Err(Refusal::NotUefi);
    }
    if !(system.kernelstub_config && system.kernelstub) {
        return Err(Refusal::NotKernelstub);
    }
    // kernelstub's folder on the ESP, named after the root it boots.
    let esp_folder = format!("Pop_OS-{}", system.root_uuid);
    if mounted("/boot/efi") != Some("vfat") || !system.esp_folders.contains(&esp_folder) {
        return Err(Refusal::BootPartition);
    }
    match mounted("/") {
        Some("ext4") => {}
        other => {
            return Err(Refusal::RootFilesystem {
                fstype: other.unwrap_or("unknown").to_owned(),
            });
        }
    }
    // A device without a filesystem has an empty UUID: an empty root UUID names none.
    let root = system
        .devices
        .iter()
        .find(|device| !system.root_uuid.is_empty() && device.uuid == system.root_uuid);
    if root.is_none_or(|device| device.kind != "part") {
        return Err(Refusal::RootDevice);
    }
    if ["/boot", "/usr", "/var"]
        .iter()
        .any(|point| mounted(point).is_some())
    {
        return Err(Refusal::SplitSystem);
    }
    let Some(info) = snapshot.info else {
        return Err(Refusal::Unreadable(Unreadable::NoInfo));
    };
    if info.kind != "rsync" {
        return Err(Refusal::Unreadable(Unreadable::NotRsync));
    }
    if !snapshot.has_localhost {
        return Err(Refusal::Unreadable(Unreadable::NoLocalhost));
    }
    if !snapshot.has_exclude_list {
        return Err(Refusal::Unreadable(Unreadable::NoExcludeList));
    }
    if info.sys_uuid != system.root_uuid {
        return Err(Refusal::OtherInstallation);
    }
    if let Some(config) = snapshot.kernelstub_config {
        kernelstub_root(config, system.root_uuid)?;
    }
    if !(kernel_is_whole(snapshot) && snapshot.has_update_initramfs && snapshot.has_kernelstub) {
        return Err(Refusal::KernelIncomplete);
    }
    if system.pending_update {
        return Err(Refusal::PendingUpdate);
    }
    Ok(())
}

/// Checks the `root=` kernel options of a snapshot's kernelstub configuration: its `default`
/// and `user` sections each have `kernel_options`. kernelstub adds `root=UUID=` itself from
/// the mounted root, so most have none. One that's there must be `root=UUID=<root_uuid>`:
/// kernelstub would write any other (another UUID, `PARTUUID=`, `LABEL=`, a device) into the
/// boot entry.
///
/// A configuration that isn't a JSON object is one kernelstub can't read either: the boot
/// refresh would fail after the copy.
fn kernelstub_root(config: &str, root_uuid: &str) -> Result<(), Refusal> {
    let Ok(Value::Object(sections)) = serde_json::from_str::<Value>(config) else {
        return Err(Refusal::KernelIncomplete);
    };
    let is_this_root = |root: &str| {
        root.strip_prefix("UUID=")
            .is_some_and(|uuid| uuid.eq_ignore_ascii_case(root_uuid))
    };
    for section in ["default", "user"] {
        // A list of options, or (older configurations) all of them in one string.
        let options: Vec<&str> = match sections.get(section).and_then(|s| s.get("kernel_options")) {
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
            Some(Value::String(text)) => text.split_whitespace().collect(),
            _ => Vec::new(),
        };
        let mut roots = options.iter().filter_map(|o| o.strip_prefix("root="));
        if roots.any(|root| !is_this_root(root)) {
            return Err(Refusal::OtherInstallation);
        }
    }
    Ok(())
}

/// Whether the kernel the snapshot's `boot/vmlinuz` points to is in the snapshot: the file
/// itself in `boot`, and its folder in `usr/lib/modules`.
fn kernel_is_whole(snapshot: &Snapshot<'_>) -> bool {
    let Some(file) = snapshot
        .vmlinuz
        .and_then(|target| target.rsplit('/').next())
    else {
        return false;
    };
    file.strip_prefix("vmlinuz-").is_some_and(|version| {
        is_kernel_version(version)
            && snapshot.boot_files.iter().any(|name| name == file)
            && snapshot.modules.iter().any(|name| name == version)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_UUID: &str = "11111111-1111-1111-1111-111111111111";
    const KERNEL: &str = "6.9.3-76060903-generic";

    /// A Pop!_OS machine as apsis-test. Devices are placeholders.
    const MOUNTINFO: &str = "\
24 1 259:3 / / rw,relatime shared:1 - ext4 /dev/sdX3 rw
25 24 0:5 / /dev rw,nosuid shared:2 - devtmpfs udev rw
28 24 0:24 / /run rw shared:5 - tmpfs tmpfs rw
30 24 259:1 / /boot/efi rw,relatime shared:6 - vfat /dev/sdX1 rw
31 24 259:2 / /recovery rw shared:7 - vfat /dev/sdX2 rw
33 28 8:17 / /run/apsis/backup ro,nosuid,nodev,noexec - ext4 /dev/sdZ1 ro
";

    fn device(name: &str, kind: &str, fstype: &str, uuid: &str) -> Device {
        Device {
            name: name.to_owned(),
            kname: name.to_owned(),
            kind: kind.to_owned(),
            fstype: fstype.to_owned(),
            uuid: uuid.to_owned(),
            label: String::new(),
            size: 0,
            parent_uuid: String::new(),
        }
    }

    /// A kernelstub configuration as Pop!_OS writes it, with these kernel options for the user.
    fn kernelstub(user_options: &str) -> String {
        format!(
            r#"{{
  "default": {{"kernel_options": ["quiet", "splash"], "esp_path": "/boot/efi", "config_rev": 3}},
  "user": {{"kernel_options": {user_options}, "esp_path": "/boot/efi", "setup_loader": true, "config_rev": 3}}
}}"#
        )
    }

    fn devices() -> Vec<Device> {
        vec![
            device("sdX", "disk", "", ""),
            device("sdX1", "part", "vfat", "AAAA-0001"),
            device("sdX3", "part", "ext4", ROOT_UUID),
        ]
    }

    fn info(uuid: &str, kind: &str) -> Info {
        Info {
            created: 1_790_000_000,
            sys_uuid: uuid.to_owned(),
            sys_distro: "Pop 24.04 (noble)".to_owned(),
            app_version: "apsis 0.4.1".to_owned(),
            file_count: 1234,
            tags: vec!["ondemand".to_owned()],
            comments: String::new(),
            live: false,
            kind: kind.to_owned(),
            rsync_flags: None,
        }
    }

    /// Everything a check reads, owned, so a test changes one thing.
    struct Case {
        mountinfo: String,
        devices: Vec<Device>,
        root_uuid: String,
        esp_folders: Vec<String>,
        info: Option<Info>,
        kernelstub_config: Option<String>,
        boot_files: Vec<String>,
        modules: Vec<String>,
        system: fn(&mut System<'_>),
        snapshot: fn(&mut Snapshot<'_>),
    }

    impl Case {
        fn good() -> Self {
            Self {
                mountinfo: MOUNTINFO.to_owned(),
                devices: devices(),
                root_uuid: ROOT_UUID.to_owned(),
                esp_folders: vec!["BOOT".to_owned(), format!("Pop_OS-{ROOT_UUID}")],
                info: Some(info(ROOT_UUID, "rsync")),
                kernelstub_config: Some(kernelstub(r#"["quiet", "splash"]"#)),
                boot_files: vec![
                    "efi".to_owned(),
                    "vmlinuz".to_owned(),
                    format!("vmlinuz-{KERNEL}"),
                    format!("initrd.img-{KERNEL}"),
                ],
                modules: vec!["6.8.0-old".to_owned(), KERNEL.to_owned()],
                system: |_| {},
                snapshot: |_| {},
            }
        }

        fn with_mount(mut self, line: &str) -> Self {
            self.mountinfo.push_str(line);
            self.mountinfo.push('\n');
            self
        }

        fn check(&self) -> Result<(), Refusal> {
            let mut system = System {
                uefi: true,
                kernelstub_config: true,
                kernelstub: true,
                mountinfo: &self.mountinfo,
                devices: &self.devices,
                root_uuid: &self.root_uuid,
                esp_folders: &self.esp_folders,
                pending_update: false,
            };
            (self.system)(&mut system);
            let mut snapshot = Snapshot {
                info: self.info.as_ref(),
                has_localhost: true,
                has_exclude_list: true,
                kernelstub_config: self.kernelstub_config.as_deref(),
                vmlinuz: Some("vmlinuz-6.9.3-76060903-generic"),
                boot_files: &self.boot_files,
                modules: &self.modules,
                has_update_initramfs: true,
                has_kernelstub: true,
            };
            (self.snapshot)(&mut snapshot);
            check(&system, &snapshot)
        }
    }

    #[test]
    fn a_plain_pop_os_machine_restores_its_own_snapshot() {
        assert_eq!(Case::good().check(), Ok(()));
    }

    #[test]
    fn a_bios_machine_is_refused() {
        let case = Case {
            system: |s| s.uefi = false,
            ..Case::good()
        };
        assert_eq!(case.check(), Err(Refusal::NotUefi));
    }

    #[test]
    fn a_machine_without_kernelstub_or_its_configuration_is_refused() {
        let no_config = Case {
            system: |s| s.kernelstub_config = false,
            ..Case::good()
        };
        assert_eq!(no_config.check(), Err(Refusal::NotKernelstub));
        let no_program = Case {
            system: |s| s.kernelstub = false,
            ..Case::good()
        };
        assert_eq!(no_program.check(), Err(Refusal::NotKernelstub));
    }

    #[test]
    fn an_esp_that_isnt_vfat_at_boot_efi_is_refused() {
        let elsewhere = Case {
            mountinfo: MOUNTINFO.replace(" /boot/efi ", " /efi "),
            ..Case::good()
        };
        assert_eq!(elsewhere.check(), Err(Refusal::BootPartition));
        let ext4 = Case {
            mountinfo: MOUNTINFO.replace("- vfat /dev/sdX1", "- ext4 /dev/sdX1"),
            ..Case::good()
        };
        assert_eq!(ext4.check(), Err(Refusal::BootPartition));
    }

    #[test]
    fn an_esp_without_this_roots_pop_os_folder_is_refused() {
        let case = Case {
            esp_folders: vec![
                "BOOT".to_owned(),
                "Pop_OS-99999999-9999-9999-9999-999999999999".to_owned(),
            ],
            ..Case::good()
        };
        assert_eq!(case.check(), Err(Refusal::BootPartition));
    }

    #[test]
    fn a_root_that_isnt_ext4_is_refused_with_its_type() {
        let case = Case {
            mountinfo: MOUNTINFO.replace("- ext4 /dev/sdX3", "- btrfs /dev/sdX3"),
            ..Case::good()
        };
        assert_eq!(
            case.check(),
            Err(Refusal::RootFilesystem {
                fstype: "btrfs".to_owned()
            })
        );
    }

    /// A later mount at `/` covers an earlier one: the last line is the root that's seen.
    #[test]
    fn the_root_filesystem_is_the_last_one_mounted_at_the_root() {
        let covered = Case {
            mountinfo: format!("1 0 0:1 / / rw - rootfs rootfs rw\n{MOUNTINFO}"),
            ..Case::good()
        };
        assert_eq!(covered.check(), Ok(()));
        let covering = Case::good().with_mount("99 24 0:40 / / rw - overlay overlay rw");
        assert_eq!(
            covering.check(),
            Err(Refusal::RootFilesystem {
                fstype: "overlay".to_owned()
            })
        );
    }

    #[test]
    fn a_root_on_a_mapped_device_is_refused() {
        for kind in ["crypt", "lvm", "raid1", "disk"] {
            let mut case = Case::good();
            case.devices[2].kind = kind.to_owned();
            assert_eq!(case.check(), Err(Refusal::RootDevice), "{kind}");
        }
    }

    #[test]
    fn a_root_lsblk_doesnt_show_is_refused() {
        let mut case = Case::good();
        case.devices.pop();
        assert_eq!(case.check(), Err(Refusal::RootDevice));
    }

    /// With no root UUID nothing can be compared: a device without a filesystem has an empty
    /// UUID too, and so can a broken `info.json`.
    #[test]
    fn an_empty_root_uuid_matches_nothing() {
        let mut case = Case {
            root_uuid: String::new(),
            esp_folders: vec!["Pop_OS-".to_owned()],
            info: Some(info("", "rsync")),
            ..Case::good()
        };
        case.devices.push(device("sdQ1", "part", "", ""));
        assert_eq!(case.check(), Err(Refusal::RootDevice));
    }

    #[test]
    fn a_system_split_over_partitions_is_refused() {
        for point in ["/boot", "/usr", "/var"] {
            let case =
                Case::good().with_mount(&format!("40 24 8:5 / {point} rw - ext4 /dev/sdX5 rw"));
            assert_eq!(case.check(), Err(Refusal::SplitSystem), "{point}");
        }
    }

    /// Only those three: a separate `/home` or a data disk is fine (PLAN 6b.4), and so is a
    /// mount below one of them.
    #[test]
    fn other_separate_mounts_arent_a_split_system() {
        for point in ["/home", "/data", "/var/lib/docker", "/usr/local"] {
            let case =
                Case::good().with_mount(&format!("40 24 8:5 / {point} rw - ext4 /dev/sdX5 rw"));
            assert_eq!(case.check(), Ok(()), "{point}");
        }
    }

    #[test]
    fn a_snapshot_that_cant_be_read_is_refused_with_the_reason() {
        let no_info = Case {
            info: None,
            ..Case::good()
        };
        assert_eq!(
            no_info.check(),
            Err(Refusal::Unreadable(Unreadable::NoInfo))
        );
        let btrfs = Case {
            info: Some(info(ROOT_UUID, "btrfs")),
            ..Case::good()
        };
        assert_eq!(
            btrfs.check(),
            Err(Refusal::Unreadable(Unreadable::NotRsync))
        );
        let no_localhost = Case {
            snapshot: |s| s.has_localhost = false,
            ..Case::good()
        };
        assert_eq!(
            no_localhost.check(),
            Err(Refusal::Unreadable(Unreadable::NoLocalhost))
        );
        let no_list = Case {
            snapshot: |s| s.has_exclude_list = false,
            ..Case::good()
        };
        assert_eq!(
            no_list.check(),
            Err(Refusal::Unreadable(Unreadable::NoExcludeList))
        );
    }

    #[test]
    fn a_snapshot_of_another_root_is_refused() {
        let case = Case {
            info: Some(info("99999999-9999-9999-9999-999999999999", "rsync")),
            ..Case::good()
        };
        assert_eq!(case.check(), Err(Refusal::OtherInstallation));
    }

    #[test]
    fn a_snapshot_whose_kernelstub_names_another_root_is_refused() {
        for root in [
            "root=UUID=99999999-9999-9999-9999-999999999999",
            "root=PARTUUID=11111111-1111-1111-1111-111111111111",
            "root=LABEL=pop",
            "root=/dev/sdX3",
            "root=",
        ] {
            let case = Case {
                kernelstub_config: Some(kernelstub(&format!(r#"["quiet", "{root}"]"#))),
                ..Case::good()
            };
            assert_eq!(case.check(), Err(Refusal::OtherInstallation), "{root}");
        }
    }

    /// Both sections are read, and one option for another root is enough.
    #[test]
    fn another_root_in_the_default_section_or_beside_this_one_is_refused() {
        let default = Case {
            kernelstub_config: Some(
                r#"{"default": {"kernel_options": ["root=LABEL=pop"]}, "user": {"kernel_options": ["quiet"]}}"#
                    .to_owned(),
            ),
            ..Case::good()
        };
        assert_eq!(default.check(), Err(Refusal::OtherInstallation));
        let beside = Case {
            kernelstub_config: Some(kernelstub(
                r#"["root=UUID=11111111-1111-1111-1111-111111111111", "root=/dev/sdX3"]"#,
            )),
            ..Case::good()
        };
        assert_eq!(beside.check(), Err(Refusal::OtherInstallation));
    }

    /// Older configurations hold the options as one string.
    #[test]
    fn kernel_options_in_one_string_are_read_too() {
        let other = Case {
            kernelstub_config: Some(kernelstub(r#""quiet root=LABEL=pop splash""#)),
            ..Case::good()
        };
        assert_eq!(other.check(), Err(Refusal::OtherInstallation));
        let same = Case {
            kernelstub_config: Some(kernelstub(
                r#""quiet root=UUID=11111111-1111-1111-1111-111111111111""#,
            )),
            ..Case::good()
        };
        assert_eq!(same.check(), Ok(()));
    }

    #[test]
    fn a_kernelstub_configuration_naming_this_root_or_none_passes() {
        let same = Case {
            kernelstub_config: Some(kernelstub(
                r#"["root=UUID=11111111-1111-1111-1111-111111111111", "quiet"]"#,
            )),
            ..Case::good()
        };
        assert_eq!(same.check(), Ok(()));
        let no_options = Case {
            kernelstub_config: Some(r#"{"user": {"esp_path": "/boot/efi"}}"#.to_owned()),
            ..Case::good()
        };
        assert_eq!(no_options.check(), Ok(()));
        let missing = Case {
            kernelstub_config: None,
            ..Case::good()
        };
        assert_eq!(missing.check(), Ok(()));
    }

    /// Only `kernel_options` is read: the same text anywhere else names no root.
    #[test]
    fn a_root_outside_the_kernel_options_is_not_an_option() {
        let case = Case {
            kernelstub_config: Some(
                r#"{"user": {"kernel_options": ["quiet", "nroot=UUID=99999999-9999-9999-9999-999999999999"], "note": "root=UUID=99999999-9999-9999-9999-999999999999"}, "other": {"kernel_options": ["root=UUID=99999999-9999-9999-9999-999999999999"]}}"#
                    .to_owned(),
            ),
            ..Case::good()
        };
        assert_eq!(case.check(), Ok(()));
    }

    /// kernelstub can't read it either, so the boot refresh after the copy would fail.
    #[test]
    fn a_kernelstub_configuration_that_isnt_json_is_refused() {
        for text in ["", "root=UUID=11111111-1111-1111-1111-111111111111", "[]"] {
            let case = Case {
                kernelstub_config: Some(text.to_owned()),
                ..Case::good()
            };
            assert_eq!(case.check(), Err(Refusal::KernelIncomplete), "{text:?}");
        }
    }

    #[test]
    fn a_root_that_isnt_in_the_mount_table_is_refused_as_not_ext4() {
        let case = Case {
            mountinfo: MOUNTINFO.replace("24 1 259:3 / / rw", "24 1 259:3 / /sysroot rw"),
            ..Case::good()
        };
        assert_eq!(
            case.check(),
            Err(Refusal::RootFilesystem {
                fstype: "unknown".to_owned()
            })
        );
    }

    #[test]
    fn a_snapshot_without_the_kernel_file_its_link_names_is_refused() {
        let mut case = Case::good();
        case.boot_files.retain(|name| !name.starts_with("vmlinuz-"));
        case.boot_files.push("vmlinuz-6.8.0-old".to_owned());
        assert_eq!(case.check(), Err(Refusal::KernelIncomplete));
    }

    #[test]
    fn a_snapshot_kernel_without_its_modules_is_refused() {
        let case = Case {
            modules: vec!["6.8.0-old".to_owned()],
            ..Case::good()
        };
        assert_eq!(case.check(), Err(Refusal::KernelIncomplete));
    }

    #[test]
    fn a_snapshot_without_a_usable_vmlinuz_link_is_refused() {
        let no_link = Case {
            snapshot: |s| s.vmlinuz = None,
            ..Case::good()
        };
        assert_eq!(no_link.check(), Err(Refusal::KernelIncomplete));
        // A link that names no version, even if a modules folder is called the same.
        let odd = Case {
            modules: vec![String::new(), "..".to_owned(), "old".to_owned()],
            snapshot: |s| s.vmlinuz = Some("vmlinuz-"),
            ..Case::good()
        };
        assert_eq!(odd.check(), Err(Refusal::KernelIncomplete));
        let other_file = Case {
            modules: vec!["old".to_owned()],
            snapshot: |s| s.vmlinuz = Some("vmlinuz.old"),
            ..Case::good()
        };
        assert_eq!(other_file.check(), Err(Refusal::KernelIncomplete));
    }

    /// Pop's link is relative (`vmlinuz-<version>`); `boot/vmlinuz-<version>` and an absolute
    /// one name the same kernel.
    #[test]
    fn the_kernel_version_is_the_links_file_name() {
        let case = Case {
            snapshot: |s| s.vmlinuz = Some("/boot/vmlinuz-6.9.3-76060903-generic"),
            ..Case::good()
        };
        assert_eq!(case.check(), Ok(()));
    }

    #[test]
    fn a_snapshot_without_its_boot_tools_is_refused() {
        let no_initramfs = Case {
            snapshot: |s| s.has_update_initramfs = false,
            ..Case::good()
        };
        assert_eq!(no_initramfs.check(), Err(Refusal::KernelIncomplete));
        let no_kernelstub = Case {
            snapshot: |s| s.has_kernelstub = false,
            ..Case::good()
        };
        assert_eq!(no_kernelstub.check(), Err(Refusal::KernelIncomplete));
    }

    #[test]
    fn a_pending_system_update_is_refused() {
        let case = Case {
            system: |s| s.pending_update = true,
            ..Case::good()
        };
        assert_eq!(case.check(), Err(Refusal::PendingUpdate));
    }

    /// One reason is shown: what's wrong with the computer comes before what's wrong with
    /// the snapshot, and a pending update last.
    #[test]
    fn the_first_failed_check_in_the_plans_order_is_the_reason() {
        let case = Case {
            info: Some(info("99999999-9999-9999-9999-999999999999", "rsync")),
            system: |s| {
                s.uefi = false;
                s.pending_update = true;
            },
            ..Case::good()
        };
        assert_eq!(case.check(), Err(Refusal::NotUefi));
        let snapshot_first = Case {
            info: Some(info("99999999-9999-9999-9999-999999999999", "rsync")),
            system: |s| s.pending_update = true,
            ..Case::good()
        };
        assert_eq!(snapshot_first.check(), Err(Refusal::OtherInstallation));
    }
}
