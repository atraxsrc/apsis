// SPDX-License-Identifier: GPL-3.0-only

//! Paths inside a snapshot, checked lexically, and resolved on disk without following a
//! symlink.

use std::fmt;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::parse_snapshot_name;
use crate::native::{SNAPSHOTS_DIR, TIMESHIFT_DIR};

/// Longest path accepted, in bytes (Linux's `PATH_MAX`).
pub const MAX_PATH_BYTES: usize = 4096;

/// The folder in a snapshot that holds its copy of `/` (`Main.vala:1593-1594`).
pub const LOCALHOST: &str = "localhost";

/// A path as it was on the system the snapshot was taken of: `/etc/fstab`, or `/` for the
/// snapshot's root. Checked lexically: absolute, no NUL, no empty, `.` or `..` components.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapPath {
    parts: Vec<String>,
}

impl SnapPath {
    /// `/`.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// [`Error::InvalidInput`] with the reason.
    pub fn parse(text: &str) -> Result<Self> {
        let refuse = |why: &str| Err(Error::InvalidInput(format!("{text:?}: {why}")));
        if text.len() > MAX_PATH_BYTES {
            return refuse("too long");
        }
        if text.contains('\0') {
            return refuse("contains a NUL byte");
        }
        let Some(rest) = text.strip_prefix('/') else {
            return refuse("not an absolute path");
        };
        let mut path = Self::root();
        if rest.is_empty() {
            return Ok(path);
        }
        for part in rest.split('/') {
            match part {
                "" => return refuse("empty path component"),
                "." | ".." => return refuse("`.` and `..` aren't allowed"),
                part => path.parts.push(part.to_owned()),
            }
        }
        Ok(path)
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.parts.is_empty()
    }

    #[must_use]
    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    /// The last component; `None` for `/`.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.parts.last().map(String::as_str)
    }

    /// The folder it's in; `None` for `/`.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let (_, parent) = self.parts.split_last()?;
        Some(Self {
            parts: parent.to_vec(),
        })
    }

    /// `self/name`, with `name` checked like a component of [`SnapPath::parse`].
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] for a name with `/` or NUL, or `.`, `..`, empty.
    pub fn join(&self, name: &str) -> Result<Self> {
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
            return Err(Error::InvalidInput(format!("{name:?}: not a file name")));
        }
        let mut parts = self.parts.clone();
        parts.push(name.to_owned());
        Ok(Self { parts })
    }

    /// The first `n` components.
    fn prefix(&self, n: usize) -> Self {
        Self {
            parts: self.parts[..n].to_vec(),
        }
    }

    /// Relative to the root: `etc/fstab` (empty for `/`).
    #[must_use]
    pub fn relative(&self) -> PathBuf {
        self.parts.iter().collect()
    }

    /// Where it is under `base`.
    #[must_use]
    pub fn under(&self, base: &Path) -> PathBuf {
        base.join(self.relative())
    }

    /// Whether it is `top` or inside it (`top` is one component, like `etc`).
    #[must_use]
    pub fn is_in(&self, top: &str) -> bool {
        self.parts.first().is_some_and(|first| first == top)
    }
}

impl fmt::Display for SnapPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.parts.is_empty() {
            return f.write_str("/");
        }
        for part in &self.parts {
            write!(f, "/{part}")?;
        }
        Ok(())
    }
}

/// One snapshot's copy of `/`: `<repo>/timeshift/snapshots/<name>/localhost`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRoot {
    pub name: String,
    pub localhost: PathBuf,
}

/// The snapshot `name` in `repo` (the backup device's mount). The name must be Timeshift's
/// pattern, and `timeshift/`, `snapshots/`, `<name>/` and `localhost/` must each be a real
/// folder, not a symlink.
///
/// # Errors
///
/// [`Error::InvalidSnapshotName`], [`Error::NoSuchSnapshot`], or [`Error::InvalidInput`] for a
/// symlink on the way.
pub fn open_snapshot(repo: &Path, name: &str) -> Result<SnapshotRoot> {
    if parse_snapshot_name(name).is_none() {
        return Err(Error::InvalidSnapshotName(name.to_owned()));
    }
    let mut dir = repo.to_path_buf();
    for part in [TIMESHIFT_DIR, SNAPSHOTS_DIR, name, LOCALHOST] {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(Error::InvalidInput(format!(
                    "{} is a symlink or not a folder",
                    dir.display()
                )));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::NoSuchSnapshot(name.to_owned()));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(SnapshotRoot {
        name: name.to_owned(),
        localhost: dir,
    })
}

/// A path found in a snapshot: where it is on disk and its `lstat`.
#[derive(Debug)]
pub struct Resolved {
    pub path: PathBuf,
    pub meta: Metadata,
}

/// `path` inside the snapshot, never following a symlink: every component before the last must
/// be a real folder (checked with `lstat`, one at a time), the last is looked at with `lstat`.
/// A symlink last is kept as a symlink, but refused if its target climbs above the snapshot's
/// root ([`link_escapes`]).
///
/// # Errors
///
/// [`Error::InvalidInput`]: not in the snapshot, a symlink or file on the way, an escaping link.
pub fn resolve(root: &SnapshotRoot, path: &SnapPath) -> Result<Resolved> {
    let (found, meta) = walk(&root.localhost, path, "the snapshot")?;
    let Some(meta) = meta else {
        return Err(Error::InvalidInput(format!("{path} isn't in the snapshot")));
    };
    if meta.file_type().is_symlink() {
        let target = fs::read_link(&found)?;
        if link_escapes(path.parts().len().saturating_sub(1), &target) {
            return Err(Error::InvalidInput(format!(
                "{path} is a symlink pointing outside the snapshot ({})",
                target.display()
            )));
        }
    }
    Ok(Resolved { path: found, meta })
}

/// The folder `path` names in the snapshot, which must be a real folder (not a symlink).
///
/// # Errors
///
/// As [`resolve`], and [`Error::InvalidInput`] if it's a symlink or not a folder.
pub fn resolve_dir(root: &SnapshotRoot, path: &SnapPath) -> Result<PathBuf> {
    let resolved = resolve(root, path)?;
    if resolved.meta.file_type().is_symlink() {
        return Err(Error::InvalidInput(format!(
            "{path} is a symlink; symlinks aren't followed"
        )));
    }
    if !resolved.meta.is_dir() {
        return Err(Error::InvalidInput(format!("{path} is not a folder")));
    }
    Ok(resolved.path)
}

/// On the running system under `live_root`: the folder `path` names, if every component of it
/// is a real folder (no symlink). `None` if it's missing, or anything on the way isn't.
#[must_use]
pub fn live_dir(live_root: &Path, path: &SnapPath) -> Option<PathBuf> {
    match walk(live_root, path, "the running system") {
        Ok((found, Some(meta))) if meta.is_dir() => Some(found),
        _ => None,
    }
}

/// On the running system under `live_root`: the folder `path` goes back into. It must exist,
/// and it and every folder above it must be a real folder, not a symlink.
///
/// # Errors
///
/// [`Error::InvalidInput`] with the reason.
pub fn live_parent(live_root: &Path, path: &SnapPath) -> Result<PathBuf> {
    const HINT: &str = "restore the folder above it instead";
    let parent = path.parent().unwrap_or_default();
    let (found, meta) = match walk(live_root, &parent, "the running system") {
        Err(Error::InvalidInput(reason)) if reason.contains("doesn't exist") => {
            return Err(Error::InvalidInput(format!("{reason}; {HINT}")));
        }
        other => other?,
    };
    match meta {
        Some(meta) if meta.is_dir() => Ok(found),
        Some(meta) if meta.file_type().is_symlink() => Err(Error::InvalidInput(format!(
            "{parent} is a symlink on the running system; symlinks aren't followed"
        ))),
        Some(_) => Err(Error::InvalidInput(format!(
            "{parent} is not a folder on the running system"
        ))),
        None => Err(Error::InvalidInput(format!(
            "{parent} doesn't exist in the running system; {HINT}"
        ))),
    }
}

/// Walks `path` under `base` with `lstat`, one component at a time. Every component but the
/// last must be a real folder; the last's `lstat` is returned (`None` if it doesn't exist).
fn walk(base: &Path, path: &SnapPath, place: &str) -> Result<(PathBuf, Option<Metadata>)> {
    let mut here = base.to_path_buf();
    let count = path.parts().len();
    if count == 0 {
        return Ok((here.clone(), Some(fs::symlink_metadata(&here)?)));
    }
    for (index, part) in path.parts().iter().enumerate() {
        here.push(part);
        let last = index + 1 == count;
        let meta = match fs::symlink_metadata(&here) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound && last => return Ok((here, None)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let missing = path.prefix(index + 1);
                return Err(Error::InvalidInput(format!(
                    "{missing} doesn't exist in {place}"
                )));
            }
            Err(e) => return Err(e.into()),
        };
        if last {
            return Ok((here, Some(meta)));
        }
        let on_the_way = path.prefix(index + 1);
        if meta.file_type().is_symlink() {
            return Err(Error::InvalidInput(format!(
                "{on_the_way} is a symlink in {place}; symlinks aren't followed"
            )));
        }
        if !meta.is_dir() {
            return Err(Error::InvalidInput(format!(
                "{on_the_way} is not a folder in {place}"
            )));
        }
    }
    unreachable!("the loop returns on the last component")
}

/// Whether a symlink `depth` folders below the snapshot's root, pointing at `target`, climbs
/// above that root, read lexically with the snapshot's `localhost/` as `/`. An absolute target
/// starts again at that root, so it never climbs above it.
#[must_use]
pub fn link_escapes(depth: usize, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth = depth;
    for component in target.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => match depth.checked_sub(1) {
                Some(up) => depth = up,
                None => return true,
            },
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(text: &str) -> bool {
        matches!(SnapPath::parse(text), Err(Error::InvalidInput(_)))
    }

    #[test]
    fn paths_are_absolute_without_dot_components() {
        assert_eq!(SnapPath::parse("/").unwrap(), SnapPath::root());
        let path = SnapPath::parse("/etc/NetworkManager/x y.conf").unwrap();
        assert_eq!(path.parts(), ["etc", "NetworkManager", "x y.conf"]);
        assert_eq!(path.to_string(), "/etc/NetworkManager/x y.conf");
        assert_eq!(path.relative(), Path::new("etc/NetworkManager/x y.conf"));
        assert_eq!(path.parent().unwrap().to_string(), "/etc/NetworkManager");
        assert_eq!(path.name(), Some("x y.conf"));
        assert!(path.is_in("etc") && !path.is_in("et"));
        // `..` inside a name is just a name.
        assert!(SnapPath::parse("/etc/..hidden").is_ok());
        assert!(SnapPath::parse("/etc/a..b").is_ok());
        for bad in [
            "",
            "etc/fstab",
            "./etc",
            "/etc/../shadow",
            "/..",
            "/../etc",
            "/etc/./fstab",
            "/etc//fstab",
            "/etc/",
            "/etc\0/x",
        ] {
            assert!(refused(bad), "{bad:?}");
        }
        assert!(refused(&format!("/{}", "a".repeat(MAX_PATH_BYTES))));
    }

    #[test]
    fn join_takes_only_file_names() {
        let etc = SnapPath::parse("/etc").unwrap();
        assert_eq!(etc.join("fstab").unwrap().to_string(), "/etc/fstab");
        for bad in ["", ".", "..", "a/b", "/abs", "x\0"] {
            assert!(etc.join(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn relative_links_may_not_climb_above_the_root() {
        // /etc/alternatives/vi -> ../../usr/bin/vim.basic: depth 2, climbs 2.
        assert!(!link_escapes(2, Path::new("../../usr/bin/vim.basic")));
        assert!(link_escapes(2, Path::new("../../../etc/shadow")));
        assert!(link_escapes(0, Path::new("..")));
        assert!(!link_escapes(1, Path::new("./a/../..")));
        assert!(link_escapes(1, Path::new("a/../../..")));
        // Absolute targets start again at the snapshot's root.
        assert!(!link_escapes(0, Path::new("/etc/shadow")));
        assert!(!link_escapes(3, Path::new("/../../..")));
    }
}
