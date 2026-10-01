// SPDX-License-Identifier: GPL-3.0-only

//! The rsync filter a restore runs with, saved as `/var/lib/apsis/restore/restore.filter` and
//! passed with `--exclude-from` (PLAN 6b.2).
//!
//! rsync reads the rules top to bottom, the first match wins, and each is anchored at the
//! transfer root (`/`). The restore never uses `--delete-excluded`, so an excluded path is
//! also protected from `--delete`: it stays exactly as it is on the live system.

use std::path::Path;

use crate::error::{Error, Result};
use crate::usage::mounts_under;

/// Group 1, always first: what runs the restore, its state, and the live Apsis config. None of
/// these belongs to the Apsis package (a test checks them against the .deb's file list).
pub const PROTECTED: [&str; 6] = [
    "/system-update",
    "/etc/systemd/system/apsis-restore.service",
    "/etc/systemd/system/system-update.target.wants/apsis-restore.service",
    // The pop-upgrade-init drop-in (PLAN 6b.6): written on arm, and kept through the copy so
    // a retry boot still has it after a snapshot without it was restored.
    "/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf",
    "/var/lib/apsis/***",
    "/etc/apsis/***",
];

/// What happens to `/home` (PLAN 6b.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Home {
    /// `/home` is fully excluded: nothing there is compared, written or deleted.
    Keep,
    /// `/home` becomes what the snapshot holds.
    Restore,
}

/// What the filter is built from.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The text of `/proc/self/mountinfo`.
    pub mountinfo: &'a str,
    pub home: Home,
    /// Pass 1: the version of the kernel the ESP boots now (`uname -r`), whose files stay
    /// until the ESP is refreshed and checked. `None` leaves the rule out.
    pub protected_kernel: Option<&'a str>,
    /// The text of the snapshot's own `exclude.list`.
    pub snapshot_excludes: &'a str,
}

/// The whole filter, one rule per entry, in order.
///
/// # Errors
///
/// [`Error::InvalidInput`] for a mount point or kernel version that can't be written as one
/// rule.
pub fn rules(request: &Request<'_>) -> Result<Vec<String>> {
    let mut list: Vec<String> = Vec::new();
    // A repeated exclude changes nothing (the first match wins), so each is written once.
    let mut exclude = |pattern: &str| {
        let rule = format!("- {pattern}");
        if !list.contains(&rule) {
            list.push(rule);
        }
    };
    for pattern in PROTECTED.iter().chain(&ESP).chain(&RECOVERY).chain(&DISKS) {
        exclude(pattern);
    }
    // Every other mount: rsync would go into it, and `--delete` there. The plain path, not
    // `<mount>/***`: that only matches a folder, and a mount point can be a file. rsync never
    // enters an excluded folder, so what's in it is covered too.
    for point in mounts_under(request.mountinfo, Path::new("/")) {
        let point = point.to_string_lossy();
        // `/` is the restore's target. A separate `/home` is entered when home is restored.
        if point == "/" || (point == "/home" && request.home == Home::Restore) {
            continue;
        }
        exclude(&escape(&point)?);
    }
    for pattern in RUNTIME.iter().chain(&BACKUP_ON_ROOT).chain(&JOURNAL) {
        exclude(pattern);
    }
    if request.home == Home::Keep {
        exclude("/home/***");
    }
    if let Some(version) = request.protected_kernel {
        if !is_kernel_version(version) {
            return Err(Error::InvalidInput(format!(
                "not a kernel version: {version:?}"
            )));
        }
        // Pop!_OS has a merged /usr: /lib is a symlink to usr/lib.
        exclude(&format!("/usr/lib/modules/{version}/***"));
        for file in KERNEL_FILES {
            exclude(&format!("/boot/{file}-{version}"));
        }
    }
    // What the snapshot didn't save stays as it is instead of being deleted. Its lines are
    // rsync rules already, in the order they were taken with.
    list.extend(
        request
            .snapshot_excludes
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned),
    );
    Ok(list)
}

/// The ESP (vfat). Only kernelstub writes it, and Apsis's own put-back.
const ESP: [&str; 1] = ["/boot/efi/***"];
/// Pop's recovery partition: the way back if a restore breaks booting.
const RECOVERY: [&str; 1] = ["/recovery/***"];
/// They describe the disks as they are now; the initramfs is rebuilt against them.
const DISKS: [&str; 2] = ["/etc/fstab", "/etc/crypttab"];
/// Pseudo and runtime filesystems, temporary files and other disks' mount places. Mostly
/// caught by the mounts and the snapshot's list too; stated so it depends on neither.
const RUNTIME: [&str; 9] = [
    "/dev/***",
    "/proc/***",
    "/sys/***",
    "/run/***",
    "/tmp/***",
    "/mnt/***",
    "/media/***",
    "/lost+found",
    "/swapfile",
];
/// A backup kept on the system disk itself (Timeshift's layout).
const BACKUP_ON_ROOT: [&str; 1] = ["/timeshift/***"];
/// journald writes there during the apply; keeping it keeps the restore's own log.
const JOURNAL: [&str; 1] = ["/var/log/journal/***"];
/// A kernel's files in `/boot`, each `<name>-<version>`.
const KERNEL_FILES: [&str; 4] = ["vmlinuz", "initrd.img", "config", "System.map"];

/// `uname -r`, and nothing that could be a path or a pattern.
pub(super) fn is_kernel_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_+~".contains(&b))
}

/// A path as an rsync rule that matches only it. rsync reads `\` as an escape only in a
/// rule that has a wildcard character (`*`, `?`, `[`); in any other rule it's the character
/// itself. So a path with a wildcard character gets a `\` in front of each and of each `\`,
/// and any other path is written as it is.
fn escape(path: &str) -> Result<String> {
    // A filter file is read line by line, and rsync ends a line at either.
    if path.contains(['\n', '\r']) {
        return Err(Error::InvalidInput(format!(
            "a mount point with a line break can't be protected: {path:?}"
        )));
    }
    if !path.contains(['*', '?', '[']) {
        return Ok(path.to_owned());
    }
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    Ok(out)
}

/// The filter file's text: each rule, then `\n`.
#[must_use]
pub fn to_text(rules: &[String]) -> String {
    rules
        .iter()
        .flat_map(|rule| [rule.as_str(), "\n"])
        .collect()
}

/// Whether a snapshot holds home files: its `exclude.list` lets something under `/home` in
/// (Apsis's `+ /home/**`, Timeshift's per-user `+ /home/<user>/**`).
#[must_use]
pub fn has_home(snapshot_excludes: &str) -> bool {
    snapshot_excludes
        .lines()
        .any(|line| line.starts_with("+ /home/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Pop!_OS machine as apsis-test: ESP, recovery, a data disk, the backup disk mounted
    /// by the helper and by the desktop, and a bind mount. Devices are placeholders.
    const MOUNTINFO: &str = "\
24 1 259:3 / / rw,relatime shared:1 - ext4 /dev/sdX3 rw
25 24 0:5 / /dev rw,nosuid shared:2 - devtmpfs udev rw
26 24 0:22 / /proc rw shared:3 - proc proc rw
27 24 0:23 / /sys rw shared:4 - sysfs sysfs rw
28 24 0:24 / /run rw shared:5 - tmpfs tmpfs rw
30 24 259:1 / /boot/efi rw,relatime shared:6 - vfat /dev/sdX1 rw
31 24 259:2 / /recovery rw shared:7 - vfat /dev/sdX2 rw
32 24 8:1 / /data rw shared:8 - ext4 /dev/sdY1 rw
33 28 8:17 / /run/apsis/backup ro,nosuid,nodev,noexec - ext4 /dev/sdZ1 ro
34 24 8:17 / /media/user1/Backup\\040Disk rw shared:9 - ext4 /dev/sdZ1 rw
35 24 8:1 /stuff /srv/bind rw shared:8 - ext4 /dev/sdY1 rw
";

    const KERNEL: &str = "6.9.3-76060903-generic";
    const EXCLUDES: &str = "/dev/*\n+ /root/**\n/root/**\n/home/*/**\n";
    /// A snapshot made by Timeshift 24.01.1 (redacted).
    const REAL_EXCLUDES: &str = include_str!(
        "../../tests/fixtures/native-repo/timeshift/snapshots/2026-09-20_10-00-00/exclude.list"
    );

    fn request<'a>(mountinfo: &'a str, home: Home) -> Request<'a> {
        Request {
            mountinfo,
            home,
            protected_kernel: Some(KERNEL),
            snapshot_excludes: EXCLUDES,
        }
    }

    fn with_separate_home() -> String {
        format!(
            "{MOUNTINFO}36 24 8:49 / /home rw shared:10 - ext4 /dev/sdW1 rw\n\
             37 36 8:65 / /home/user1/disk rw shared:11 - ext4 /dev/sdV1 rw\n"
        )
    }

    #[test]
    fn the_whole_filter_for_pass_one_keeping_home() {
        let list = rules(&request(MOUNTINFO, Home::Keep)).unwrap();
        assert_eq!(
            list,
            [
                "- /system-update",
                "- /etc/systemd/system/apsis-restore.service",
                "- /etc/systemd/system/system-update.target.wants/apsis-restore.service",
                "- /etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf",
                "- /var/lib/apsis/***",
                "- /etc/apsis/***",
                "- /boot/efi/***",
                "- /recovery/***",
                "- /etc/fstab",
                "- /etc/crypttab",
                "- /dev",
                "- /proc",
                "- /sys",
                "- /run",
                "- /boot/efi",
                "- /recovery",
                "- /data",
                "- /run/apsis/backup",
                "- /media/user1/Backup Disk",
                "- /srv/bind",
                "- /dev/***",
                "- /proc/***",
                "- /sys/***",
                "- /run/***",
                "- /tmp/***",
                "- /mnt/***",
                "- /media/***",
                "- /lost+found",
                "- /swapfile",
                "- /timeshift/***",
                "- /var/log/journal/***",
                "- /home/***",
                "- /usr/lib/modules/6.9.3-76060903-generic/***",
                "- /boot/vmlinuz-6.9.3-76060903-generic",
                "- /boot/initrd.img-6.9.3-76060903-generic",
                "- /boot/config-6.9.3-76060903-generic",
                "- /boot/System.map-6.9.3-76060903-generic",
                "/dev/*",
                "+ /root/**",
                "/root/**",
                "/home/*/**",
            ]
        );
    }

    #[test]
    fn the_protect_list_comes_first() {
        for home in [Home::Keep, Home::Restore] {
            let list = rules(&request(MOUNTINFO, home)).unwrap();
            let first: Vec<String> = PROTECTED.iter().map(|p| format!("- {p}")).collect();
            assert_eq!(list[..PROTECTED.len()], first[..]);
        }
    }

    #[test]
    fn restoring_home_leaves_the_home_rule_out() {
        let list = rules(&request(MOUNTINFO, Home::Restore)).unwrap();
        assert!(!list.iter().any(|r| r == "- /home/***"), "{list:?}");
        // What the snapshot left out of the homes still stays as it is.
        assert_eq!(list.last().unwrap(), "/home/*/**");
    }

    /// PLAN 6b.2 rule 5 and 6b.3: a separate `/home` is another disk like any other when home
    /// is kept, and is the one mount rsync goes into when home is restored.
    #[test]
    fn a_separate_home_is_entered_only_when_home_is_restored() {
        let mountinfo = with_separate_home();
        let kept = rules(&request(&mountinfo, Home::Keep)).unwrap();
        assert_eq!(kept.iter().filter(|r| *r == "- /home").count(), 1);
        assert_eq!(kept.iter().filter(|r| *r == "- /home/***").count(), 1);
        let restored = rules(&request(&mountinfo, Home::Restore)).unwrap();
        assert!(
            !restored
                .iter()
                .any(|r| r == "- /home" || r == "- /home/***"),
            "{restored:?}"
        );
        // A disk mounted inside a home is still another disk.
        assert!(restored.iter().any(|r| r == "- /home/user1/disk"));
    }

    /// The restore's target is `/`: its own mount never gets a rule, however often the mount
    /// table lists it (a stacked root, or a sandbox binding it again), or nothing would be
    /// restored at all.
    #[test]
    fn the_targets_own_mount_gets_no_rule() {
        let stacked = format!(
            "1 0 0:1 / / rw - rootfs rootfs rw\n{MOUNTINFO}99 24 259:3 / / rw - ext4 /dev/sdX3 rw\n"
        );
        for mountinfo in [MOUNTINFO, stacked.as_str()] {
            for home in [Home::Keep, Home::Restore] {
                let list = rules(&request(mountinfo, home)).unwrap();
                let whole_target = ["- /", "- /*", "- /**", "- /***", "- ", "- //***"];
                assert!(
                    !list.iter().any(|r| whole_target.contains(&r.as_str())),
                    "{list:?}"
                );
            }
        }
        // The same list either way: the extra lines for `/` add nothing.
        assert_eq!(
            rules(&request(&stacked, Home::Keep)).unwrap(),
            rules(&request(MOUNTINFO, Home::Keep)).unwrap()
        );
    }

    /// A mount rule has no `/***`: that only matches a folder, and a file can be a mount
    /// point too (checked with rsync 3.2.7, see DECISIONS.md).
    #[test]
    fn a_mount_rule_is_the_plain_anchored_path() {
        let mountinfo = "24 1 8:1 / / rw - ext4 /dev/sdX1 rw\n\
            40 24 8:1 /x /etc/resolv.conf rw - ext4 /dev/sdX1 rw\n";
        let list = rules(&request(mountinfo, Home::Keep)).unwrap();
        assert!(list.iter().any(|r| r == "- /etc/resolv.conf"), "{list:?}");
        assert!(!list.iter().any(|r| r == "- /etc/resolv.conf/***"));
    }

    #[test]
    fn the_kernel_rule_is_only_in_pass_one() {
        let later = Request {
            protected_kernel: None,
            ..request(MOUNTINFO, Home::Keep)
        };
        let list = rules(&later).unwrap();
        assert!(
            !list
                .iter()
                .any(|r| r.contains("/usr/lib/modules") || r.starts_with("- /boot/vmlinuz")),
            "{list:?}"
        );
        // Everything else is the same.
        let first = rules(&request(MOUNTINFO, Home::Keep)).unwrap();
        assert_eq!(list.len() + 5, first.len());
    }

    #[test]
    fn a_kernel_version_that_isnt_one_is_refused() {
        for bad in ["", "6.9/../x", "6.9*", "6.9 generic", "6.9\n", "[6]"] {
            let odd = Request {
                protected_kernel: Some(bad),
                ..request(MOUNTINFO, Home::Keep)
            };
            assert!(
                matches!(rules(&odd), Err(Error::InvalidInput(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_snapshots_own_list_is_appended_line_for_line() {
        let real = Request {
            snapshot_excludes: REAL_EXCLUDES,
            ..request(MOUNTINFO, Home::Keep)
        };
        let list = rules(&real).unwrap();
        let lines: Vec<&str> = REAL_EXCLUDES.lines().collect();
        assert!(lines.len() > 50);
        // Repeated lines stay too: it's the snapshot's list, unchanged.
        assert_eq!(list[list.len() - lines.len()..], lines[..]);
    }

    #[test]
    fn blank_lines_in_the_snapshots_list_are_dropped() {
        let gaps = Request {
            snapshot_excludes: "/a\n\n/b\n",
            ..request(MOUNTINFO, Home::Keep)
        };
        let list = rules(&gaps).unwrap();
        assert_eq!(list[list.len() - 2..], ["/a", "/b"]);
    }

    #[test]
    fn wildcard_characters_in_a_mount_point_are_escaped() {
        let mountinfo = "24 1 8:1 / / rw - ext4 /dev/sdX1 rw\n\
            40 24 8:2 / /srv/a*b?[c]\\134d rw - ext4 /dev/sdX2 rw\n";
        let list = rules(&request(mountinfo, Home::Keep)).unwrap();
        assert!(
            list.iter().any(|r| r == r"- /srv/a\*b\?\[c]\\d"),
            "{list:?}"
        );
    }

    /// rsync reads `\` as an escape only in a rule that has a wildcard; in any other it's the
    /// character itself.
    #[test]
    fn a_backslash_alone_in_a_mount_point_is_left_as_it_is() {
        let mountinfo = "24 1 8:1 / / rw - ext4 /dev/sdX1 rw\n\
            40 24 8:2 / /srv/c\\134d rw - ext4 /dev/sdX2 rw\n";
        let list = rules(&request(mountinfo, Home::Keep)).unwrap();
        assert!(list.iter().any(|r| r == r"- /srv/c\d"), "{list:?}");
    }

    #[test]
    fn a_mount_point_with_a_line_break_is_refused() {
        for escape in ["\\012", "\\015"] {
            let mountinfo = format!(
                "24 1 8:1 / / rw - ext4 /dev/sdX1 rw\n\
                 40 24 8:2 / /srv/a{escape}b rw - ext4 /dev/sdX2 rw\n"
            );
            let result = rules(&request(&mountinfo, Home::Keep));
            assert!(matches!(result, Err(Error::InvalidInput(_))), "{escape}");
        }
    }

    #[test]
    fn text_is_one_rule_per_line() {
        let list = ["- /a/***".to_owned(), "/b".to_owned()];
        assert_eq!(to_text(&list), "- /a/***\n/b\n");
    }

    #[test]
    fn home_is_in_a_snapshot_when_its_list_lets_it_in() {
        // Apsis with /home included, Timeshift per user, hidden files only.
        assert!(has_home("/dev/*\n+ /home/**\n/root/**\n/home/*/**\n"));
        assert!(has_home(REAL_EXCLUDES));
        assert!(has_home("+ /home/user1/.**\n/home/*/**\n"));
        // /root only, nothing, or a home that's left out.
        assert!(!has_home("+ /root/**\n/root/**\n/home/*/**\n"));
        assert!(!has_home(""));
        assert!(!has_home("/home/user1/**\n- /home/user2/***\n"));
        assert!(!has_home("+ /homework/**\n"));
    }

    /// Where the .deb puts each file, from `[package.metadata.deb]`'s assets.
    fn packaged_paths() -> Vec<String> {
        let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/../apsis/Cargo.toml");
        let text = std::fs::read_to_string(manifest).unwrap();
        let manifest: toml::Value = toml::from_str(&text).unwrap();
        let assets = manifest["package"]["metadata"]["deb"]["assets"]
            .as_array()
            .unwrap();
        assets
            .iter()
            .map(|asset| {
                let source = asset[0].as_str().unwrap();
                let target = asset[1].as_str().unwrap();
                match target.strip_suffix('/') {
                    Some(dir) => format!("/{dir}/{}", source.rsplit('/').next().unwrap()),
                    None => format!("/{target}"),
                }
            })
            .collect()
    }

    #[test]
    fn nothing_the_package_installs_is_protected() {
        let packaged = packaged_paths();
        assert!(packaged.contains(&"/usr/libexec/apsis-helper".to_owned()));
        assert!(packaged.len() >= 12, "{packaged:?}");
        for path in &packaged {
            for rule in PROTECTED {
                let covered = match rule.strip_suffix("/***") {
                    Some(dir) => path == dir || path.starts_with(&format!("{dir}/")),
                    None => path == rule,
                };
                assert!(!covered, "{path} is protected by {rule}");
            }
        }
    }
}
