// SPDX-License-Identifier: GPL-3.0-only

use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};

/// Parses the output of `timeshift --list`.
///
/// See `docs/TIMESHIFT-CLI.md` for the format. Lines are matched by content, not by column
/// position, and `GLib` warnings are skipped wherever they appear.
///
/// Timeshift's own `E:`/`W:` lines, anywhere, and table lines that aren't snapshot rows don't
/// fail the list: they're collected in [`SnapshotList::warnings`]. Whether Timeshift failed is
/// decided by its exit code, not here.
///
/// # Errors
///
/// [`Error::UnrecognisedOutput`] when the output has neither a snapshot table nor
/// "No snapshots found".
pub fn parse_list(output: &str) -> Result<SnapshotList> {
    let mut list = SnapshotList::default();
    let mut recognised = false;
    let mut in_table = false;

    for (index, raw) in output.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || is_glib_message(line) {
            continue;
        }
        if is_diagnostic(line) {
            list.warnings.push(line.to_owned());
            continue;
        }

        if in_table {
            if line.chars().all(|c| c == '-') {
                continue;
            }
            match parse_row(line) {
                Some(snapshot) => list.snapshots.push(snapshot),
                None => list
                    .warnings
                    .push(format!("line {}: not a snapshot row: {line}", index + 1)),
            }
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

/// `E: <message>` or `W: <message>`: Timeshift's own errors and warnings. It prints them on
/// stdout, mixed with its normal output.
pub(crate) fn is_diagnostic(line: &str) -> bool {
    line.starts_with("E: ") || line.starts_with("W: ")
}

/// What Timeshift said went wrong, from a failed run: its `E:`/`W:` lines from stdout, then
/// stderr's lines (without `GLib` noise). If neither has anything, stdout's lines, so there's
/// always something to show.
pub(crate) fn failure_output(stdout: &str, stderr: &str) -> Vec<String> {
    let meaningful = |line: &&str| !line.is_empty() && !is_glib_message(line);
    let mut lines: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| is_diagnostic(line))
        .chain(stderr.lines().map(str::trim).filter(meaningful))
        .collect();
    if lines.is_empty() {
        lines = stdout.lines().map(str::trim).filter(meaningful).collect();
    }
    lines.into_iter().map(str::to_owned).collect()
}

/// The device in `E: Device not found: '<device>'`.
pub(crate) fn device_not_found(line: &str) -> Option<String> {
    let rest = line.split_once("Device not found")?.1;
    let rest = rest.trim_start_matches(':').trim();
    let device = rest
        .strip_prefix('\'')
        .and_then(|quoted| quoted.split_once('\''))
        .map_or(rest, |(device, _)| device);
    Some(device.to_owned())
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
