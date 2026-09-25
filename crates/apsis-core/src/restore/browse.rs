// SPDX-License-Identifier: GPL-3.0-only

//! One folder of a snapshot, each entry compared with the same path on the running system.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use super::path::{SnapPath, SnapshotRoot, live_dir, resolve_dir};
use crate::error::Result;

/// Most entries one browse returns; `truncated` says there were more.
pub const MAX_ENTRIES: usize = 10_000;

/// What an entry is. Devices, FIFOs and sockets are [`Kind::Other`]: listed, never restored in
/// folder mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Link,
    Other,
}

impl Kind {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Dir => "dir",
            Kind::Link => "link",
            Kind::Other => "other",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "file" => Kind::File,
            "dir" => Kind::Dir,
            "link" => Kind::Link,
            "other" => Kind::Other,
            _ => return None,
        })
    }

    fn of(meta: &Metadata) -> Self {
        let kind = meta.file_type();
        if kind.is_symlink() {
            Kind::Link
        } else if kind.is_dir() {
            Kind::Dir
        } else if kind.is_file() {
            Kind::File
        } else {
            Kind::Other
        }
    }
}

/// The same path on the running system, compared without following symlinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Live {
    /// Not there (or a folder on the way is missing or a symlink).
    Missing,
    /// Same type, and for a file the same size and mtime (seconds, like rsync's quick check),
    /// for a link the same target.
    Same,
    /// Another type, size, mtime or link target.
    Changed,
    /// A folder is there; folders aren't compared recursively.
    Present,
}

impl Live {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Live::Missing => "missing",
            Live::Same => "same",
            Live::Changed => "changed",
            Live::Present => "present",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "missing" => Live::Missing,
            "same" => Live::Same,
            "changed" => Live::Changed,
            "present" => Live::Present,
            _ => return None,
        })
    }
}

/// One entry of a snapshot folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Not UTF-8 names are shown with `�` (D-Bus strings are UTF-8); such an entry can't be
    /// restored on its own, only with its folder.
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub mtime: i64,
    /// `st_mode`, type bits included.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// `user:group` from the running system's names, numbers where it has none.
    pub owner: String,
    /// A symlink's target, as written; empty otherwise.
    pub target: String,
    pub live: Live,
    /// The running system's size and mtime, 0 when missing.
    pub live_size: u64,
    pub live_mtime: i64,
}

/// A folder's entries, sorted by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// There were more than [`MAX_ENTRIES`].
    pub truncated: bool,
}

/// User and group names, from `/etc/passwd` and `/etc/group`.
#[derive(Debug, Clone, Default)]
pub struct Names {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
}

impl Names {
    #[must_use]
    pub fn parse(passwd: &str, group: &str) -> Self {
        let ids = |text: &str| {
            text.lines()
                .filter_map(|line| {
                    let mut fields = line.split(':');
                    let name = fields.next()?;
                    let id = fields.nth(1)?.parse().ok()?;
                    Some((id, name.to_owned()))
                })
                .collect()
        };
        Self {
            users: ids(passwd),
            groups: ids(group),
        }
    }

    /// `user:group`, with the number where there's no name.
    #[must_use]
    pub fn owner(&self, uid: u32, gid: u32) -> String {
        let user = self.users.get(&uid).cloned().unwrap_or(uid.to_string());
        let group = self.groups.get(&gid).cloned().unwrap_or(gid.to_string());
        format!("{user}:{group}")
    }
}

/// The folder `path` of the snapshot (a real folder, reached without following a symlink), and
/// for each entry how the same path on the running system under `live_root` compares.
///
/// # Errors
///
/// See [`resolve_dir`]; reading the folder.
pub fn browse(
    root: &SnapshotRoot,
    path: &SnapPath,
    live_root: &Path,
    names: &Names,
) -> Result<Listing> {
    let dir = resolve_dir(root, path)?;
    let live = live_dir(live_root, path);
    let mut found: Vec<(Vec<u8>, PathBuf)> = fs::read_dir(&dir)?
        .map(|entry| {
            let entry = entry?;
            let name = entry.file_name();
            Ok((
                std::os::unix::ffi::OsStrExt::as_bytes(name.as_os_str()).to_vec(),
                entry.path(),
            ))
        })
        .collect::<io::Result<_>>()?;
    found.sort();
    let truncated = found.len() > MAX_ENTRIES;
    found.truncate(MAX_ENTRIES);
    let entries = found
        .into_iter()
        .map(|(name, full)| {
            let meta = fs::symlink_metadata(&full)?;
            let live = live
                .as_ref()
                .map(|dir| dir.join(full.file_name().unwrap_or_default()));
            Ok(entry(
                String::from_utf8_lossy(&name).into_owned(),
                &full,
                &meta,
                live.as_deref(),
                names,
            ))
        })
        .collect::<io::Result<_>>()?;
    Ok(Listing { entries, truncated })
}

fn entry(name: String, path: &Path, meta: &Metadata, live: Option<&Path>, names: &Names) -> Entry {
    let kind = Kind::of(meta);
    let target = link_text(path, kind);
    let (live, live_size, live_mtime) = match live.map(fs::symlink_metadata) {
        Some(Ok(live_meta)) => {
            let live_kind = Kind::of(&live_meta);
            let state = if live_kind != kind || !same_special(meta, &live_meta) {
                Live::Changed
            } else {
                match kind {
                    Kind::Dir => Live::Present,
                    Kind::File
                        if meta.size() != live_meta.size() || meta.mtime() != live_meta.mtime() =>
                    {
                        Live::Changed
                    }
                    Kind::Link if live.map(|p| link_text(p, kind)).as_deref() != Some(&target) => {
                        Live::Changed
                    }
                    _ => Live::Same,
                }
            };
            (state, live_meta.size(), live_meta.mtime())
        }
        _ => (Live::Missing, 0, 0),
    };
    Entry {
        name,
        kind,
        size: meta.size(),
        mtime: meta.mtime(),
        mode: meta.mode(),
        uid: meta.uid(),
        gid: meta.gid(),
        owner: names.owner(meta.uid(), meta.gid()),
        target,
        live,
        live_size,
        live_mtime,
    }
}

/// For [`Kind::Other`]: the same sort of special file (FIFO, socket, block or char device).
fn same_special(a: &Metadata, b: &Metadata) -> bool {
    let (a, b) = (a.file_type(), b.file_type());
    a.is_fifo() == b.is_fifo()
        && a.is_socket() == b.is_socket()
        && a.is_block_device() == b.is_block_device()
        && a.is_char_device() == b.is_char_device()
}

fn link_text(path: &Path, kind: Kind) -> String {
    if kind != Kind::Link {
        return String::new();
    }
    fs::read_link(path)
        .map(|target| target.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_have_names_or_numbers() {
        let names = Names::parse(
            "root:x:0:0:root:/root:/bin/bash\nuser1:x:1000:1000::/home/user1:/bin/sh\nbad line\n",
            "root:x:0:\nuser1:x:1000:\n",
        );
        assert_eq!(names.owner(0, 0), "root:root");
        assert_eq!(names.owner(1000, 1000), "user1:user1");
        assert_eq!(names.owner(1234, 0), "1234:root");
    }

    #[test]
    fn words_round_trip() {
        for kind in [Kind::File, Kind::Dir, Kind::Link, Kind::Other] {
            assert_eq!(Kind::from_word(kind.word()), Some(kind));
        }
        for live in [Live::Missing, Live::Same, Live::Changed, Live::Present] {
            assert_eq!(Live::from_word(live.word()), Some(live));
        }
        assert_eq!(Kind::from_word("x"), None);
        assert_eq!(Live::from_word("x"), None);
    }
}
