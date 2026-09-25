// SPDX-License-Identifier: GPL-3.0-only

//! A snapshot's `info.json`, as Timeshift writes and reads it.
//!
//! Written by `Snapshot.write_control_file` and then rewritten by `update_control_file` once
//! `set_tags` has added the tag (linuxmint/timeshift 24.01.1, `Snapshot.vala:394-434` and
//! `:339-386`, `Main.vala:1597-1604` and `:1699-1728`). The end result for an on-demand rsync
//! snapshot is one object with nine string members, in this order, pretty-printed by json-glib
//! (2-space indent, `"key" : value`, no final newline):
//!
//! ```text
//! created, sys-uuid, sys-distro, app-version, file_count, tags, comments, live, type
//! ```
//!
//! Read by `Snapshot.read_control_file` (`Snapshot.vala:190-289`): each member is read with
//! `get_string_member`, a missing one gets a default.

use serde_json::{Map, Value};

use crate::model::Tag;
use crate::settings::write_object;

/// The control file in each snapshot folder.
pub const INFO_FILE: &str = "info.json";

/// What `info.json` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Unix time in seconds (UTC). Timeshift: `dt_created.to_utc().to_unix()`.
    pub created: i64,
    /// Filesystem UUID of the system's `/`. Timeshift only links against, and schedules by,
    /// snapshots whose `sys-uuid` is the running system's (`SnapshotRepo.vala:390-402`).
    pub sys_uuid: String,
    /// `LinuxDistro.full_name()`, e.g. `Pop 24.04 (noble)`.
    pub sys_distro: String,
    /// Timeshift writes its version. Only stored, never used (`Snapshot.vala:226`).
    pub app_version: String,
    /// Lines in `rsync-log` (`Main.vala:1597`).
    pub file_count: u64,
    /// Timeshift's tag words, in order (see [`Tag::word`]).
    pub tags: Vec<String>,
    /// The description; empty for none.
    pub comments: String,
    /// Always `false` for a snapshot taken of the running system.
    pub live: bool,
    /// `rsync` or `btrfs`.
    pub kind: String,
}

impl Info {
    /// The tags Apsis knows. Unknown words are dropped (they're kept in [`Info::tags`]).
    #[must_use]
    pub fn known_tags(&self) -> Vec<Tag> {
        self.tags.iter().filter_map(|w| Tag::from_word(w)).collect()
    }

    /// The file, byte for byte as Timeshift writes it.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut fields = Map::new();
        let mut put = |key: &str, value: String| {
            fields.insert(key.to_owned(), Value::String(value));
        };
        put("created", self.created.to_string());
        put("sys-uuid", self.sys_uuid.clone());
        put("sys-distro", self.sys_distro.clone());
        put("app-version", self.app_version.clone());
        put("file_count", self.file_count.to_string());
        // `Snapshot.taglist`: the words joined by single spaces.
        put("tags", self.tags.join(" "));
        put("comments", self.comments.clone());
        // Vala's `bool.to_string()`.
        put("live", if self.live { "true" } else { "false" }.to_owned());
        put("type", self.kind.clone());
        let mut out = String::new();
        write_object(&mut out, &fields, 0);
        out
    }

    /// Reads `info.json` the way Timeshift does. `None` where Timeshift would mark the
    /// snapshot invalid (not JSON, not an object) or trip over it (a member that isn't a
    /// string).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) else {
            return None;
        };
        // `json_get_string`: the member if it's there, else the default.
        let string = |key: &str| -> Option<Option<&str>> {
            match fields.get(key) {
                None => Some(None),
                Some(Value::String(s)) => Some(Some(s.as_str())),
                Some(_) => None,
            }
        };
        let created = string("created")?.map_or(0, vala_int64);
        let tags = string("tags")?.unwrap_or_default();
        Some(Self {
            created,
            sys_uuid: string("sys-uuid")?.unwrap_or_default().to_owned(),
            sys_distro: string("sys-distro")?.unwrap_or_default().to_owned(),
            app_version: string("app-version")?.unwrap_or_default().to_owned(),
            file_count: string("file_count")?.map_or(0, |s| vala_int64(s).max(0).unsigned_abs()),
            tags: taglist(tags),
            comments: string("comments")?.unwrap_or_default().to_owned(),
            live: string("live")? == Some("true"),
            kind: string("type")?.unwrap_or("rsync").to_owned(),
        })
    }
}

/// `Snapshot.taglist`'s setter: split on single spaces, strip, keep the first of each. Empty
/// words (from `""` or double spaces) are left out; Timeshift keeps one `""`, which matches no
/// tag.
fn taglist(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for word in text.split(' ').map(vala_strip) {
        if !word.is_empty() && !tags.iter().any(|t| t == word) {
            tags.push(word.to_owned());
        }
    }
    tags
}

/// Vala's `int64.parse` (`g_ascii_strtoll`, base 10): leading whitespace and a sign, then as
/// many digits as there are; 0 if none.
pub(crate) fn vala_int64(text: &str) -> i64 {
    let text = text.trim_start_matches(is_vala_space);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    let value: i64 = digits[..end].parse().unwrap_or(0);
    if negative { -value } else { value }
}

/// `g_ascii_isspace`, which Vala's `strip()` uses.
pub(crate) fn is_vala_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// Vala's `string.strip()`.
pub(crate) fn vala_strip(text: &str) -> &str {
    text.trim_matches(is_vala_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Info {
        Info {
            created: 1_790_000_000,
            sys_uuid: "11111111-2222-3333-4444-555555555555".to_owned(),
            sys_distro: "Pop 24.04 (noble)".to_owned(),
            app_version: "apsis 0.1.0".to_owned(),
            file_count: 1234,
            tags: vec!["ondemand".to_owned()],
            comments: "before \"update\" / ümlaut".to_owned(),
            live: false,
            kind: "rsync".to_owned(),
        }
    }

    #[test]
    fn written_like_json_glib() {
        let expected = "{\n  \"created\" : \"1790000000\",\n  \"sys-uuid\" : \
            \"11111111-2222-3333-4444-555555555555\",\n  \"sys-distro\" : \"Pop 24.04 (noble)\",\n  \
            \"app-version\" : \"apsis 0.1.0\",\n  \"file_count\" : \"1234\",\n  \"tags\" : \
            \"ondemand\",\n  \"comments\" : \"before \\\"update\\\" / ümlaut\",\n  \"live\" : \
            \"false\",\n  \"type\" : \"rsync\"\n}";
        assert_eq!(info().to_text(), expected);
    }

    #[test]
    fn reads_back_what_it_writes() {
        assert_eq!(Info::parse(&info().to_text()), Some(info()));
    }

    #[test]
    fn missing_members_get_timeshifts_defaults() {
        let parsed = Info::parse("{}").unwrap();
        assert_eq!(parsed.created, 0);
        assert_eq!(parsed.kind, "rsync");
        assert!(parsed.tags.is_empty());
        assert!(!parsed.live);
    }

    #[test]
    fn broken_files_are_invalid() {
        for text in [
            "",
            "[]",
            "{",
            "{\"created\" : 5}",
            "{\"tags\" : [\"ondemand\"]}",
        ] {
            assert_eq!(Info::parse(text), None, "{text}");
        }
    }

    #[test]
    fn tags_split_like_timeshift() {
        let parsed = Info::parse(r#"{"tags" : "ondemand  daily ondemand future"}"#).unwrap();
        assert_eq!(parsed.tags, ["ondemand", "daily", "future"]);
        assert_eq!(parsed.known_tags(), [Tag::OnDemand, Tag::Daily]);
    }

    #[test]
    fn numbers_parse_like_vala() {
        assert_eq!(vala_int64(" 42abc"), 42);
        assert_eq!(vala_int64("-7"), -7);
        assert_eq!(vala_int64("x"), 0);
        assert_eq!(vala_int64(""), 0);
    }
}
