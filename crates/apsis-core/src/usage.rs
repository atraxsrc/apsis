// SPDX-License-Identifier: GPL-3.0-only

//! Space on the backup device: what `statvfs` says about a mounted filesystem, Timeshift's own
//! `N snapshots, X GB free` line, and finding where a device is mounted in
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

/// The free space in Timeshift's `5 snapshots, 123.4 GB free` line (`SnapshotRepo.vala`,
/// `format_file_size`: decimal units, one decimal, `B` for small sizes). `None` for any other
/// line. Timeshift runs in the C locale for Apsis, so it's English with a `.`.
#[must_use]
pub fn parse_free_line(line: &str) -> Option<u64> {
    let (count, rest) = line.trim().split_once(" snapshots, ")?;
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let size = rest.strip_suffix(" free")?;
    let (number, unit) = size.split_once(' ')?;
    let multiplier: u64 = match unit {
        "B" => 1,
        "KB" => 1000,
        "MB" => 1000_u64.pow(2),
        "GB" => 1000_u64.pow(3),
        "TB" => 1000_u64.pow(4),
        "KiB" => 1024,
        "MiB" => 1024_u64.pow(2),
        "GiB" => 1024_u64.pow(3),
        "TiB" => 1024_u64.pow(4),
        _ => return None,
    };
    // `%'0.1f` may group thousands with commas in some locales; there's no grouping in C.
    let number = number.replace(',', "");
    let valid = !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && number.bytes().filter(|&b| b == b'.').count() <= 1;
    if !valid {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "Timeshift rounded it to one decimal already"
    )]
    let bytes = (value * multiplier as f64).round() as u64;
    Some(bytes)
}

/// Where the block device `rdev` (major, minor) is mounted, from the text of
/// `/proc/self/mountinfo`. A mount matches by its device number (ext4 and most filesystems) or
/// when `is_device` says its mount source is the device (btrfs reports an anonymous device
/// number; the helper compares canonical paths, so `/dev/mapper/x` finds `/dev/dm-0`). Bind
/// mounts of a subfolder (root other than `/`) are skipped: `statvfs` would be the same, but
/// the first plain mount is clearer.
#[must_use]
pub fn mount_points(
    mountinfo: &str,
    rdev: (u32, u32),
    is_device: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let wanted = format!("{}:{}", rdev.0, rdev.1);
    mountinfo
        .lines()
        .filter_map(|line| {
            // `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
            let (fields, after) = line.split_once(" - ")?;
            let mut fields = fields.split(' ');
            let device = fields.nth(2)?;
            let root = fields.next()?;
            let point = fields.next()?;
            let mount_source = after.split(' ').nth(1)?;
            let matches = device == wanted || is_device(Path::new(&unescape(mount_source)));
            (matches && root == "/").then(|| PathBuf::from(unescape(point)))
        })
        .collect()
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

    #[test]
    fn timeshift_free_lines() {
        let cases = [
            ("5 snapshots, 123.4 GB free", Some(123_400_000_000)),
            ("  0 snapshots, 999 B free  ", Some(999)),
            ("1 snapshots, 1.2 TB free", Some(1_200_000_000_000)),
            ("12 snapshots, 512.0 MB free", Some(512_000_000)),
            ("3 snapshots, 1.5 KB free", Some(1500)),
            ("3 snapshots, 2.0 GiB free", Some(2 * 1024 * 1024 * 1024)),
            ("3 snapshots, 1,000 B free", Some(1000)),
            ("snapshots, 1 GB free", None),
            ("x snapshots, 1 GB free", None),
            ("5 snapshots, 1 GB", None),
            ("5 snapshots, 1 PB free", None),
            ("5 snapshots, 1.2.3 GB free", None),
            ("5 snapshots, -1 GB free", None),
            ("5 snapshots, GB free", None),
            ("Status : OK", None),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_free_line(line), expected, "{line}");
        }
    }

    const MOUNTINFO: &str = "\
22 1 259:2 / / rw,relatime shared:1 - btrfs /dev/nvme0n1p2 rw,ssd,subvol=/@
23 22 259:2 /@home /home rw,relatime shared:2 - btrfs /dev/nvme0n1p2 rw,subvol=/@home
61 22 0:42 / /run/user/1000 rw,nosuid shared:3 - tmpfs tmpfs rw,size=1628k
90 61 8:17 / /media/user1/Backup\\040Disk rw,nosuid,nodev shared:4 - ext4 /dev/sdb1 rw
91 22 8:17 /timeshift /mnt/bind rw shared:4 - ext4 /dev/sdb1 rw
92 22 0:55 / /mnt/pool rw shared:5 - btrfs /dev/sdc1 rw
";

    fn source(device: &'static str) -> impl Fn(&Path) -> bool {
        move |path| path == Path::new(device)
    }

    #[test]
    fn mounts_are_found_by_device_number() {
        let points = mount_points(MOUNTINFO, (8, 17), source("/dev/sdb1"));
        assert_eq!(points, [PathBuf::from("/media/user1/Backup Disk")]);
        let by_number_only = mount_points(MOUNTINFO, (8, 17), |_| false);
        assert_eq!(by_number_only, points);
    }

    #[test]
    fn btrfs_mounts_are_found_by_source() {
        // btrfs gives an anonymous device number; the source names the device.
        let points = mount_points(MOUNTINFO, (8, 33), source("/dev/sdc1"));
        assert_eq!(points, [PathBuf::from("/mnt/pool")]);
        let root = mount_points(MOUNTINFO, (259, 2), source("/dev/nvme0n1p2"));
        assert_eq!(root, [PathBuf::from("/")]);
    }

    #[test]
    fn unmounted_devices_have_no_mount_points() {
        assert!(mount_points(MOUNTINFO, (8, 1), source("/dev/sda1")).is_empty());
        assert!(mount_points("", (8, 17), source("/dev/sdb1")).is_empty());
        assert!(mount_points("garbage line\n1 2", (8, 17), source("/dev/sdb1")).is_empty());
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
