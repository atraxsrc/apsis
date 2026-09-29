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

/// A device size the way lsblk prints it: `931.5G`, `512M`, binary units.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    #[allow(clippy::cast_precision_loss, reason = "one decimal shown")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text}{}", UNITS[unit])
}

/// [`size`] without the decimal from 10 up: `448G`, `4.5G`. For the disk line, where the
/// numbers sit side by side.
#[must_use]
pub fn size_short(bytes: u64) -> String {
    let text = size(bytes);
    let split = text
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    match number.parse::<f64>() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a displayed size under 1024"
        )]
        Ok(value) if value >= 10.0 => format!("{}{unit}", value.round() as u64),
        _ => text,
    }
}

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

/// Time left, roughly: `<1 min`, `~3 min`, `~2 h`, `~2 h 5 min`.
#[must_use]
pub fn eta(seconds: u64) -> String {
    if seconds < 60 {
        return "<1 min".to_owned();
    }
    // To the nearest minute.
    let minutes = (seconds + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("~{m} min"),
        (h, 0) => format!("~{h} h"),
        (h, m) => format!("~{h} h {m} min"),
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
    fn eta_is_rough() {
        assert_eq!(eta(0), "<1 min");
        assert_eq!(eta(59), "<1 min");
        assert_eq!(eta(60), "~1 min");
        assert_eq!(eta(192), "~3 min");
        assert_eq!(eta(3569), "~59 min");
        assert_eq!(eta(3590), "~1 h");
        assert_eq!(eta(2 * 3600 + 5 * 60), "~2 h 5 min");
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
