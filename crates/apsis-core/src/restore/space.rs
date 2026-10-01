// SPDX-License-Identifier: GPL-3.0-only

//! Disk space for a restore (PLAN 6b.4), checked before anything is copied and again at
//! "Restart now".
//!
//! The sizes come from rsync `--dry-run --stats`, the free space from `statvfs`; the helper
//! runs both and passes the text and the numbers in.

use super::refusal::Refusal;

/// What must stay free on top of what's copied.
const GIB: u64 = 1 << 30;

/// "Total transferred file size" in rsync's `--stats` output, in bytes: what a run would
/// copy. `None` if the line isn't there or isn't a plain byte count.
#[must_use]
pub fn transfer_size(stats: &str) -> Option<u64> {
    let number = stats
        .lines()
        .find_map(|line| line.strip_prefix("Total transferred file size: "))?
        .strip_suffix(" bytes")?;
    // rsync groups the digits with commas. Anything else (`1.23M` with `-h`) isn't a count.
    if !number.bytes().all(|b| b.is_ascii_digit() || b == b',') {
        return None;
    }
    number.replace(',', "").parse().ok()
}

/// The size a dry run found ([`super::argv::rsync_dry_run`]), from its output.
///
/// # Errors
///
/// [`Refusal::SizeUnknown`] if the output has no size that reads as a plain byte count. A
/// size that can't be read is never taken as zero: the space check would pass on nothing.
pub fn dry_run_size(stats: &str) -> Result<u64, Refusal> {
    transfer_size(stats).ok_or(Refusal::SizeUnknown)
}

/// What the backup disk must have free for a safety snapshot that copies `transfer` bytes:
/// its size and 1 GiB.
#[must_use]
pub fn backup_needs(transfer: u64) -> u64 {
    transfer.saturating_add(GIB)
}

/// What a partition the restore writes to (`/`, or a separate `/home` being restored) must
/// have free when `transfer` bytes land on it: that and 1 GiB, or 2% of the partition
/// (`total` bytes) if that's more. An upper bound: rsync also frees space as it deletes.
#[must_use]
pub fn system_needs(transfer: u64, total: u64) -> u64 {
    transfer.saturating_add(GIB.max(total / 50))
}

/// # Errors
///
/// [`Refusal::BackupSpace`] if the backup disk has less than `needs` bytes free.
pub fn check_backup(needs: u64, free: u64) -> Result<(), Refusal> {
    if free < needs {
        return Err(Refusal::BackupSpace { needs, free });
    }
    Ok(())
}

/// One partition the restore writes to. A short separate `/home` refuses with the same line
/// as `/`.
///
/// # Errors
///
/// [`Refusal::SystemSpace`] if it has less than `needs` bytes free.
pub fn check_system(needs: u64, free: u64) -> Result<(), Refusal> {
    if free < needs {
        return Err(Refusal::SystemSpace { needs, free });
    }
    Ok(())
}

/// How a restore's transfer is spread over the partitions it writes to: what lands on `/`,
/// and what lands on a separate `/home`. `under_home` is the part of `transfer` under
/// `/home`; `None` when home is kept or `/home` is on `/`.
#[must_use]
pub fn split(transfer: u64, under_home: Option<u64>) -> (u64, Option<u64>) {
    // The two numbers come from two dry runs, and a file can change between them.
    let home = under_home.map(|home| home.min(transfer));
    (transfer - home.unwrap_or(0), home)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rsync 3.2.7, `-a --dry-run --stats` on a temp tree.
    const STATS: &str = "
Number of files: 5 (reg: 2, dir: 3)
Number of created files: 4 (reg: 2, dir: 2)
Number of deleted files: 0
Number of regular files transferred: 2
Total file size: 1,236,567 bytes
Total transferred file size: 1,236,567 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 149
Total bytes received: 30

sent 149 bytes  received 30 bytes  358.00 bytes/sec
total size is 1,236,567  speedup is 6,908.20 (DRY RUN)
";

    fn stats(line: &str) -> String {
        format!("Total file size: 99 bytes\n{line}\nLiteral data: 0 bytes\n")
    }

    /// A dry run whose size can't be read refuses: the size is never taken as zero.
    #[test]
    fn a_dry_run_without_a_readable_size_refuses() {
        assert_eq!(dry_run_size(STATS), Ok(1_236_567));
        assert_eq!(
            dry_run_size(&stats("Total transferred file size: 0 bytes")),
            Ok(0)
        );
        for text in [
            "",
            "rsync error: some files/attrs were not transferred (code 23)",
            &stats("Total transferred file size: 1.18M bytes"),
        ] {
            assert_eq!(dry_run_size(text), Err(Refusal::SizeUnknown), "{text}");
        }
    }

    #[test]
    fn the_transfer_size_is_read_from_rsyncs_stats() {
        assert_eq!(transfer_size(STATS), Some(1_236_567));
    }

    /// Not "Total file size": that counts what's already the same on both sides too.
    #[test]
    fn only_the_transferred_size_counts() {
        let text = STATS.replace(
            "Total transferred file size: 1,236,567 bytes",
            "Total transferred file size: 2,000 bytes",
        );
        assert_eq!(transfer_size(&text), Some(2000));
    }

    #[test]
    fn sizes_with_and_without_separators_are_read() {
        for (line, bytes) in [
            ("Total transferred file size: 0 bytes", 0),
            ("Total transferred file size: 512 bytes", 512),
            (
                "Total transferred file size: 14123456789 bytes",
                14_123_456_789,
            ),
            (
                "Total transferred file size: 14,123,456,789 bytes",
                14_123_456_789,
            ),
        ] {
            assert_eq!(transfer_size(&stats(line)), Some(bytes), "{line}");
        }
    }

    /// A size that isn't a plain byte count is unknown, never a smaller number.
    #[test]
    fn a_size_that_isnt_a_byte_count_is_unknown() {
        for line in [
            "Total transferred file size: 1.23M bytes",
            "Total transferred file size: 13.15G bytes",
            "Total transferred file size: 1.236.567 bytes",
            "Total transferred file size: -5 bytes",
            "Total transferred file size: bytes",
            "Total transferred file size: 12 files",
            "Total transferred file size: 12",
            "Total transferred file size: 99999999999999999999999 bytes",
            "  Total transferred file size: 12 bytes",
            "Total file size: 12 bytes",
        ] {
            assert_eq!(transfer_size(&stats(line)), None, "{line}");
        }
        assert_eq!(transfer_size(""), None);
    }

    #[test]
    fn the_backup_disk_needs_the_snapshot_and_one_gib() {
        assert_eq!(backup_needs(0), 1_073_741_824);
        assert_eq!(backup_needs(14_000_000_000), 15_073_741_824);
        assert_eq!(backup_needs(u64::MAX), u64::MAX);
    }

    #[test]
    fn a_small_system_disk_needs_the_transfer_and_one_gib() {
        // 2% of 20 GiB is less than 1 GiB.
        let total = 20 * GIB;
        assert_eq!(system_needs(6_000_000_000, total), 7_073_741_824);
        assert_eq!(system_needs(0, total), 1_073_741_824);
    }

    #[test]
    fn a_large_system_disk_needs_the_transfer_and_two_percent() {
        // 2% of 500 GB is 10 GB.
        assert_eq!(system_needs(6_000_000_000, 500_000_000_000), 16_000_000_000);
        // Exactly 50 GiB: 2% is 1 GiB either way.
        assert_eq!(system_needs(5, 50 * GIB), GIB + 5);
        assert_eq!(system_needs(u64::MAX, 500_000_000_000), u64::MAX);
    }

    #[test]
    fn a_backup_disk_thats_short_is_refused_with_both_numbers() {
        assert_eq!(
            check_backup(14_000_000_000, 9_000_000_000),
            Err(Refusal::BackupSpace {
                needs: 14_000_000_000,
                free: 9_000_000_000
            })
        );
        assert_eq!(
            check_backup(1001, 1000),
            Err(Refusal::BackupSpace {
                needs: 1001,
                free: 1000
            })
        );
    }

    #[test]
    fn a_system_partition_thats_short_is_refused_with_both_numbers() {
        assert_eq!(
            check_system(6_000_000_000, 3_000_000_000),
            Err(Refusal::SystemSpace {
                needs: 6_000_000_000,
                free: 3_000_000_000
            })
        );
    }

    #[test]
    fn exactly_enough_free_space_is_enough() {
        assert_eq!(check_backup(1000, 1000), Ok(()));
        assert_eq!(check_backup(1000, 5000), Ok(()));
        assert_eq!(check_system(1000, 1000), Ok(()));
        assert_eq!(check_system(0, 0), Ok(()));
    }

    /// PLAN 6b.4: what lands under a separate `/home` is checked against that partition, and
    /// only the rest against `/`.
    #[test]
    fn a_separate_homes_part_comes_off_the_roots() {
        assert_eq!(split(10_000, Some(4000)), (6000, Some(4000)));
        assert_eq!(split(10_000, Some(0)), (10_000, Some(0)));
        assert_eq!(split(10_000, None), (10_000, None));
    }

    /// The two dry runs can disagree by a file that changed between them: the home part is
    /// never more than the whole, and `/` never needs less than nothing.
    #[test]
    fn a_home_part_larger_than_the_whole_takes_all_of_it() {
        assert_eq!(split(10_000, Some(12_000)), (0, Some(10_000)));
    }

    /// The numbers of the plan's own example lines, end to end.
    #[test]
    fn a_restore_onto_a_full_small_disk_is_refused() {
        let total = 40 * GIB;
        let transfer =
            transfer_size(&stats("Total transferred file size: 5,000,000,000 bytes")).unwrap();
        let (root, home) = split(transfer, None);
        assert_eq!(home, None);
        let needs = system_needs(root, total);
        assert_eq!(
            check_system(needs, 3_000_000_000),
            Err(Refusal::SystemSpace {
                needs: 6_073_741_824,
                free: 3_000_000_000
            })
        );
        assert_eq!(check_system(needs, 6_073_741_824), Ok(()));
    }
}
