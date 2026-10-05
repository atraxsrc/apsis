// SPDX-License-Identifier: GPL-3.0-only

//! Why a snapshot can't be restored on this computer (PLAN 6b.7).
//!
//! Checked for the dialog, again when preparing, and again at apply. Everything here works on
//! what the helper has already read (mountinfo, lsblk, `info.json`, a folder listing), so
//! nothing is opened or run; the one exception is [`pop_upgrade_found`], an `lstat` of three
//! names, which hands [`check_pending`] what it found. The space lines (6b.4) need the dry
//! runs, so they're checked after these, in [`super::space`]. Busy isn't here.

use std::path::Path;

use serde_json::Value;

use super::esp::CheckFailure;
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
    /// `/` is on a device Restore doesn't know ([`is_restorable_root`]: anything but a plain
    /// partition or an encrypted Pop!_OS install's layout), or lsblk doesn't show its device.
    RootDevice,
    /// `/boot`, `/usr` or `/var` is a separate mount.
    SplitSystem,
    /// The snapshot was made of a system with another root UUID.
    OtherInstallation,
    Unreadable(Unreadable),
    /// The snapshot's kernel has no modules, or its boot tools are missing.
    KernelIncomplete,
    /// `/system-update` or `/etc/system-update` exists already: another update is pending
    /// and waits for a restart ([`check_arming`]).
    PendingUpdate,
    /// `/system-update` is Apsis's own link: a restore is armed and waits for the restart.
    /// The dialog's [`check`] says so, read-only, and the helper's preparation refuses with
    /// it before it touches anything, so the armed plan's files stay as they are (the
    /// armed-state gap, 2026-10-04). Another tool's link at either name is
    /// [`Refusal::PendingUpdate`] in both places. Never at apply, where the link is the
    /// restore's own.
    RestoreArmed,
    /// A Pop!_OS release upgrade is in progress or half done: one of [`POP_UPGRADE_NAMES`]
    /// exists ([`check_pending`]). Checked in the dialog, when preparing and before arming,
    /// not at apply (PLAN 6b.6, "pop-upgrade-init").
    PopUpgradePending,
    /// The snapshot's `etc/crypttab` differs from the live one ([`crypttab_differs`]): the
    /// snapshot's initrd carries the entries its system needed at boot, and the apply doesn't
    /// rebuild it (PLAN 6b.6 step 5; the rebuild is 0.5.x).
    CrypttabDiffers,
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
    /// A dry run gave no size that could be read, so the space can't be checked
    /// ([`super::space::dry_run_size`]).
    SizeUnknown,
    /// On the live system, the ESP doesn't boot the kernel `/boot` links to
    /// ([`super::esp::check_before_arming`]).
    BootFiles(CheckFailure),
}

impl Refusal {
    /// The refusal as `CheckRestore` carries it (PLAN 6b.9): one stable word per variant, the
    /// space ones with their two numbers and the boot-files one with its failure's word after
    /// a `:`. Never empty, never with whitespace.
    #[must_use]
    pub fn to_wire(&self) -> String {
        match self {
            Self::NotUefi => "not-uefi".to_owned(),
            Self::NotKernelstub => "not-kernelstub".to_owned(),
            Self::BootPartition => "boot-partition".to_owned(),
            Self::RootFilesystem { fstype } => format!("root-filesystem:{fstype}"),
            Self::RootDevice => "root-device".to_owned(),
            Self::SplitSystem => "split-system".to_owned(),
            Self::OtherInstallation => "other-installation".to_owned(),
            Self::Unreadable(why) => format!("unreadable:{}", why.word()),
            Self::KernelIncomplete => "kernel-incomplete".to_owned(),
            Self::PendingUpdate => "pending-update".to_owned(),
            Self::RestoreArmed => "restore-armed".to_owned(),
            Self::PopUpgradePending => "pop-upgrade-pending".to_owned(),
            Self::CrypttabDiffers => "crypttab-differs".to_owned(),
            Self::BackupSpace { needs, free } => format!("backup-space:{needs}:{free}"),
            Self::SystemSpace { needs, free } => format!("system-space:{needs}:{free}"),
            Self::BootSpace { needs, free } => format!("boot-space:{needs}:{free}"),
            Self::SizeUnknown => "size-unknown".to_owned(),
            Self::BootFiles(failure) => format!("boot-files:{}", failure.to_wire()),
        }
    }

    /// The refusal a helper sent; `None` for a word this version doesn't know (a newer
    /// helper's) or for none.
    #[must_use]
    pub fn from_wire(word: &str) -> Option<Self> {
        let (head, rest) = word.split_once(':').unwrap_or((word, ""));
        let space = |make: fn(u64, u64) -> Self| {
            let (needs, free) = rest.split_once(':')?;
            Some(make(needs.parse().ok()?, free.parse().ok()?))
        };
        match (head, rest) {
            ("not-uefi", "") => Some(Self::NotUefi),
            ("not-kernelstub", "") => Some(Self::NotKernelstub),
            ("boot-partition", "") => Some(Self::BootPartition),
            ("root-filesystem", fstype) if !fstype.is_empty() => Some(Self::RootFilesystem {
                fstype: fstype.to_owned(),
            }),
            ("root-device", "") => Some(Self::RootDevice),
            ("split-system", "") => Some(Self::SplitSystem),
            ("other-installation", "") => Some(Self::OtherInstallation),
            ("unreadable", why) => Unreadable::from_word(why).map(Self::Unreadable),
            ("kernel-incomplete", "") => Some(Self::KernelIncomplete),
            ("pending-update", "") => Some(Self::PendingUpdate),
            ("restore-armed", "") => Some(Self::RestoreArmed),
            ("pop-upgrade-pending", "") => Some(Self::PopUpgradePending),
            ("crypttab-differs", "") => Some(Self::CrypttabDiffers),
            ("backup-space", _) => space(|needs, free| Self::BackupSpace { needs, free }),
            ("system-space", _) => space(|needs, free| Self::SystemSpace { needs, free }),
            ("boot-space", _) => space(|needs, free| Self::BootSpace { needs, free }),
            ("size-unknown", "") => Some(Self::SizeUnknown),
            ("boot-files", failure) => CheckFailure::from_wire(failure).map(Self::BootFiles),
            _ => None,
        }
    }
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

impl Unreadable {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NoInfo => "no-info",
            Self::NotRsync => "not-rsync",
            Self::NoLocalhost => "no-localhost",
            Self::NoExcludeList => "no-exclude-list",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "no-info" => Some(Self::NoInfo),
            "not-rsync" => Some(Self::NotRsync),
            "no-localhost" => Some(Self::NoLocalhost),
            "no-exclude-list" => Some(Self::NoExcludeList),
            _ => None,
        }
    }
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
    /// The text of `/etc/crypttab`, empty if there's none.
    pub crypttab: &'a str,
    /// `/system-update` or `/etc/system-update` exists (not followed).
    pub pending_update: bool,
    /// `/system-update` is Apsis's own link (it points at the state folder): a restore is
    /// armed and waits for the restart. Never set at apply, where the link is the restore's
    /// own.
    pub restore_armed: bool,
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
    /// The text of its `etc/initramfs/post-update.d/zz-kernelstub`, if it has one. The apply's
    /// boot refresh relies on the hook's `--preserve-live-mode` (PLAN 6b.6 step 5).
    pub hook: Option<&'a str>,
    /// It has a `kernelstub` program.
    pub has_kernelstub: bool,
}

/// What the snapshot's kernelstub hook must contain for the boot refresh to keep the ESP's
/// kernel (PLAN 6b.6 step 5).
pub const HOOK_FLAG: &str = "--preserve-live-mode";

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
    if root.is_none_or(|device| !is_restorable_root(device, system.devices, system.crypttab)) {
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
    let hook_has_flag = snapshot.hook.is_some_and(|hook| hook.contains(HOOK_FLAG));
    if !(kernel_is_whole(snapshot) && hook_has_flag && snapshot.has_kernelstub) {
        return Err(Refusal::KernelIncomplete);
    }
    if system.restore_armed {
        return Err(Refusal::RestoreArmed);
    }
    if system.pending_update {
        return Err(Refusal::PendingUpdate);
    }
    Ok(())
}

/// How an encrypted root is unlocked: what the recovery note's unlock lines are made of
/// ([`super::recover::text`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unlocking<'a> {
    /// The mapping's name, as the crypttab has it (`cryptdata`). `update-initramfs` looks the
    /// root's mapping up in the crypttab by this name, so a disk opened by hand under another
    /// gets an initrd that can't unlock it.
    pub name: &'a str,
    /// The UUID of the LUKS partition the mapping is of.
    pub luks_uuid: &'a str,
}

impl System<'_> {
    /// How the root is unlocked, if it's the encrypted layout [`check`] lets through; `None`
    /// on a plain partition, and for a root [`check`] refuses.
    #[must_use]
    pub fn unlocking(&self) -> Option<Unlocking<'_>> {
        let root = self
            .devices
            .iter()
            .find(|device| !self.root_uuid.is_empty() && device.uuid == self.root_uuid)?;
        encrypted_root(root, self.devices, self.crypttab)
    }
}

/// Whether `/` may be on `root`, one of lsblk's `devices`. Two layouts are let through:
///
/// - a plain partition;
/// - what Pop!_OS's installer makes with "Encrypt drive" ([`encrypted_root`]).
///
/// The second is the layout both runs of the LUKS spike restored on (apsis-test, 2026-10-05
/// and 2026-10-06), and nothing wider: LUKS without LVM, LVM without LUKS, a volume on
/// several devices, a mapping of a whole disk or of a RAID stay refused until a machine has
/// shown them to work. The same installation only, as [`Refusal::OtherInstallation`] and
/// [`Refusal::CrypttabDiffers`] still hold; the apply's boot refresh then checks that the new
/// boot files can unlock the disk as the old ones did.
fn is_restorable_root(root: &Device, devices: &[Device], crypttab: &str) -> bool {
    root.kind == "part" || encrypted_root(root, devices, crypttab).is_some()
}

/// The encrypted layout, if `root` is on it: an LVM volume on one dm-crypt mapping of one
/// LUKS partition, which the live `crypttab` opens under that name and by that partition's
/// UUID. The name and the UUID are plain words (they end up in the recovery note's command
/// lines).
///
/// A volume group with a second physical volume isn't seen from here as long as the root's
/// own volume sits on one: lsblk lists a volume under the devices it's on.
fn encrypted_root<'a>(
    root: &Device,
    devices: &'a [Device],
    crypttab: &str,
) -> Option<Unlocking<'a>> {
    // The one device holding `device`, if it's of this kind and holds this.
    let held_by = |device: &Device, kind: &str, holds: &str| {
        let [parent] = device.parents.as_slice() else {
            return None;
        };
        devices
            .iter()
            .find(|d| d.kname == *parent)
            .filter(|d| d.kind == kind && d.fstype.eq_ignore_ascii_case(holds))
    };
    if root.kind != "lvm" {
        return None;
    }
    let mapping = held_by(root, "crypt", "LVM2_member")?;
    let partition = held_by(mapping, "part", "crypto_LUKS")?;
    let (name, luks_uuid) = (mapping.name.as_str(), partition.uuid.as_str());
    let plain = |word: &str, more: &str| {
        word.starts_with(|c: char| c.is_ascii_alphanumeric())
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || more.contains(c))
    };
    (plain(name, "_-.+") && plain(luks_uuid, "-") && crypttab_opens(crypttab, name, luks_uuid))
        .then_some(Unlocking { name, luks_uuid })
}

/// Whether `crypttab` has an entry that opens the LUKS device of `uuid` under `name`: the
/// name first, then the device as `UUID=<uuid>` or `/dev/disk/by-uuid/<uuid>`. An entry that
/// names it another way (a device path, `PARTUUID=`, `LABEL=`) isn't one: nothing here could
/// tell whether it's the same device.
fn crypttab_opens(crypttab: &str, name: &str, uuid: &str) -> bool {
    crypttab_entries(crypttab).iter().any(|entry| {
        let mut fields = entry.split(' ');
        fields.next() == Some(name)
            && fields.next().is_some_and(|source| {
                source
                    .strip_prefix("UUID=")
                    .or_else(|| source.strip_prefix("/dev/disk/by-uuid/"))
                    .is_some_and(|found| found.eq_ignore_ascii_case(uuid))
            })
    })
}

/// What's at `/system-update` or `/etc/system-update`, asked of the name itself (`lstat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateLink {
    Nothing,
    /// A link to something that's there, wherever it points: also to Apsis's own folder.
    Link,
    /// A link to nothing.
    Dangling,
    /// A file or a folder.
    Other,
}

/// The last check before arming, which makes `/system-update`: nothing may be at that name
/// or at `/etc/system-update` (systemd's generator reads both, `systemd.offline-updates(7)`).
/// Whatever is there is another update that's pending, or was left by one: it isn't Apsis's
/// to replace, and systemd would act on it at the restart.
///
/// # Errors
///
/// [`Refusal::PendingUpdate`] if either name is taken, by anything.
pub fn check_arming(
    system_update: UpdateLink,
    etc_system_update: UpdateLink,
) -> Result<(), Refusal> {
    if system_update != UpdateLink::Nothing || etc_system_update != UpdateLink::Nothing {
        return Err(Refusal::PendingUpdate);
    }
    Ok(())
}

/// The names Pop!_OS's release upgrade leaves at the root while it's in progress or half done
/// (`pop-upgrade`'s `upgrade.sh`; PLAN 6b.6): the two it makes before the offline boot, and
/// the one it touches during it. Any of them present refuses arming. fwupd's history database
/// and PackageKit's staged update are not among them, on purpose (owner, 2026-10-01).
pub const POP_UPGRADE_NAMES: [&str; 3] = [
    "/pop-upgrade",
    "/pop_preparing_release_upgrade",
    "/upgrade-attempted",
];

/// Which of [`POP_UPGRADE_NAMES`] exist under `root` (`/` on the live system), asked of each
/// name itself (`lstat`): a file, a folder, a link or a dangling link all count. The input
/// for [`check_pending`].
#[must_use]
pub fn pop_upgrade_found(root: &Path) -> Vec<&'static str> {
    POP_UPGRADE_NAMES
        .into_iter()
        .filter(|name| {
            let under_root = root.join(name.trim_start_matches('/'));
            under_root.symlink_metadata().is_ok()
        })
        .collect()
}

/// Refuses while a Pop!_OS release upgrade is in progress or half done: `found` is what
/// [`pop_upgrade_found`] saw. Pure, so the dialog, the preparation and the arming check
/// (`check_arming`'s neighbour) share it; not run at apply, where `/system-update` is
/// Apsis's own and the drop-in holds `pop-upgrade-init` off.
///
/// # Errors
///
/// [`Refusal::PopUpgradePending`] if any name was found.
pub fn check_pending(found: &[&str]) -> Result<(), Refusal> {
    if found.is_empty() {
        Ok(())
    } else {
        Err(Refusal::PopUpgradePending)
    }
}

/// Whether the snapshot's `etc/crypttab` (`None`: it has none, read as empty) sets up the
/// encrypted disks differently from the live one. "Same" means: comment lines (`#` after
/// trimming) and blank lines dropped, each remaining line's fields joined by one space,
/// compared in order. A comment edit never refuses; a changed field does.
#[must_use]
pub fn crypttab_differs(snapshot: Option<&str>, live: &str) -> bool {
    crypttab_entries(snapshot.unwrap_or_default()) != crypttab_entries(live)
}

/// The entries of a crypttab, normalised as [`crypttab_differs`] compares them.
fn crypttab_entries(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
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
    /// Pop!_OS 24.04's `etc/initramfs/post-update.d/zz-kernelstub`, in short.
    const HOOK: &str = "#!/bin/sh\nexec kernelstub --verbose --preserve-live-mode\n";

    /// Every refusal has a word on the wire and comes back the same (`CheckRestore`'s
    /// `refusal`, PLAN 6b.9); the numbers of the space ones ride along.
    #[test]
    fn every_refusal_survives_the_bus() {
        let all = [
            Refusal::NotUefi,
            Refusal::NotKernelstub,
            Refusal::BootPartition,
            Refusal::RootFilesystem {
                fstype: "btrfs".to_owned(),
            },
            Refusal::RootDevice,
            Refusal::SplitSystem,
            Refusal::OtherInstallation,
            Refusal::Unreadable(Unreadable::NoInfo),
            Refusal::Unreadable(Unreadable::NotRsync),
            Refusal::Unreadable(Unreadable::NoLocalhost),
            Refusal::Unreadable(Unreadable::NoExcludeList),
            Refusal::KernelIncomplete,
            Refusal::PendingUpdate,
            Refusal::RestoreArmed,
            Refusal::PopUpgradePending,
            Refusal::CrypttabDiffers,
            Refusal::BackupSpace {
                needs: 1_000_000,
                free: 999,
            },
            Refusal::SystemSpace { needs: 5, free: 0 },
            Refusal::BootSpace { needs: 7, free: 6 },
            Refusal::SizeUnknown,
            Refusal::BootFiles(CheckFailure::NoKernelLink),
            Refusal::BootFiles(CheckFailure::KernelDiffers),
            Refusal::BootFiles(CheckFailure::InitrdDiffers),
            Refusal::BootFiles(CheckFailure::NoModules {
                version: KERNEL.to_owned(),
            }),
            Refusal::BootFiles(CheckFailure::NoEntry),
            Refusal::BootFiles(CheckFailure::PreviousIncomplete),
        ];
        let mut words = std::collections::HashSet::new();
        for refusal in all {
            let word = refusal.to_wire();
            assert!(
                !word.is_empty() && !word.contains(char::is_whitespace),
                "{word:?}"
            );
            assert!(words.insert(word.clone()), "{word} twice");
            assert_eq!(Refusal::from_wire(&word), Some(refusal), "{word}");
        }
        assert_eq!(Refusal::NotUefi.to_wire(), "not-uefi");
        assert_eq!(Refusal::RestoreArmed.to_wire(), "restore-armed");
        assert_eq!(
            Refusal::SystemSpace { needs: 5, free: 0 }.to_wire(),
            "system-space:5:0"
        );
        assert_eq!(
            Refusal::BootFiles(CheckFailure::NoModules {
                version: KERNEL.to_owned()
            })
            .to_wire(),
            format!("boot-files:no-modules:{KERNEL}")
        );
        // A newer helper's word, or none, is nothing this version knows.
        assert_eq!(Refusal::from_wire(""), None);
        assert_eq!(Refusal::from_wire("moon-phase"), None);
        assert_eq!(Refusal::from_wire("system-space:x:0"), None);
        assert_eq!(Refusal::from_wire("boot-files:no-modules"), None);
    }

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
            parents: Vec::new(),
        }
    }

    /// A device held by `parent`, under the kernel's name `kname`.
    fn held(kname: &str, parent: &str, device: Device) -> Device {
        Device {
            kname: kname.to_owned(),
            parents: vec![parent.to_owned()],
            ..device
        }
    }

    const LUKS_UUID: &str = "22222222-2222-2222-2222-222222222222";
    /// The crypttab of an encrypted Pop!_OS install: the root's mapping, and the swap's.
    const ENCRYPTED_CRYPTTAB: &str = "\
cryptdata UUID=22222222-2222-2222-2222-222222222222 none luks
cryptswap UUID=44444444-4444-4444-4444-444444444444 /dev/urandom swap,plain,offset=1024,cipher=aes-xts-plain64,size=512
";

    /// The devices of an encrypted Pop!_OS install ("Encrypt drive"), as apsis-test's after
    /// its reinstall (2026-10-05): the root is an LVM volume on a mapping of a partition.
    fn encrypted_devices() -> Vec<Device> {
        vec![
            device("sdX", "disk", "", ""),
            held("sdX1", "sdX", device("sdX1", "part", "vfat", "AAAA-0001")),
            held(
                "sdX3",
                "sdX",
                device("sdX3", "part", "crypto_LUKS", LUKS_UUID),
            ),
            held(
                "dm-0",
                "sdX3",
                device("cryptdata", "crypt", "LVM2_member", "pv-uuid"),
            ),
            held(
                "dm-1",
                "dm-0",
                device("data-root", "lvm", "ext4", ROOT_UUID),
            ),
        ]
    }

    /// [`Case::good`] on the encrypted layout.
    fn encrypted() -> Case {
        Case {
            mountinfo: MOUNTINFO.replace("/dev/sdX3 rw", "/dev/mapper/data-root rw"),
            devices: encrypted_devices(),
            crypttab: ENCRYPTED_CRYPTTAB.to_owned(),
            ..Case::good()
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
        crypttab: String,
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
                crypttab: String::new(),
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
                crypttab: &self.crypttab,
                pending_update: false,
                restore_armed: false,
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
                hook: Some(HOOK),
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

    /// A root on a mapped device of any kind, with nothing known about what holds it.
    #[test]
    fn a_root_on_a_mapped_device_is_refused() {
        for kind in ["crypt", "lvm", "dm", "raid1", "disk", ""] {
            let mut case = Case::good();
            case.devices[2].kind = kind.to_owned();
            assert_eq!(case.check(), Err(Refusal::RootDevice), "{kind}");
        }
    }

    /// The layout the LUKS spike restored on: an LVM volume on a mapping of a LUKS partition
    /// that the live crypttab opens.
    #[test]
    fn an_encrypted_pop_os_machine_restores_its_own_snapshot() {
        assert_eq!(encrypted().check(), Ok(()));
        // The entry's other spellings of the same device, and a comment beside it.
        for crypttab in [
            format!("# the disk\ncryptdata\tUUID={LUKS_UUID}  none  luks,discard\n"),
            format!("cryptdata UUID={} none luks\n", LUKS_UUID.to_uppercase()),
            format!("cryptdata /dev/disk/by-uuid/{LUKS_UUID} none luks\n"),
        ] {
            let case = Case {
                crypttab: crypttab.clone(),
                ..encrypted()
            };
            assert_eq!(case.check(), Ok(()), "{crypttab}");
        }
    }

    /// What the recovery note's unlock lines are made of: the mapping's name and the LUKS
    /// partition's UUID, only for the layout that's let through.
    #[test]
    fn the_unlocking_of_an_encrypted_root_is_its_mapping_and_its_partition() {
        let system = |case: &Case| {
            System {
                uefi: true,
                kernelstub_config: true,
                kernelstub: true,
                mountinfo: "",
                devices: &case.devices,
                root_uuid: &case.root_uuid,
                esp_folders: &[],
                crypttab: &case.crypttab,
                pending_update: false,
                restore_armed: false,
            }
            .unlocking()
            .map(|u| (u.name.to_owned(), u.luks_uuid.to_owned()))
        };
        assert_eq!(
            system(&encrypted()),
            Some(("cryptdata".to_owned(), LUKS_UUID.to_owned()))
        );
        assert_eq!(system(&Case::good()), None, "a plain partition");
        let no_entry = Case {
            crypttab: String::new(),
            ..encrypted()
        };
        assert_eq!(system(&no_entry), None, "a root the check refuses");
        // A name or a UUID that isn't a plain word never reaches a command line.
        for (name, uuid) in [
            ("crypt data", LUKS_UUID),
            ("crypt;data", LUKS_UUID),
            ("-cryptdata", LUKS_UUID),
            ("", LUKS_UUID),
            ("cryptdata", "2222 2222"),
            ("cryptdata", "$(x)"),
        ] {
            let mut case = encrypted();
            case.devices[3].name = name.to_owned();
            case.devices[2].uuid = uuid.to_owned();
            case.crypttab = format!("{name} UUID={uuid} none luks\n");
            assert_eq!(system(&case), None, "{name:?} {uuid:?}");
            assert_eq!(case.check(), Err(Refusal::RootDevice), "{name:?} {uuid:?}");
        }
    }

    /// The same machine from lsblk's own JSON, through [`parse_lsblk`]. The file is written
    /// by hand from what the spike's log shows of apsis-test, with placeholders.
    #[test]
    fn the_encrypted_layout_is_read_from_lsblks_json() {
        let devices =
            crate::settings::parse_lsblk(include_str!("../../tests/fixtures/lsblk-encrypted.json"))
                .unwrap();
        let root = devices.iter().find(|d| d.uuid == ROOT_UUID).unwrap();
        assert_eq!(
            (root.kind.as_str(), root.parents.as_slice()),
            ("lvm", &["dm-0".to_owned()][..])
        );
        let case = Case {
            devices,
            ..encrypted()
        };
        assert_eq!(case.check(), Ok(()));
    }

    /// Every other stack under the root is refused: none has been restored on.
    #[test]
    fn a_layout_other_than_the_encrypted_installs_is_refused() {
        let refused = |what: &str, change: fn(&mut Vec<Device>)| {
            let mut case = encrypted();
            change(&mut case.devices);
            assert_eq!(case.check(), Err(Refusal::RootDevice), "{what}");
        };
        refused("LUKS without LVM", |devices| {
            devices.pop();
            devices[3].fstype = "ext4".to_owned();
            devices[3].uuid = ROOT_UUID.to_owned();
        });
        refused("LVM without LUKS", |devices| {
            devices[4].parents = vec!["sdX3".to_owned()];
            devices[2].fstype = "LVM2_member".to_owned();
            devices.remove(3);
        });
        refused("a volume on two mappings", |devices| {
            devices[4].parents.push("dm-7".to_owned());
        });
        refused("a volume lsblk gives no parent for", |devices| {
            devices[4].parents.clear();
        });
        refused("a mapping of a whole disk", |devices| {
            devices[3].parents = vec!["sdX".to_owned()];
            devices[0].fstype = "crypto_LUKS".to_owned();
            devices[0].uuid = LUKS_UUID.to_owned();
        });
        refused("a mapping of a RAID", |devices| {
            devices[2].kind = "raid1".to_owned();
        });
        refused("a mapping of a mapping", |devices| {
            devices[2].kind = "crypt".to_owned();
        });
        refused("a volume on a volume", |devices| {
            devices[3].kind = "lvm".to_owned();
        });
        refused("a mapping that isn't a physical volume", |devices| {
            devices[3].fstype = "ext4".to_owned();
        });
        refused("a partition that isn't LUKS", |devices| {
            devices[2].fstype = "ext4".to_owned();
        });
        refused("a mapping whose partition lsblk doesn't list", |devices| {
            devices.remove(2);
        });
    }

    /// The live crypttab must open the root's mapping under its name, by the UUID of the
    /// partition it's on.
    #[test]
    fn an_encrypted_root_the_crypttab_doesnt_open_is_refused() {
        for crypttab in [
            String::new(),
            "# cryptdata UUID=22222222-2222-2222-2222-222222222222 none luks\n".to_owned(),
            format!("other UUID={LUKS_UUID} none luks\n"),
            "cryptdata UUID=99999999-9999-9999-9999-999999999999 none luks\n".to_owned(),
            "cryptdata /dev/sdX3 none luks\n".to_owned(),
            "cryptdata PARTUUID=22222222-2222-2222-2222-222222222222 none luks\n".to_owned(),
            "cryptdata\n".to_owned(),
            // The swap's entry alone.
            ENCRYPTED_CRYPTTAB.lines().nth(1).unwrap().to_owned(),
        ] {
            let case = Case {
                crypttab: crypttab.clone(),
                ..encrypted()
            };
            assert_eq!(case.check(), Err(Refusal::RootDevice), "{crypttab:?}");
        }
        // A LUKS partition lsblk gives no UUID for can't be matched with an entry.
        let mut case = encrypted();
        case.devices[2].uuid.clear();
        case.crypttab = "cryptdata UUID= none luks\n".to_owned();
        assert_eq!(case.check(), Err(Refusal::RootDevice));
        // A plain partition needs nothing of the crypttab.
        assert!(Case::good().crypttab.is_empty());
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

    /// The hook flag rule (PLAN 6b.6 step 5): the snapshot's
    /// `etc/initramfs/post-update.d/zz-kernelstub` must be there and contain
    /// `--preserve-live-mode`, or the boot refresh of the apply would rewrite the ESP for the
    /// snapshot's kernel. `update-initramfs` isn't needed: the apply doesn't run it.
    #[test]
    fn a_snapshot_without_its_boot_tools_is_refused() {
        let no_hook = Case {
            snapshot: |s| s.hook = None,
            ..Case::good()
        };
        assert_eq!(no_hook.check(), Err(Refusal::KernelIncomplete));
        let hook_without_flag = Case {
            snapshot: |s| s.hook = Some("#!/bin/sh\nexec kernelstub --verbose\n"),
            ..Case::good()
        };
        assert_eq!(hook_without_flag.check(), Err(Refusal::KernelIncomplete));
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

    /// Apsis's own link is told apart from another tool's update: a restore that's armed
    /// is refused as such, also when `/etc/system-update` is taken as well. It keeps the
    /// pending update's place in the order.
    #[test]
    fn an_armed_restore_is_refused_as_such() {
        let armed = Case {
            system: |s| {
                s.pending_update = true;
                s.restore_armed = true;
            },
            ..Case::good()
        };
        assert_eq!(armed.check(), Err(Refusal::RestoreArmed));
        let unreadable = Case {
            snapshot: |s| s.has_localhost = false,
            ..armed
        };
        assert_eq!(
            unreadable.check(),
            Err(Refusal::Unreadable(Unreadable::NoLocalhost))
        );
    }

    /// Arming makes `/system-update`. Anything already at that name, or at
    /// `/etc/system-update`, which systemd's generator reads too, is another update that's
    /// pending, whatever it is and wherever it points: arming is refused.
    #[test]
    fn arming_is_refused_while_another_update_is_pending() {
        use UpdateLink::{Dangling, Link, Nothing, Other};
        assert_eq!(check_arming(Nothing, Nothing), Ok(()));
        for found in [Link, Dangling, Other] {
            assert_eq!(
                check_arming(found, Nothing),
                Err(Refusal::PendingUpdate),
                "{found:?}"
            );
            assert_eq!(
                check_arming(Nothing, found),
                Err(Refusal::PendingUpdate),
                "{found:?} in /etc"
            );
        }
    }

    /// A folder for the live root, with the given names made as files, folders or links.
    fn root_with(label: &str, names: &[(&str, &str)]) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("apsis-pending-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for (name, kind) in names {
            let path = root.join(name.trim_start_matches('/'));
            match *kind {
                "file" => std::fs::write(&path, b"").unwrap(),
                "dir" => std::fs::create_dir_all(&path).unwrap(),
                "dangling" => std::os::unix::fs::symlink("/nowhere/at/all", &path).unwrap(),
                other => panic!("{other}"),
            }
        }
        root
    }

    /// Each of the three Pop names refuses on its own, as a file, a folder or a dangling
    /// link (`lstat`); fwupd's database and PackageKit's staged update don't.
    #[test]
    fn a_pop_upgrade_in_progress_or_half_done_is_refused() {
        let clean = root_with("clean", &[]);
        assert_eq!(pop_upgrade_found(&clean), Vec::<&str>::new());
        assert_eq!(check_pending(&pop_upgrade_found(&clean)), Ok(()));
        for (i, name) in POP_UPGRADE_NAMES.iter().enumerate() {
            for kind in ["file", "dir", "dangling"] {
                let root = root_with(&format!("{i}-{kind}"), &[(name, kind)]);
                let found = pop_upgrade_found(&root);
                assert_eq!(found, [*name], "{name} as a {kind}");
                assert_eq!(check_pending(&found), Err(Refusal::PopUpgradePending));
                std::fs::remove_dir_all(&root).unwrap();
            }
        }
        let not_refused = root_with(
            "others",
            &[
                ("/var/lib/fwupd", "dir"),
                ("/var/lib/fwupd/pending.db", "file"),
                ("/var/lib/PackageKit", "dir"),
                ("/var/lib/PackageKit/prepared-update", "file"),
                ("/system-update", "dangling"),
            ],
        );
        assert_eq!(pop_upgrade_found(&not_refused), Vec::<&str>::new());
        assert_eq!(check_pending(&pop_upgrade_found(&not_refused)), Ok(()));
        // All three at once: still one refusal, naming all three.
        let all = root_with(
            "all",
            &[
                ("/pop-upgrade", "file"),
                ("/pop_preparing_release_upgrade", "file"),
                ("/upgrade-attempted", "file"),
            ],
        );
        assert_eq!(pop_upgrade_found(&all), POP_UPGRADE_NAMES);
        assert_eq!(
            check_pending(&["/upgrade-attempted"]),
            Err(Refusal::PopUpgradePending)
        );
        for root in [clean, not_refused, all] {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// apsis-test's live crypttab: cryptswap with a random key.
    const CRYPTTAB: &str = "cryptswap UUID=00000000-0000-0000-0000-00000000c0de /dev/urandom \
                            swap,plain,offset=1024,cipher=aes-xts-plain64,size=512
";

    #[test]
    fn a_crypttab_with_the_same_entries_is_the_same_whatever_its_comments() {
        assert!(!crypttab_differs(Some(CRYPTTAB), CRYPTTAB), "byte-equal");
        let commented = format!(
            "# <target name> <source device> <key file> <options>

{CRYPTTAB}
# the end
"
        );
        assert!(
            !crypttab_differs(Some(&commented), CRYPTTAB),
            "comments and blank lines"
        );
        let spaced = CRYPTTAB.replace(' ', "\t  ");
        assert!(
            !crypttab_differs(Some(&spaced), CRYPTTAB),
            "whitespace between fields"
        );
        let unterminated = CRYPTTAB.trim_end();
        assert!(!crypttab_differs(Some(unterminated), CRYPTTAB));
        // Only comments: as good as none.
        assert!(!crypttab_differs(
            Some(
                "# nothing
"
            ),
            ""
        ));
        assert!(!crypttab_differs(None, ""), "no file against an empty one");
        assert!(!crypttab_differs(
            None,
            "
# header
"
        ));
    }

    #[test]
    fn a_crypttab_whose_fields_changed_differs() {
        let other_key = CRYPTTAB.replace("/dev/urandom", "/etc/keys/swap.key");
        assert!(crypttab_differs(Some(&other_key), CRYPTTAB), "a field");
        let other_options = CRYPTTAB.replace("size=512", "size=256");
        assert!(crypttab_differs(Some(&other_options), CRYPTTAB));
        let one_more = format!(
            "{CRYPTTAB}cryptroot UUID=11111111-1111-1111-1111-111111111111 none luks,discard
"
        );
        assert!(crypttab_differs(Some(&one_more), CRYPTTAB), "an entry more");
        assert!(
            crypttab_differs(Some(CRYPTTAB), &one_more),
            "an entry fewer"
        );
        assert!(
            crypttab_differs(None, CRYPTTAB),
            "no file against a live entry"
        );
        assert!(
            crypttab_differs(Some(CRYPTTAB), ""),
            "an entry against no live one"
        );
        // A comment that was an entry, or the other way round, is a change.
        let commented_out = format!("#{CRYPTTAB}");
        assert!(crypttab_differs(Some(&commented_out), CRYPTTAB));
        // The order of entries counts.
        let two = format!(
            "{CRYPTTAB}cryptroot UUID=11111111-1111-1111-1111-111111111111 none luks
"
        );
        let swapped = format!(
            "cryptroot UUID=11111111-1111-1111-1111-111111111111 none luks
{CRYPTTAB}"
        );
        assert!(crypttab_differs(Some(&two), &swapped));
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
