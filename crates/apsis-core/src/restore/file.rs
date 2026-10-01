// SPDX-License-Identifier: GPL-3.0-only

//! What the restore's plan and state files share (PLAN 6b.9): where they are, their version,
//! how they're written and how they're read back.
//!
//! Each file is one JSON object with `"version"` first. A file is taken whole or not at all:
//! another version, a missing or unknown field, or a value of the wrong kind or range refuses
//! it. Nothing is written that wouldn't be read back.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use serde_json::{Map, Value};

use super::filter::Home;
use crate::model::parse_snapshot_name;
use crate::usage::is_plain_uuid;

/// The folder the files are in: root's, 0700, made by the helper, protected from the restore
/// itself ([`super::filter::PROTECTED`]).
pub const DIR: &str = "/var/lib/apsis/restore";

/// The version this Apsis writes, and the only one it reads.
pub const VERSION: i64 = 1;

/// No file here comes near this; a larger one isn't ours and isn't read.
const MAX_BYTES: usize = 64 * 1024;

/// Root only: the plan names disks and the user who started it.
const MODE: u32 = 0o600;

/// Why a file couldn't be read or written.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// It isn't there, or can't be opened, read or written.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// It's there but not one this Apsis trusts. The text says why.
    #[error("{0}")]
    Invalid(String),
}

impl FileError {
    fn invalid(reason: impl Into<String>) -> Self {
        Self::Invalid(reason.into())
    }

    /// The same error with `prefix: ` in front of an invalid file's reason.
    pub(super) fn within(self, prefix: &str) -> Self {
        match self {
            Self::Invalid(reason) => Self::Invalid(format!("{prefix}: {reason}")),
            io @ Self::Io(_) => io,
        }
    }
}

/// Replaces `dir/name` with `text` in one step: a temporary file in the same folder, flushed,
/// renamed over it, then the folder flushed so the rename survives a power cut. A reader sees
/// the old file or the new one, never a part.
pub(super) fn save(dir: &Path, name: &str, text: &str) -> Result<(), FileError> {
    let path = dir.join(name);
    let temp = dir.join(format!("{name}.apsis-tmp"));
    // Left over from a crash: never ours to keep.
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let written = (|| {
        let mut file = create_new(&temp)?;
        // `mode` is masked by the umask on create; set it exactly.
        file.set_permissions(fs::Permissions::from_mode(MODE))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        File::open(dir)?.sync_all()
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    Ok(written?)
}

/// The text of `dir/name`, if it's a regular file of a plausible size.
pub(super) fn load(dir: &Path, name: &str) -> Result<String, FileError> {
    let not_regular = || FileError::invalid("not a regular file").within(name);
    let file = match open_nofollow(&dir.join(name)) {
        Err(error) if error.raw_os_error() == Some(Errno::LOOP.raw_os_error()) => {
            return Err(not_regular());
        }
        other => other?,
    };
    // Asked of the open file, not of the name.
    if !file.metadata()?.is_file() {
        return Err(not_regular());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(FileError::invalid(format!("larger than {MAX_BYTES} bytes")).within(name));
    }
    String::from_utf8(bytes).map_err(|_| FileError::invalid("not UTF-8 text").within(name))
}

/// A new file at `path` for writing: `O_CREAT | O_EXCL`, so a name that's taken, also by a
/// link, is an error and nothing is written through it.
pub(super) fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(MODE)
        .open(path)
}

/// `path` for reading, with `O_NOFOLLOW`: a link at the name fails the open itself (`ELOOP`),
/// so there's no check before it to race. `O_NONBLOCK` keeps a FIFO from holding it up.
pub(super) fn open_nofollow(path: &Path) -> io::Result<File> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    Ok(File::from(rustix::fs::open(path, flags, Mode::empty())?))
}

/// A file's text: the version, then `fields` in order, as indented JSON with a final newline.
pub(super) fn to_text(fields: impl IntoIterator<Item = (&'static str, Value)>) -> String {
    let mut map = Map::new();
    map.insert("version".to_owned(), VERSION.into());
    for (key, value) in fields {
        map.insert(key.to_owned(), value);
    }
    format!("{:#}\n", Value::Object(map))
}

/// A file's fields: a JSON object of this version with exactly `keys` besides the version.
pub(super) fn object(text: &str, keys: &[&str]) -> Result<Map<String, Value>, FileError> {
    let Ok(Value::Object(mut map)) = serde_json::from_str(text) else {
        return Err(FileError::invalid("not a JSON object"));
    };
    // First: another version's fields aren't this one's to judge.
    let version = map.remove("version");
    if version.as_ref().and_then(Value::as_i64) != Some(VERSION) {
        return Err(FileError::invalid(format!(
            "version {} (this Apsis reads {VERSION})",
            version.map_or_else(|| "missing".to_owned(), |v| v.to_string())
        )));
    }
    fields(&map, keys)?;
    Ok(map)
}

/// `map` has every one of `keys` and nothing else.
pub(super) fn fields(map: &Map<String, Value>, keys: &[&str]) -> Result<(), FileError> {
    if let Some(key) = keys.iter().find(|key| !map.contains_key(**key)) {
        return Err(FileError::invalid(format!("no {key:?}")));
    }
    if let Some(key) = map.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(FileError::invalid(format!("unknown field {key:?}")));
    }
    Ok(())
}

pub(super) fn text<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str, FileError> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| FileError::invalid(format!("{key:?} isn't text")))
}

/// Text, or `null` for none.
pub(super) fn optional_text<'a>(
    map: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, FileError> {
    match map.get(key) {
        Some(Value::Null) => Ok(None),
        _ => text(map, key).map(Some),
    }
}

pub(super) fn flag(map: &Map<String, Value>, key: &str) -> Result<bool, FileError> {
    map.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| FileError::invalid(format!("{key:?} isn't true or false")))
}

/// A size in bytes.
pub(super) fn bytes(map: &Map<String, Value>, key: &str) -> Result<u64, FileError> {
    map.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| FileError::invalid(format!("{key:?} isn't a whole number of bytes")))
}

/// Unix seconds.
pub(super) fn time(map: &Map<String, Value>, key: &str) -> Result<i64, FileError> {
    let when = map.get(key).and_then(Value::as_i64).unwrap_or(0);
    check_time(key, when)?;
    Ok(when)
}

pub(super) fn home(map: &Map<String, Value>) -> Result<Home, FileError> {
    match text(map, "home")? {
        "keep" => Ok(Home::Keep),
        "restore" => Ok(Home::Restore),
        _ => Err(FileError::invalid(
            "\"home\" is neither \"keep\" nor \"restore\"",
        )),
    }
}

pub(super) fn home_word(home: Home) -> &'static str {
    match home {
        Home::Keep => "keep",
        Home::Restore => "restore",
    }
}

/// `name` is a snapshot's folder name, so it can't be a path or an option.
pub(super) fn check_snapshot(key: &str, name: &str) -> Result<(), FileError> {
    if parse_snapshot_name(name).is_none() {
        return Err(FileError::invalid(format!("{key:?} isn't a snapshot name")));
    }
    Ok(())
}

/// `uuid` is safe in `/dev/disk/by-uuid/<uuid>`.
pub(super) fn check_uuid(key: &str, uuid: &str) -> Result<(), FileError> {
    if !is_plain_uuid(uuid) {
        return Err(FileError::invalid(format!("{key:?} isn't a UUID")));
    }
    Ok(())
}

/// `when` is after 1970: a clock that was never set gives nothing to compare.
pub(super) fn check_time(key: &str, when: i64) -> Result<(), FileError> {
    if when <= 0 {
        return Err(FileError::invalid(format!("{key:?} isn't a time")));
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::super::filter::{self, PROTECTED};
    use super::*;

    /// A fresh, empty folder for one test.
    pub(in crate::restore) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apsis-restore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn invalid<T: std::fmt::Debug>(result: Result<T, FileError>) -> String {
        match result {
            Err(FileError::Invalid(reason)) => reason,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn a_saved_file_reads_back_as_written() {
        let dir = temp_dir("roundtrip");
        save(&dir, "a.json", "{\n  \"version\": 1\n}\n").unwrap();
        assert_eq!(load(&dir, "a.json").unwrap(), "{\n  \"version\": 1\n}\n");
    }

    #[test]
    fn a_saved_file_is_for_its_owner_only_and_leaves_no_temporary_file() {
        let dir = temp_dir("mode");
        save(&dir, "a.json", "{}").unwrap();
        let mode = fs::metadata(dir.join("a.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o600);
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.json"]);
    }

    #[test]
    fn saving_replaces_the_whole_file() {
        let dir = temp_dir("replace");
        save(&dir, "a.json", "a much longer first text").unwrap();
        save(&dir, "a.json", "short").unwrap();
        assert_eq!(load(&dir, "a.json").unwrap(), "short");
    }

    /// A crash between the write and the rename leaves the temporary file behind.
    #[test]
    fn a_temporary_file_left_by_a_crash_is_replaced() {
        let dir = temp_dir("leftover");
        fs::write(dir.join("a.json.apsis-tmp"), "half a fi").unwrap();
        save(&dir, "a.json", "whole").unwrap();
        assert_eq!(load(&dir, "a.json").unwrap(), "whole");
        assert!(!dir.join("a.json.apsis-tmp").exists());
    }

    /// The folder is the helper's to make (root, 0700).
    #[test]
    fn saving_into_a_missing_folder_fails_and_makes_nothing() {
        let dir = temp_dir("missing").join("not-there");
        assert!(matches!(save(&dir, "a.json", "{}"), Err(FileError::Io(_))));
        assert!(!dir.exists());
    }

    #[test]
    fn a_missing_file_is_an_io_error_not_an_invalid_file() {
        let dir = temp_dir("absent");
        match load(&dir, "a.json") {
            Err(FileError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_file_larger_than_the_limit_is_refused_unread() {
        let dir = temp_dir("large");
        fs::write(dir.join("a.json"), vec![b' '; MAX_BYTES + 1]).unwrap();
        assert_eq!(
            invalid(load(&dir, "a.json")),
            "a.json: larger than 65536 bytes"
        );
        fs::write(dir.join("a.json"), vec![b' '; MAX_BYTES]).unwrap();
        assert_eq!(load(&dir, "a.json").unwrap().len(), MAX_BYTES);
    }

    #[test]
    fn a_file_that_isnt_text_is_refused() {
        let dir = temp_dir("binary");
        fs::write(dir.join("a.json"), [0xff, 0xfe]).unwrap();
        assert_eq!(invalid(load(&dir, "a.json")), "a.json: not UTF-8 text");
    }

    #[test]
    fn a_link_in_place_of_the_file_is_refused() {
        let dir = temp_dir("link");
        fs::write(dir.join("real.json"), "{}").unwrap();
        std::os::unix::fs::symlink(dir.join("real.json"), dir.join("a.json")).unwrap();
        assert_eq!(invalid(load(&dir, "a.json")), "a.json: not a regular file");
    }

    /// The link itself is refused, whatever it points to: nothing, or a folder.
    #[test]
    fn a_dangling_link_or_a_link_to_a_folder_is_refused_not_followed() {
        let dir = temp_dir("link-dangling");
        std::os::unix::fs::symlink(dir.join("gone.json"), dir.join("a.json")).unwrap();
        assert_eq!(invalid(load(&dir, "a.json")), "a.json: not a regular file");
        std::os::unix::fs::symlink(&dir, dir.join("b.json")).unwrap();
        assert_eq!(invalid(load(&dir, "b.json")), "b.json: not a regular file");
    }

    #[test]
    fn a_folder_in_place_of_the_file_is_refused() {
        let dir = temp_dir("folder");
        fs::create_dir(dir.join("a.json")).unwrap();
        assert_eq!(invalid(load(&dir, "a.json")), "a.json: not a regular file");
    }

    /// The temporary file is made new (`O_EXCL`), so nothing is written through a link put
    /// at its name.
    #[test]
    fn a_link_planted_as_the_temporary_file_isnt_written_through() {
        let dir = temp_dir("planted");
        fs::write(dir.join("victim"), "untouched").unwrap();
        std::os::unix::fs::symlink(dir.join("victim"), dir.join("a.json.apsis-tmp")).unwrap();
        save(&dir, "a.json", "whole").unwrap();
        assert_eq!(fs::read_to_string(dir.join("victim")).unwrap(), "untouched");
        assert_eq!(load(&dir, "a.json").unwrap(), "whole");
        assert!(fs::symlink_metadata(dir.join("a.json")).unwrap().is_file());
    }

    /// The temporary file is opened with `O_EXCL`: a name that's taken is an error, never a
    /// file written into.
    #[test]
    fn the_temporary_file_is_only_ever_made_new() {
        let dir = temp_dir("excl");
        fs::write(dir.join("taken"), "untouched").unwrap();
        let error = create_new(&dir.join("taken")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("link")).unwrap();
        let error = create_new(&dir.join("link")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!dir.join("nowhere").exists());
        assert_eq!(fs::read_to_string(dir.join("taken")).unwrap(), "untouched");
    }

    /// `O_NOFOLLOW`: the open itself refuses a link, with no check before it to race.
    #[test]
    fn the_open_for_reading_doesnt_follow_a_link() {
        let dir = temp_dir("nofollow");
        fs::write(dir.join("real.json"), "{}").unwrap();
        std::os::unix::fs::symlink(dir.join("real.json"), dir.join("a.json")).unwrap();
        assert!(open_nofollow(&dir.join("a.json")).is_err());
        assert!(open_nofollow(&dir.join("real.json")).is_ok());
    }

    /// PLAN 6b.2: the folder with `request.json`, `state.json` and `result.json` is under a
    /// protected path.
    #[test]
    fn the_state_folder_is_on_the_protect_list() {
        assert!(
            PROTECTED.iter().any(|pattern| pattern
                .strip_suffix("/***")
                .is_some_and(|folder| Path::new(DIR).starts_with(folder))),
            "{PROTECTED:?}"
        );
    }

    /// The restore's own rsync, with its own filter, over a fake live root: `--delete`
    /// removes what the snapshot doesn't have, but not the state folder or its files.
    #[test]
    fn rsync_delete_never_removes_the_state_folder() {
        let lab = temp_dir("protected");
        let snapshot = lab.join("localhost");
        let live = lab.join("live");
        let state = live.join(DIR.trim_start_matches('/'));
        fs::create_dir_all(snapshot.join("etc")).unwrap();
        fs::write(snapshot.join("etc/hostname"), "restored").unwrap();
        fs::create_dir_all(&state).unwrap();
        for name in ["request.json", "state.json", "result.json"] {
            fs::write(state.join(name), name).unwrap();
        }
        fs::create_dir_all(live.join("var/lib/newer")).unwrap();
        fs::write(live.join("var/lib/newer/file"), "after the snapshot").unwrap();

        let rules = filter::rules(&filter::Request {
            mountinfo: "",
            home: Home::Keep,
            protected_kernel: None,
            snapshot_excludes: "",
        })
        .unwrap();
        fs::write(lab.join("restore.filter"), filter::to_text(&rules)).unwrap();
        let argv = super::super::argv::rsync(
            &snapshot,
            &live,
            &lab.join("restore.filter"),
            &lab.join("rsync-log"),
            true,
        );
        let output = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");

        // `--delete` ran.
        assert!(!live.join("var/lib/newer").exists());
        assert_eq!(
            fs::read_to_string(live.join("etc/hostname")).unwrap(),
            "restored"
        );
        for name in ["request.json", "state.json", "result.json"] {
            assert_eq!(fs::read_to_string(state.join(name)).unwrap(), name);
        }
    }

    #[test]
    fn an_object_with_this_version_and_exactly_its_fields_is_read() {
        let map = object(r#"{"version": 1, "a": true, "b": null}"#, &["a", "b"]).unwrap();
        assert_eq!(map.get("a"), Some(&Value::Bool(true)));
    }

    #[test]
    fn text_that_isnt_a_json_object_is_refused() {
        for text in ["", "not json", "[1]", "1", "null", r#"{"version": 1"#] {
            assert_eq!(invalid(object(text, &[])), "not a JSON object", "{text}");
        }
    }

    /// A file from another Apsis is refused whole, whether older, newer or unmarked.
    #[test]
    fn a_file_of_another_version_is_refused() {
        for (text, reason) in [
            (r#"{"version": 2}"#, "version 2 (this Apsis reads 1)"),
            (r#"{"version": 0}"#, "version 0 (this Apsis reads 1)"),
            (r#"{}"#, "version missing (this Apsis reads 1)"),
            (r#"{"version": "1"}"#, "version \"1\" (this Apsis reads 1)"),
            (r#"{"version": 1.0}"#, "version 1.0 (this Apsis reads 1)"),
            (r#"{"version": null}"#, "version null (this Apsis reads 1)"),
        ] {
            assert_eq!(invalid(object(text, &[])), reason, "{text}");
        }
    }

    /// The version is looked at first: another version's fields aren't this one's to judge.
    #[test]
    fn the_version_is_checked_before_the_fields() {
        assert_eq!(
            invalid(object(r#"{"version": 2, "new": 1}"#, &["a"])),
            "version 2 (this Apsis reads 1)"
        );
    }

    #[test]
    fn a_missing_or_unknown_field_is_refused() {
        assert_eq!(
            invalid(object(r#"{"version": 1, "a": 1}"#, &["a", "b"])),
            "no \"b\""
        );
        assert_eq!(
            invalid(object(r#"{"version": 1, "a": 1, "c": 2}"#, &["a"])),
            "unknown field \"c\""
        );
    }

    #[test]
    fn text_is_json_with_the_version_first_and_a_final_newline() {
        let text = to_text([("b", Value::from(2)), ("a", Value::Null)]);
        assert_eq!(
            text,
            "{\n  \"version\": 1,\n  \"b\": 2,\n  \"a\": null\n}\n"
        );
        assert!(object(&text, &["a", "b"]).is_ok());
    }
}
