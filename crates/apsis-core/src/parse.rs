// SPDX-License-Identifier: GPL-3.0-only

use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};

/// Parses the output of `timeshift --list`.
///
/// See `docs/TIMESHIFT-CLI.md` for the format. Lines are matched by content, not by column
/// position, and `GLib` warnings are skipped wherever they appear.
///
/// # Errors
///
/// [`Error::UnrecognisedOutput`] when the output has neither a snapshot table nor
/// "No snapshots found", and [`Error::BadRow`] when a line in the table isn't a snapshot row.
pub fn parse_list(output: &str) -> Result<SnapshotList> {
    let mut list = SnapshotList::default();
    let mut recognised = false;
    let mut in_table = false;

    for (index, raw) in output.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || is_glib_message(line) {
            continue;
        }

        if in_table {
            if line.chars().all(|c| c == '-') {
                continue;
            }
            let snapshot = parse_row(line).ok_or_else(|| Error::BadRow {
                line: index + 1,
                text: raw.to_owned(),
            })?;
            list.snapshots.push(snapshot);
        } else if line == "No snapshots found" {
            recognised = true;
        } else if is_table_header(line) {
            recognised = true;
            in_table = true;
        } else if let Some((key, value)) = line.split_once(':') {
            let value = value.trim();
            match key.trim() {
                "Device" if value != "Not Selected" => list.device = non_empty(value),
                "UUID" => list.uuid = non_empty(value),
                "Mode" => list.mode = parse_mode(value),
                _ => {}
            }
        }
    }

    if recognised {
        Ok(list)
    } else {
        Err(Error::UnrecognisedOutput)
    }
}

/// `** (process:N): CRITICAL **: ...` or `(timeshift:N): GLib-WARNING **: ...`
fn is_glib_message(line: &str) -> bool {
    (line.starts_with("** (") || line.starts_with('(')) && line.contains(" **: ")
}

/// `Num     Name                 Tags  Description`
fn is_table_header(line: &str) -> bool {
    line.split_whitespace().take(3).eq(["Num", "Name", "Tags"])
}

/// `<num> > <name> [<tags>] [<description>]`
fn parse_row(line: &str) -> Option<Snapshot> {
    let (num, rest) = next_token(line)?;
    if !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (mut name, mut rest) = next_token(rest)?;
    if name == ">" {
        (name, rest) = next_token(rest)?;
    }
    let created = parse_snapshot_name(name)?;

    // A token made only of tag letters is the Tags column; otherwise the row has no tags.
    let tags_and_rest =
        next_token(rest).and_then(|(token, after)| Some((parse_tags(token)?, after)));
    let (tags, description) = tags_and_rest.unwrap_or((Vec::new(), rest));

    Some(Snapshot {
        name: name.to_owned(),
        created,
        tags,
        comment: non_empty(description.trim()),
    })
}

fn parse_tags(token: &str) -> Option<Vec<Tag>> {
    token.chars().map(Tag::from_char).collect()
}

fn parse_mode(value: &str) -> Option<Mode> {
    if value.eq_ignore_ascii_case("rsync") {
        Some(Mode::Rsync)
    } else if value.eq_ignore_ascii_case("btrfs") {
        Some(Mode::Btrfs)
    } else {
        None
    }
}

/// Splits off the first whitespace-separated token; the rest keeps its inner spacing.
fn next_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    Some(s.split_at(end))
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}
