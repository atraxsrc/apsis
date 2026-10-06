// SPDX-License-Identifier: GPL-3.0-only

//! `CheckRestore`'s reads: what the live system and the snapshot look like,
//! read into text and names for core's pure checks, and the dialog built from them.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use apsis_core::native::{INFO_FILE, Info};
use apsis_core::restore::dialog::{self, Dialog, Inputs};
use apsis_core::restore::esp;
use apsis_core::restore::refusal::{self, Refusal, pop_upgrade_found};
use apsis_core::settings::{Device, parse_lsblk};
use apsis_core::{Result, Runner};
use rustix::fs::{Mode, OFlags};

use crate::arm;
use crate::native::FINDMNT_ROOT_UUID;
use crate::settings::lsblk;

/// The running system, read for the checks. `root` is `/` for real; the tests pass a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub uefi: bool,
    pub kernelstub_config: bool,
    pub kernelstub: bool,
    pub mountinfo: String,
    pub devices: Vec<Device>,
    pub root_uuid: String,
    /// The names in `boot/efi/EFI`.
    pub esp_folders: Vec<String>,
    /// `system-update` or `etc/system-update` exists (not followed).
    pub pending_update: bool,
    /// `etc/system-update` alone exists (not followed): what counts at apply, where
    /// `system-update` is Apsis's own link.
    pub etc_system_update: bool,
    /// `system-update` is Apsis's own link ([`arm::is_armed`]): a restore is armed.
    pub restore_armed: bool,
    pub pop_upgrade_found: Vec<&'static str>,
    /// `etc/crypttab`, empty if there's none.
    pub crypttab: String,
    /// [`esp::check_before_arming`] on the live system.
    pub boot_files: Result<(), Refusal>,
}

impl Live {
    /// Reads `root` and runs `lsblk` and `findmnt`; `mountinfo` is `/proc/self/mountinfo`'s
    /// text (the caller reads it, so a tree can stand in for `/`).
    ///
    /// # Errors
    ///
    /// `lsblk` or `findmnt` failed, or lsblk's JSON can't be read.
    pub fn read(root: &Path, runner: &impl Runner, mountinfo: &str) -> Result<Self> {
        let devices = parse_lsblk(&lsblk(runner)?)?;
        let argv: Vec<_> = [&FINDMNT_ROOT_UUID[..], &["/"]]
            .concat()
            .iter()
            .map(Into::into)
            .collect();
        let output = runner.run(&argv)?;
        if !output.success {
            return Err(apsis_core::Error::Helper(format!(
                "findmnt failed: {}",
                output.stderr.trim()
            )));
        }
        let root_uuid = output.stdout.trim().to_owned();
        let exists = |name: &str| root.join(name).symlink_metadata().is_ok();
        let boot_files =
            esp::check_before_arming(&root.join("boot/efi"), root, &root_uuid).map(|_| ());
        Ok(Self {
            uefi: exists("sys/firmware/efi"),
            kernelstub_config: exists("etc/kernelstub/configuration"),
            kernelstub: exists("usr/bin/kernelstub"),
            mountinfo: mountinfo.to_owned(),
            devices,
            root_uuid,
            esp_folders: names_in(&root.join("boot/efi/EFI")),
            pending_update: exists("system-update") || exists("etc/system-update"),
            etc_system_update: exists("etc/system-update"),
            restore_armed: arm::is_armed(&arm::Paths::under(root)),
            pop_upgrade_found: pop_upgrade_found(root),
            crypttab: read_nofollow(&root.join("etc/crypttab")).unwrap_or_default(),
            boot_files,
        })
    }

    /// The same view in the offline boot: `/system-update` is Apsis's own link there (the
    /// apply's step 1 checked it), so only `/etc/system-update` is another update's
    /// (`refusal::check_pending`'s note), and the restore that runs
    /// isn't refused as armed.
    pub(crate) fn as_system_at_apply(&self) -> refusal::System<'_> {
        refusal::System {
            pending_update: self.etc_system_update,
            restore_armed: false,
            ..self.as_system()
        }
    }

    pub(crate) fn as_system(&self) -> refusal::System<'_> {
        refusal::System {
            uefi: self.uefi,
            kernelstub_config: self.kernelstub_config,
            kernelstub: self.kernelstub,
            mountinfo: &self.mountinfo,
            devices: &self.devices,
            root_uuid: &self.root_uuid,
            esp_folders: &self.esp_folders,
            crypttab: &self.crypttab,
            pending_update: self.pending_update,
            restore_armed: self.restore_armed,
        }
    }
}

/// A snapshot folder (`<repo>/timeshift/snapshots/<name>/`), read for the checks. Every
/// file is opened with `O_NOFOLLOW` and every folder asked of its own name: a link where a
/// file or folder should be reads as nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotFiles {
    pub info: Option<Info>,
    pub has_localhost: bool,
    pub has_exclude_list: bool,
    pub excludes: Option<String>,
    /// `localhost/etc/kernelstub/configuration`.
    pub kernelstub_config: Option<String>,
    /// Where `localhost/boot/vmlinuz` points.
    pub vmlinuz: Option<String>,
    /// The names in `localhost/boot`.
    pub boot_files: Vec<String>,
    /// The names in `localhost/usr/lib/modules`.
    pub modules: Vec<String>,
    /// `localhost/etc/initramfs/post-update.d/zz-kernelstub`.
    pub hook: Option<String>,
    /// `localhost/usr/bin/kernelstub` is there.
    pub has_kernelstub: bool,
    /// `localhost/etc/crypttab`.
    pub crypttab: Option<String>,
    /// `localhost/var/lib/dpkg/status`.
    pub dpkg_status: Option<String>,
    /// `localhost/root` is a folder with at least one entry.
    pub root_has_content: bool,
}

impl SnapshotFiles {
    #[must_use]
    pub fn read(dir: &Path) -> Self {
        let root = dir.join("localhost");
        let excludes = read_nofollow(&dir.join("exclude.list"));
        let top = Self {
            info: read_nofollow(&dir.join(INFO_FILE)).and_then(|text| Info::parse(&text)),
            has_localhost: is_dir(&root),
            has_exclude_list: excludes.is_some(),
            excludes,
            ..Self::default()
        };
        // `O_NOFOLLOW` guards the last name only: nothing is read through a `localhost` that
        // is a link. (Links deeper down are the snapshot's own content.)
        if !top.has_localhost {
            return top;
        }
        Self {
            kernelstub_config: read_nofollow(&root.join("etc/kernelstub/configuration")),
            vmlinuz: fs::read_link(root.join("boot/vmlinuz"))
                .ok()
                .map(|target| target.to_string_lossy().into_owned()),
            boot_files: names_in(&root.join("boot")),
            modules: names_in(&root.join("usr/lib/modules")),
            hook: read_nofollow(&root.join("etc/initramfs/post-update.d/zz-kernelstub")),
            has_kernelstub: root.join("usr/bin/kernelstub").symlink_metadata().is_ok(),
            crypttab: read_nofollow(&root.join("etc/crypttab")),
            dpkg_status: read_nofollow(&root.join("var/lib/dpkg/status")),
            root_has_content: !names_in(&root.join("root")).is_empty(),
            ..top
        }
    }

    pub(crate) fn as_snapshot(&self) -> refusal::Snapshot<'_> {
        refusal::Snapshot {
            info: self.info.as_ref(),
            has_localhost: self.has_localhost,
            has_exclude_list: self.has_exclude_list,
            kernelstub_config: self.kernelstub_config.as_deref(),
            vmlinuz: self.vmlinuz.as_deref(),
            boot_files: &self.boot_files,
            modules: &self.modules,
            hook: self.hook.as_deref(),
            has_kernelstub: self.has_kernelstub,
        }
    }
}

/// The dialog for `snapshot` on `live` (the refusals in order, then the lines that aren't
/// refusals).
#[must_use]
pub fn dialog(live: &Live, snapshot: &SnapshotFiles) -> Dialog {
    let rsync_flags = snapshot
        .info
        .as_ref()
        .and_then(|info| info.rsync_flags.as_deref())
        .unwrap_or_default();
    dialog::build(&Inputs {
        system: refusal::check(&live.as_system(), &snapshot.as_snapshot()),
        pop_upgrade_found: &live.pop_upgrade_found,
        snapshot_crypttab: snapshot.crypttab.as_deref(),
        live_crypttab: &live.crypttab,
        boot_files: live.boot_files.clone(),
        snapshot_excludes: snapshot.excludes.as_deref(),
        root_has_content: snapshot.root_has_content,
        rsync_flags,
        dpkg_status: snapshot.dpkg_status.as_deref(),
    })
}

/// The snapshot `name`'s folder under the backup device's `repo`.
#[must_use]
pub fn snapshot_dir(repo: &Path, name: &str) -> PathBuf {
    repo.join(apsis_core::native::TIMESHIFT_DIR)
        .join(apsis_core::native::SNAPSHOTS_DIR)
        .join(name)
}

/// `path` is a folder by its own name (a link isn't).
pub(crate) fn is_dir(path: &Path) -> bool {
    path.symlink_metadata().is_ok_and(|meta| meta.is_dir())
}

/// The names in the folder `path`, sorted; none if it isn't a folder by its own name.
fn names_in(path: &Path) -> Vec<String> {
    if !is_dir(path) {
        return Vec::new();
    }
    let mut names: Vec<String> = fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// The text of the regular file at `path`, opened with `O_NOFOLLOW`; `None` if it's a link,
/// missing, not a file or can't be read.
pub(crate) fn read_nofollow(path: &Path) -> Option<String> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut file = File::from(rustix::fs::open(path, flags, Mode::empty()).ok()?);
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use apsis_core::native::Info;
    use apsis_core::restore::apsis::InSnapshot;
    use apsis_core::restore::esp::CheckFailure;
    use apsis_core::restore::refusal::{Refusal, Unreadable};
    use apsis_core::settings::LSBLK_ARGS;
    use apsis_core::{RunOutput, Runner};

    use super::*;
    use crate::prepare::tests::{lab, tree};

    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");
    /// `nvme0n1p2` in the lsblk fixture: a plain ext4 partition.
    const ROOT_UUID: &str = "11111111-1111-1111-1111-111111111111";
    const KERNEL: &str = "6.9.3-76060903-generic";
    const HOOK: &str = "#!/bin/sh\nexec kernelstub --verbose --preserve-live-mode\n";
    const MOUNTINFO: &str = "\
24 1 259:3 / / rw,relatime shared:1 - ext4 /dev/sdX3 rw
25 24 0:5 / /dev rw,nosuid shared:2 - devtmpfs udev rw
28 24 0:24 / /run rw shared:5 - tmpfs tmpfs rw
30 24 259:1 / /boot/efi rw,relatime shared:6 - vfat /dev/sdX1 rw
";
    const EXCLUDES: &str = "/dev/*\n+ /home/**\n/root/**\n/home/*/**\n";
    const DPKG: &str = "Package: apsis\nStatus: install ok installed\nVersion: 0.4.2\n\n";

    /// Answers lsblk with the fixture and findmnt with the root UUID.
    struct FakeSystem;

    impl Runner for FakeSystem {
        fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
            let stdout = if argv == LSBLK_ARGS.map(OsString::from) {
                LSBLK.to_owned()
            } else if argv.first().is_some_and(|a| a == "findmnt") {
                format!("{ROOT_UUID}\n")
            } else {
                panic!("unexpected {argv:?}")
            };
            Ok(RunOutput {
                success: true,
                code: Some(0),
                stdout,
                stderr: String::new(),
            })
        }
    }

    fn temp(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apsis-check-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A kernel's files in `root`'s `boot` and `usr/lib/modules`, with the links.
    fn kernel(root: &Path) {
        write(root.join(format!("boot/vmlinuz-{KERNEL}")), "kernel bytes");
        write(
            root.join(format!("boot/initrd.img-{KERNEL}")),
            "initrd bytes",
        );
        symlink(format!("vmlinuz-{KERNEL}"), root.join("boot/vmlinuz")).unwrap();
        symlink(format!("initrd.img-{KERNEL}"), root.join("boot/initrd.img")).unwrap();
        fs::create_dir_all(root.join("usr/lib/modules").join(KERNEL)).unwrap();
    }

    /// A restorable snapshot folder, as Apsis 0.4.2 made it of a Pop!_OS machine.
    fn snapshot(dir: &Path) {
        let info = Info {
            created: 1_790_000_000,
            sys_uuid: ROOT_UUID.to_owned(),
            sys_distro: "Pop 24.04 (noble)".to_owned(),
            app_version: "0.4.2".to_owned(),
            file_count: 1,
            tags: vec!["ondemand".to_owned()],
            comments: String::new(),
            live: false,
            kind: "rsync".to_owned(),
            rsync_flags: Some("-aAX --numeric-ids".to_owned()),
        };
        write(dir.join("info.json"), &info.to_text());
        write(dir.join("exclude.list"), EXCLUDES);
        let root = dir.join("localhost");
        kernel(&root);
        write(
            root.join("etc/kernelstub/configuration"),
            "{\"default\": {}, \"user\": {}}",
        );
        write(root.join("etc/initramfs/post-update.d/zz-kernelstub"), HOOK);
        write(root.join("usr/bin/kernelstub"), "#!/usr/bin/python3\n");
        write(root.join("etc/crypttab"), "# none\n");
        write(root.join("var/lib/dpkg/status"), DPKG);
        write(root.join("root/.bashrc"), "# root's\n");
    }

    /// A Pop!_OS root with its ESP booting the kernel `boot` links to.
    fn live(root: &Path) {
        fs::create_dir_all(root.join("sys/firmware/efi")).unwrap();
        write(root.join("etc/kernelstub/configuration"), "{}");
        write(root.join("usr/bin/kernelstub"), "#!/usr/bin/python3\n");
        write(root.join("etc/crypttab"), "# none\n");
        kernel(root);
        let esp = root.join("boot/efi");
        let kernels = esp.join(format!("EFI/Pop_OS-{ROOT_UUID}"));
        write(kernels.join("vmlinuz.efi"), "kernel bytes");
        write(kernels.join("initrd.img"), "initrd bytes");
        write(kernels.join("cmdline"), "quiet\n");
        write(
            esp.join("loader/entries/Pop_OS-current.conf"),
            "title Pop\n",
        );
    }

    #[test]
    fn a_snapshot_folder_is_read() {
        let dir = temp("snapshot");
        snapshot(&dir);
        let files = SnapshotFiles::read(&dir);
        assert_eq!(files.info.as_ref().unwrap().sys_uuid, ROOT_UUID);
        assert!(files.has_localhost && files.has_exclude_list);
        assert_eq!(files.excludes.as_deref(), Some(EXCLUDES));
        assert!(files.kernelstub_config.is_some());
        assert_eq!(
            files.vmlinuz.as_deref(),
            Some(format!("vmlinuz-{KERNEL}").as_str())
        );
        assert!(files.boot_files.contains(&format!("vmlinuz-{KERNEL}")));
        assert_eq!(files.modules, [KERNEL.to_owned()]);
        assert_eq!(files.hook.as_deref(), Some(HOOK));
        assert!(files.has_kernelstub);
        assert_eq!(files.crypttab.as_deref(), Some("# none\n"));
        assert_eq!(files.dpkg_status.as_deref(), Some(DPKG));
        assert!(files.root_has_content);
        // An empty /root, or none: no content.
        fs::remove_file(dir.join("localhost/root/.bashrc")).unwrap();
        assert!(!SnapshotFiles::read(&dir).root_has_content);
        fs::remove_dir(dir.join("localhost/root")).unwrap();
        assert!(!SnapshotFiles::read(&dir).root_has_content);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Nothing there reads as nothing, and a link where a file is expected isn't followed.
    #[test]
    fn an_empty_or_linked_snapshot_folder_reads_as_nothing() {
        let dir = temp("empty");
        let files = SnapshotFiles::read(&dir);
        assert!(files.info.is_none() && !files.has_localhost && !files.has_exclude_list);
        assert!(files.excludes.is_none() && files.hook.is_none() && files.vmlinuz.is_none());
        assert!(files.boot_files.is_empty() && files.modules.is_empty());
        assert!(!files.has_kernelstub && files.crypttab.is_none() && files.dpkg_status.is_none());
        // `localhost` a link, `exclude.list` a link: neither counts.
        snapshot(&dir);
        let elsewhere = temp("elsewhere");
        fs::rename(dir.join("localhost"), elsewhere.join("localhost")).unwrap();
        symlink(elsewhere.join("localhost"), dir.join("localhost")).unwrap();
        fs::remove_file(dir.join("exclude.list")).unwrap();
        symlink(
            elsewhere.join("localhost/etc/crypttab"),
            dir.join("exclude.list"),
        )
        .unwrap();
        let files = SnapshotFiles::read(&dir);
        assert!(!files.has_localhost && !files.has_exclude_list && files.excludes.is_none());
        assert!(files.hook.is_none() && !files.has_kernelstub);
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&elsewhere).unwrap();
    }

    #[test]
    fn the_live_system_is_read() {
        let root = temp("live");
        live(&root);
        let system = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        assert!(system.uefi && system.kernelstub_config && system.kernelstub);
        assert_eq!(system.root_uuid, ROOT_UUID);
        assert_eq!(system.esp_folders, [format!("Pop_OS-{ROOT_UUID}")]);
        assert!(!system.pending_update);
        assert!(system.pop_upgrade_found.is_empty());
        assert_eq!(system.crypttab, "# none\n");
        assert_eq!(system.boot_files, Ok(()));
        assert!(system.devices.iter().any(|d| d.uuid == ROOT_UUID));
        assert_eq!(system.mountinfo, MOUNTINFO);
        // A dangling `/system-update`, a `/pop-upgrade` file, no ESP entry, no crypttab.
        symlink("/nowhere", root.join("system-update")).unwrap();
        write(root.join("pop-upgrade"), "");
        fs::remove_file(root.join("boot/efi/loader/entries/Pop_OS-current.conf")).unwrap();
        fs::remove_file(root.join("etc/crypttab")).unwrap();
        let system = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        assert!(system.pending_update);
        // In the offline boot `/system-update` is Apsis's own link (the apply's step 1
        // checked), so only `/etc/system-update` counts there.
        assert!(!system.etc_system_update);
        assert!(!system.as_system_at_apply().pending_update);
        write(root.join("etc/system-update"), "");
        let both = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        assert!(both.etc_system_update && both.as_system_at_apply().pending_update);
        fs::remove_file(root.join("etc/system-update")).unwrap();
        assert_eq!(system.pop_upgrade_found, ["/pop-upgrade"]);
        assert_eq!(
            system.boot_files,
            Err(Refusal::BootFiles(CheckFailure::NoEntry))
        );
        assert_eq!(system.crypttab, "");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_restorable_snapshot_on_its_own_machine_fills_the_dialog() {
        let root = temp("live-good");
        live(&root);
        let dir = temp("snapshot-good");
        snapshot(&dir);
        let system = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        let files = SnapshotFiles::read(&dir);
        let good = dialog(&system, &files);
        assert_eq!(good.refusal, None);
        assert!(good.has_home && !good.has_root && !good.old_format);
        assert_eq!(
            good.apsis,
            InSnapshot::NoRestore {
                version: "0.4.2".to_owned()
            }
        );
        // The same snapshot on a BIOS machine, and an empty folder on this one.
        fs::remove_dir_all(root.join("sys/firmware/efi")).unwrap();
        let bios = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        assert_eq!(dialog(&bios, &files).refusal, Some(Refusal::NotUefi));
        let empty = SnapshotFiles::read(&temp("snapshot-empty"));
        assert_eq!(
            dialog(&system, &empty).refusal,
            Some(Refusal::Unreadable(Unreadable::NoInfo))
        );
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The dialog's check while a link is in place: Apsis's
    /// own link is `restore-armed`, another tool's at either name is `pending-update`, the
    /// same split as the preparation's. A read: nothing under the root changes.
    #[test]
    fn an_armed_restore_is_told_apart_from_another_update_and_nothing_is_touched() {
        let dir = temp("snapshot-armed");
        snapshot(&dir);
        let files = SnapshotFiles::read(&dir);
        let (root, paths, exe) = lab("check-armed");
        live(&root);
        arm::arm(&paths, &exe).unwrap();
        let before = tree(&root);
        let armed = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        assert!(armed.restore_armed && armed.pending_update);
        assert_eq!(dialog(&armed, &files).refusal, Some(Refusal::RestoreArmed));
        // The offline boot's own check doesn't refuse its own link.
        assert!(!armed.as_system_at_apply().restore_armed);
        assert!(!armed.as_system_at_apply().pending_update);
        assert_eq!(tree(&root), before);
        // Armed, and another update at the other name: still the armed restore's words.
        write(root.join("etc/system-update"), "");
        let both = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
        assert_eq!(dialog(&both, &files).refusal, Some(Refusal::RestoreArmed));
        fs::remove_dir_all(&root).unwrap();
        for name in ["system-update", "etc/system-update"] {
            let (root, _, _) = lab("check-foreign");
            live(&root);
            symlink("/var/lib/other-tool", root.join(name)).unwrap();
            let before = tree(&root);
            let other = Live::read(&root, &FakeSystem, MOUNTINFO).unwrap();
            assert!(!other.restore_armed && other.pending_update, "{name}");
            assert_eq!(
                dialog(&other, &files).refusal,
                Some(Refusal::PendingUpdate),
                "{name}"
            );
            assert_eq!(tree(&root), before, "{name}");
            fs::remove_dir_all(&root).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
