// SPDX-License-Identifier: GPL-3.0-only

//! Plain-text formatting for the popup and tooltip. No widgets, so it's unit-testable.

use apsis_core::{DiskUsage, Mode, Snapshot, Tag};
use jiff::civil::DateTime;

/// How long ago `then` was, as seen at `now`: `just now`, `5m ago`, `3h ago`, `2d ago`.
///
/// Both are local civil times. A `then` in the future (clock changes) reads as `just now`.
#[must_use]
pub fn ago(then: DateTime, now: DateTime) -> String {
    let minutes = now.duration_since(then).as_secs() / 60;
    match minutes {
        ..1 => "just now".to_owned(),
        1..60 => format!("{minutes}m ago"),
        60..2880 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

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

/// `OB`, the way Timeshift prints the Tags column.
#[must_use]
pub fn tag_letters(tags: &[Tag]) -> String {
    tags.iter().map(|t| t.letter()).collect()
}

/// `O on-demand, B boot`
#[must_use]
pub fn tag_names(tags: &[Tag]) -> String {
    if tags.is_empty() {
        return "-".to_owned();
    }
    tags.iter()
        .map(|t| format!("{} {}", t.letter(), t.name()))
        .collect::<Vec<_>>()
        .join(", ")
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

/// `rsync · 3 snapshots`
#[must_use]
pub fn summary(mode: Option<Mode>, count: usize) -> String {
    let noun = if count == 1 { "snapshot" } else { "snapshots" };
    match mode {
        Some(Mode::Rsync) => format!("rsync · {count} {noun}"),
        Some(Mode::Btrfs) => format!("btrfs · {count} {noun}"),
        None => format!("{count} {noun}"),
    }
}

/// `1a2b…`: enough of a filesystem UUID to recognise the disk.
#[must_use]
pub fn short_uuid(uuid: &str) -> String {
    truncate(uuid, 5)
}

pub use apsis_core::status::{size, size_short};

/// The disk line's text: `448G used · 483G free · 9 snapshots` from `statvfs`. `None` when
/// the usage isn't known: nothing is shown then.
#[must_use]
pub fn disk_text(usage: Option<DiskUsage>, count: usize) -> Option<String> {
    let noun = if count == 1 { "snapshot" } else { "snapshots" };
    let usage = usage?;
    Some(format!(
        "{} used · {} free · {count} {noun}",
        size_short(usage.used),
        size_short(usage.free)
    ))
}

/// How many of a bar's `cells` are filled for `fraction` used, and how many are empty.
/// Anything used shows at least one filled cell, and anything free at least one empty one.
#[must_use]
pub fn bar_cells(fraction: f64, cells: usize) -> (usize, usize) {
    let fraction = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a cell count, clamped to 0..=cells"
    )]
    let mut filled = ((fraction * cells as f64).round() as usize).min(cells);
    if cells >= 2 {
        if fraction > 0.0 && filled == 0 {
            filled = 1;
        } else if fraction < 1.0 && filled == cells {
            filled = cells - 1;
        }
    }
    (filled, cells - filled)
}

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

/// One line of the panel overview: `2026-09-19 08:00  D   pre-nvidia`, the comment cut to
/// `comment_chars`.
#[must_use]
pub fn overview_row(snapshot: &Snapshot, comment_chars: usize) -> String {
    let row = format!(
        "{}  {:<3} {}",
        when(snapshot.created),
        tag_letters(&snapshot.tags),
        truncate(&quoted_comment(snapshot), comment_chars),
    );
    row.trim_end().to_owned()
}

/// How many snapshots the overview leaves out of its `shown` rows, if any.
#[must_use]
pub fn older_count(total: usize, shown: usize) -> Option<usize> {
    total.checked_sub(shown).filter(|&more| more > 0)
}

/// `sdX1` for `/dev/sdX1`: the device's name without the folder.
#[must_use]
pub fn device_name(device: &str) -> &str {
    device.rsplit('/').next().unwrap_or(device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn labels_are_the_short_date_and_the_quoted_comment() {
        let mut snapshot = Snapshot {
            name: "2026-09-27_09-12-33".to_owned(),
            created: date(2026, 9, 27).at(9, 12, 33, 0),
            tags: vec![Tag::OnDemand],
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
    fn disk_text_says_only_what_is_known() {
        let usage = DiskUsage {
            total: 1_000_203_837_440,
            used: 481_036_337_152,
            free: 518_617_202_688,
        };
        assert_eq!(
            disk_text(Some(usage), 9).as_deref(),
            Some("448G used · 483G free · 9 snapshots")
        );
        assert!(disk_text(Some(usage), 1).unwrap().ends_with("1 snapshot"));
        assert_eq!(disk_text(None, 3), None);
    }

    #[test]
    fn bar_cells_add_up_and_never_hide_a_little_use_or_space() {
        assert_eq!(bar_cells(0.0, 10), (0, 10));
        assert_eq!(bar_cells(1.0, 10), (10, 0));
        assert_eq!(bar_cells(0.58, 10), (6, 4));
        assert_eq!(bar_cells(0.001, 10), (1, 9));
        assert_eq!(bar_cells(0.999, 10), (9, 1));
        assert_eq!(bar_cells(0.5, 0), (0, 0));
        assert_eq!(bar_cells(0.7, 1), (1, 0));
        assert_eq!(bar_cells(f64::NAN, 4), (0, 4));
        assert_eq!(bar_cells(7.0, 4), (4, 0));
        for cells in 0..50 {
            let (filled, empty) = bar_cells(0.37, cells);
            assert_eq!(filled + empty, cells);
        }
    }

    #[test]
    fn percents_round_down() {
        assert_eq!(percent(58.23), "58%");
        assert_eq!(percent(99.91), "99%");
        assert_eq!(percent(100.0), "100%");
        assert_eq!(percent(-3.0), "0%");
    }

    #[test]
    fn overview_rows_are_date_tags_and_a_short_comment() {
        let mut snapshot = Snapshot {
            name: "2026-09-19_08-00-00".to_owned(),
            created: date(2026, 9, 19).at(8, 0, 0, 0),
            tags: vec![Tag::Daily],
            comment: None,
        };
        assert_eq!(overview_row(&snapshot, 14), "2026-09-19 08:00  D");
        snapshot.comment = Some("pre-nvidia".to_owned());
        assert_eq!(
            overview_row(&snapshot, 14),
            "2026-09-19 08:00  D   \"pre-nvidia\""
        );
        snapshot.comment = Some("before the big kernel update".to_owned());
        assert!(overview_row(&snapshot, 14).ends_with('…'));
        snapshot.tags = vec![Tag::OnDemand, Tag::Boot];
        assert!(overview_row(&snapshot, 14).starts_with("2026-09-19 08:00  OB  "));
    }

    #[test]
    fn older_snapshots_are_counted_only_when_there_are_some() {
        assert_eq!(older_count(9, 5), Some(4));
        assert_eq!(older_count(6, 5), Some(1));
        assert_eq!(older_count(5, 5), None);
        assert_eq!(older_count(0, 5), None);
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
    fn ago_buckets() {
        let now = date(2026, 9, 25).at(12, 0, 0, 0);
        let cases = [
            (date(2026, 9, 25).at(12, 0, 30, 0), "just now"), // future
            (date(2026, 9, 25).at(11, 59, 30, 0), "just now"),
            (date(2026, 9, 25).at(11, 59, 0, 0), "1m ago"),
            (date(2026, 9, 25).at(11, 0, 1, 0), "59m ago"),
            (date(2026, 9, 25).at(9, 0, 0, 0), "3h ago"),
            (date(2026, 9, 23).at(12, 0, 1, 0), "47h ago"),
            (date(2026, 9, 23).at(12, 0, 0, 0), "2d ago"),
            (date(2025, 9, 25).at(12, 0, 0, 0), "365d ago"),
        ];
        for (then, want) in cases {
            assert_eq!(ago(then, now), want, "{then}");
        }
    }

    #[test]
    fn tags_format() {
        assert_eq!(tag_letters(&[Tag::Boot, Tag::Daily]), "BD");
        assert_eq!(tag_names(&[Tag::OnDemand]), "O on-demand");
        assert_eq!(tag_names(&[]), "-");
    }

    #[test]
    fn truncate_counts_characters() {
        assert_eq!(truncate("short", 5), "short");
        assert_eq!(truncate("longer", 5), "long…");
        assert_eq!(truncate("äöüäöü", 4), "äöü…");
        assert_eq!(truncate("", 0), "");
    }

    #[test]
    fn summary_counts() {
        assert_eq!(summary(Some(Mode::Rsync), 3), "rsync · 3 snapshots");
        assert_eq!(summary(Some(Mode::Btrfs), 1), "btrfs · 1 snapshot");
        assert_eq!(summary(None, 0), "0 snapshots");
    }

    #[test]
    fn short_uuid_keeps_four_characters() {
        assert_eq!(short_uuid("1a2b1234-0000-0000-0000-000000000000"), "1a2b…");
        assert_eq!(short_uuid("1a2b"), "1a2b");
    }
}
