// SPDX-License-Identifier: GPL-3.0-only

//! Plain-text formatting for the popup and tooltip. No widgets, so it's unit-testable.

use apsis_core::Snapshot;
use jiff::civil::DateTime;

/// `2026-09-18 12:41`
#[must_use]
pub fn when(created: DateTime) -> String {
    created.strftime("%Y-%m-%d %H:%M").to_string()
}

/// `09-27 09:12 "bulk 1"`, or `09-27 09:12` without a comment: a snapshot as the delete
/// prompts, progress and results name it. The whole comment; long lines wrap.
#[must_use]
pub fn label(snapshot: &Snapshot) -> String {
    let when = snapshot.created.strftime("%m-%d %H:%M").to_string();
    match &snapshot.comment {
        Some(_) => format!("{when} {}", quoted_comment(snapshot)),
        None => when,
    }
}

/// `"before kernel update"`, or nothing.
#[must_use]
pub fn quoted_comment(snapshot: &Snapshot) -> String {
    snapshot
        .comment
        .as_deref()
        .map(|c| format!("\"{c}\""))
        .unwrap_or_default()
}

/// At most `max` characters, ending in `…` when shortened.
#[must_use]
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max.saturating_sub(1)).collect();
    short.push('…');
    short
}

/// `1a2b…`: enough of a filesystem UUID to recognise the disk.
#[must_use]
pub fn short_uuid(uuid: &str) -> String {
    truncate(uuid, 5)
}

pub use apsis_core::status::{size, size_short};

/// `58%`: whole percent, rounded down, so `100%` means done.
#[must_use]
pub fn percent(percent: f64) -> String {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100"
    )]
    let whole = percent.clamp(0.0, 100.0).floor() as u8;
    format!("{whole}%")
}

/// A time span to the second, for the time left and the time elapsed: `42s`, `3m 12s`,
/// `1h 02m` (seconds are dropped from an hour up).
#[must_use]
pub fn duration(seconds: u64) -> String {
    match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s:02}s"),
        (h, m, _) => format!("{h}h {m:02}m"),
    }
}

/// `sdX1` for `/dev/sdX1`: the device's name without the folder.
#[must_use]
pub fn device_name(device: &str) -> &str {
    device.rsplit('/').next().unwrap_or(device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apsis_core::Tag;
    use jiff::civil::date;

    #[test]
    fn labels_are_the_short_date_and_the_quoted_comment() {
        let mut snapshot = Snapshot {
            name: "2026-09-27_09-12-33".to_owned(),
            created: date(2026, 9, 27).at(9, 12, 33, 0),
            tags: vec![Tag::OnDemand],
            rsync_flags: None,
            comment: Some("bulk 1".to_owned()),
        };
        assert_eq!(label(&snapshot), "09-27 09:12 \"bulk 1\"");
        snapshot.comment = None;
        assert_eq!(label(&snapshot), "09-27 09:12");
    }

    #[test]
    fn sizes_like_lsblk() {
        assert_eq!(size(0), "0B");
        assert_eq!(size(512 * 1024 * 1024), "512M");
        assert_eq!(size(1_000_203_837_440), "931.5G");
        assert_eq!(size(2 * 1024_u64.pow(4)), "2T");
    }

    #[test]
    fn short_sizes_drop_the_decimal_from_ten_up() {
        assert_eq!(size_short(0), "0B");
        assert_eq!(size_short(4_831_838_208), "4.5G");
        assert_eq!(size_short(481_036_337_152), "448G");
        assert_eq!(size_short(1_000_203_837_440), "932G");
        assert_eq!(size_short(2 * 1024_u64.pow(4)), "2T");
    }

    #[test]
    fn percents_round_down() {
        assert_eq!(percent(58.23), "58%");
        assert_eq!(percent(99.91), "99%");
        assert_eq!(percent(100.0), "100%");
        assert_eq!(percent(-3.0), "0%");
    }

    #[test]
    fn durations_are_to_the_second_and_drop_seconds_from_an_hour() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(59), "59s");
        assert_eq!(duration(60), "1m 00s");
        assert_eq!(duration(68), "1m 08s");
        assert_eq!(duration(192), "3m 12s");
        assert_eq!(duration(3599), "59m 59s");
        assert_eq!(duration(3600), "1h 00m");
        assert_eq!(duration(2 * 3600 + 5 * 60 + 9), "2h 05m");
        assert_eq!(duration(7325), "2h 02m");
    }

    #[test]
    fn device_names_lose_their_folder() {
        assert_eq!(device_name("/dev/sdX1"), "sdX1");
        assert_eq!(device_name("/dev/mapper/luks-1"), "luks-1");
        assert_eq!(device_name("sdb"), "sdb");
    }

    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate("short", 5), "short");
        assert_eq!(truncate("longer", 5), "long…");
        assert_eq!(truncate("äöüäöü", 4), "äöü…");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn short_uuid_keeps_four_characters() {
        assert_eq!(short_uuid("1a2b1234-0000-0000-0000-000000000000"), "1a2b…");
        assert_eq!(short_uuid("1a2b"), "1a2b");
    }
}
