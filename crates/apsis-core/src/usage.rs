// SPDX-License-Identifier: GPL-3.0-only

//! Space on the backup device (what `statvfs` says about a mounted filesystem), and reading
//! `/proc/self/mountinfo`. No syscalls here; `apsis-helper` makes them.

use std::path::{Path, PathBuf};

/// Size, used and free space of the backup device's filesystem, in bytes, as `df` shows them.
///
/// `used + free` can be less than `total`: ext4 keeps blocks back for root, and `free` is what
/// is left for everyone (`f_bavail`), as in `df`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskUsage {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

impl DiskUsage {
    /// From `statvfs` fields: block count, free blocks, blocks free for everyone, fragment size.
    /// `None` for a filesystem with no size (`/proc` and the like), or numbers that don't add up.
    #[must_use]
    pub fn from_statvfs(blocks: u64, bfree: u64, bavail: u64, frsize: u64) -> Option<Self> {
        if blocks == 0 || frsize == 0 || bfree > blocks || bavail > bfree {
            return None;
        }
        Some(Self {
            total: blocks.checked_mul(frsize)?,
            used: (blocks - bfree).checked_mul(frsize)?,
            free: bavail.checked_mul(frsize)?,
        })
    }

    /// Share of the usable space still free, `0.0..=1.0`: `free / (used + free)`.
    #[must_use]
    pub fn free_fraction(&self) -> f64 {
        let usable = self.used.saturating_add(self.free);
        if usable == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss, reason = "a fraction for a colour")]
        let fraction = self.free as f64 / usable as f64;
        fraction.clamp(0.0, 1.0)
    }

    /// Share of the usable space in use, `0.0..=1.0`: `used / (used + free)`, as `df`'s `Use%`.
    #[must_use]
    pub fn used_fraction(&self) -> f64 {
        let usable = self.used.saturating_add(self.free);
        if usable == 0 {
            return 1.0;
        }
        #[allow(clippy::cast_precision_loss, reason = "a fraction for a bar")]
        let fraction = self.used as f64 / usable as f64;
        fraction.clamp(0.0, 1.0)
    }
}

/// Mount points at `path` or anywhere below it, from the text of `/proc/self/mountinfo`
/// (field 5, unescaped). A delete refuses a snapshot folder that has any: a bind mount inside
/// it has the same device number as the folder, so walking the tree can't tell it apart.
#[must_use]
pub fn mounts_under(mountinfo: &str, path: &Path) -> Vec<PathBuf> {
    mountinfo
        .lines()
        .filter_map(|line| {
            // `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
            let (fields, _) = line.split_once(" - ")?;
            let point = PathBuf::from(unescape(fields.split(' ').nth(4)?));
            point.starts_with(path).then_some(point)
        })
        .collect()
}

/// The filesystem type of what's mounted at exactly `path`, from the text of
/// `/proc/self/mountinfo`. A later mount covers an earlier one, so it's the last such line.
#[must_use]
pub fn fstype_at<'a>(mountinfo: &'a str, path: &Path) -> Option<&'a str> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (fields, rest) = line.split_once(" - ")?;
            let point = unescape(fields.split(' ').nth(4)?);
            let fstype = rest.split(' ').next().filter(|t| !t.is_empty())?;
            (Path::new(&point) == path).then_some(fstype)
        })
        .next_back()
}

/// mountinfo writes space, tab, newline and backslash as `\040`, `\011`, `\012`, `\134`.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 1..i + 4)
            .filter(|digits| bytes[i] == b'\\' && digits.iter().all(|d| (b'0'..=b'7').contains(d)));
        match octal.and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok())
        {
            Some(byte) => {
                out.push(byte);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `uuid` is safe to put in `/dev/disk/by-uuid/<uuid>`: hex digits and dashes (ext4,
/// btrfs, LUKS) or the `XXXX-XXXX` of FAT, nothing that could leave the folder.
#[must_use]
pub fn is_plain_uuid(uuid: &str) -> bool {
    !uuid.is_empty() && uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statvfs_numbers_become_df_numbers() {
        let usage = DiskUsage::from_statvfs(1000, 400, 350, 4096).unwrap();
        assert_eq!(usage.total, 4_096_000);
        assert_eq!(usage.used, 600 * 4096);
        assert_eq!(usage.free, 350 * 4096);
        let fraction = usage.used_fraction();
        assert!((fraction - 600.0 / 950.0).abs() < 1e-9, "{fraction}");
        let free = usage.free_fraction();
        assert!((free - 350.0 / 950.0).abs() < 1e-9, "{free}");
    }

    #[test]
    fn empty_or_inconsistent_statvfs_is_unknown() {
        assert_eq!(DiskUsage::from_statvfs(0, 0, 0, 4096), None);
        assert_eq!(DiskUsage::from_statvfs(10, 0, 0, 0), None);
        assert_eq!(DiskUsage::from_statvfs(10, 11, 0, 4096), None);
        assert_eq!(DiskUsage::from_statvfs(10, 5, 6, 4096), None);
        assert_eq!(DiskUsage::from_statvfs(u64::MAX, 0, 0, 4096), None);
    }

    #[test]
    fn full_disk_is_all_used() {
        let usage = DiskUsage::from_statvfs(100, 0, 0, 1).unwrap();
        assert!((usage.used_fraction() - 1.0).abs() < f64::EPSILON);
        let empty = DiskUsage::from_statvfs(100, 100, 100, 1).unwrap();
        assert!(empty.used_fraction().abs() < f64::EPSILON);
    }

    const MOUNTINFO: &str = "\
22 1 259:2 / / rw,relatime shared:1 - btrfs /dev/nvme0n1p2 rw,ssd,subvol=/@
23 22 259:2 /@home /home rw,relatime shared:2 - btrfs /dev/nvme0n1p2 rw,subvol=/@home
61 22 0:42 / /run/user/1000 rw,nosuid shared:3 - tmpfs tmpfs rw,size=1628k
90 61 8:17 / /media/user1/Backup\\040Disk rw,nosuid,nodev shared:4 - ext4 /dev/sdb1 rw
91 22 8:17 /timeshift /mnt/bind rw shared:4 - ext4 /dev/sdb1 rw
92 22 0:55 / /mnt/pool rw shared:5 - btrfs /dev/sdc1 rw
";

    #[test]
    fn mounts_at_or_below_a_path_are_found() {
        let under = |path: &str| mounts_under(MOUNTINFO, Path::new(path));
        assert_eq!(
            under("/media/user1"),
            [PathBuf::from("/media/user1/Backup Disk")]
        );
        assert_eq!(under("/mnt/pool"), [PathBuf::from("/mnt/pool")]);
        assert_eq!(
            under("/mnt"),
            [PathBuf::from("/mnt/bind"), PathBuf::from("/mnt/pool")]
        );
        // Whole components only: /mnt/po isn't /mnt/pool's parent.
        assert!(under("/mnt/po").is_empty());
        assert!(under("/srv").is_empty());
        assert!(mounts_under("garbage line\n1 2", Path::new("/")).is_empty());
    }

    #[test]
    fn the_filesystem_type_at_a_mount_point_is_the_last_ones() {
        let at = |path: &str| fstype_at(MOUNTINFO, Path::new(path));
        assert_eq!(at("/"), Some("btrfs"));
        assert_eq!(at("/media/user1/Backup Disk"), Some("ext4"));
        // Exactly there: not a mount above or below it.
        assert_eq!(at("/media"), None);
        assert_eq!(at("/home/user1"), None);
        let covered = format!("{MOUNTINFO}99 22 8:33 / /mnt/pool rw - xfs /dev/sdd1 rw\n");
        assert_eq!(fstype_at(&covered, Path::new("/mnt/pool")), Some("xfs"));
        assert_eq!(fstype_at("garbage line\n1 2 - ", Path::new("/")), None);
    }

    #[test]
    fn mountinfo_escapes_are_undone() {
        assert_eq!(unescape("a\\040b\\011c\\134d"), "a b\tc\\d");
        assert_eq!(unescape("trailing\\04"), "trailing\\04");
        assert_eq!(unescape("\\999"), "\\999");
    }

    #[test]
    fn only_plain_uuids_become_paths() {
        assert!(is_plain_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(is_plain_uuid("ABCD-1234"));
        for bad in ["", "../sda1", "a/b", "uuid with space", "x"] {
            assert!(!is_plain_uuid(bad), "{bad}");
        }
    }
}
