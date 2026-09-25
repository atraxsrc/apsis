// SPDX-License-Identifier: GPL-3.0-only

//! Plain-text formatting for the popup and tooltip. No widgets, so it's unit-testable.

use apsis_core::{Mode, Snapshot, Tag};
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

/// The last `n` non-blank lines of `text`, trimmed on the right.
#[must_use]
pub fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| (*l).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

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
    fn tail_keeps_last_non_blank_lines() {
        assert_eq!(tail("a\n\nb  \nc\n\n", 2), ["b", "c"]);
        assert_eq!(tail("a\n", 5), ["a"]);
        assert!(tail("", 3).is_empty());
    }
}
