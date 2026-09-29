// SPDX-License-Identifier: GPL-3.0-only

//! Deleting a snapshot folder without ever leaving it: every folder is opened with
//! `openat(O_NOFOLLOW | O_DIRECTORY)` relative to the one above, symlinks are removed as links
//! (never followed), and a folder on another filesystem (something mounted inside the
//! snapshot) stops the delete instead of being emptied.
//!
//! `std::fs::remove_dir_all` doesn't follow symlinks either, but it does descend into a mount
//! point and delete what's there.

use std::ffi::CStr;
use std::os::fd::OwnedFd;
use std::path::Path;

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, fstat, open, openat, unlinkat};
use rustix::io::Errno;

use crate::error::{Error, Result};

fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Deletes `parent/name` and everything in it. `parent` must be a real folder (not reached
/// through a symlink at its last component), and `name` a folder in it on the same
/// filesystem.
///
/// # Errors
///
/// As [`remove_at`]; also `parent` can't be opened.
pub fn remove_tree(parent: &Path, name: &str) -> Result<()> {
    let parent_fd = open(parent, dir_flags(), Mode::empty()).map_err(|e| {
        Error::Native(format!(
            "{}/{name}: can't open the folder above: {e}",
            parent.display()
        ))
    })?;
    remove_at(&parent_fd, parent, name)
}

/// Deletes the folder `name` in the already opened folder `parent_fd` (`parent` is only for
/// messages), and everything in it, never following a symlink and never leaving the
/// filesystem `parent_fd` is on.
///
/// # Errors
///
/// `name` is a symlink or not a folder; a folder inside is on another filesystem (the delete
/// stops there, leaving what isn't deleted yet); or an I/O error.
pub fn remove_at(parent_fd: &OwnedFd, parent: &Path, name: &str) -> Result<()> {
    let describe = |what: &str| format!("{}/{name}: {what}", parent.display());
    let device = fstat(parent_fd).map_err(io)?.st_dev;
    let cname =
        std::ffi::CString::new(name).map_err(|_| Error::Native(describe("a NUL in the name")))?;
    remove_in(parent_fd, &cname, device).map_err(|e| match e {
        Removal::Refused(what) => Error::Native(describe(&what)),
        Removal::Io(errno) => Error::Native(describe(&errno.to_string())),
    })
}

enum Removal {
    Refused(String),
    Io(Errno),
}

impl From<Errno> for Removal {
    fn from(errno: Errno) -> Self {
        Self::Io(errno)
    }
}

fn io(errno: Errno) -> Error {
    Error::Io(std::io::Error::from(errno))
}

/// Deletes the folder `name` in `parent` (both on `device`).
fn remove_in(parent: &OwnedFd, name: &CStr, device: u64) -> std::result::Result<(), Removal> {
    let fd = match openat(parent, name, dir_flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::LOOP | Errno::NOTDIR) => {
            return Err(Removal::Refused(format!(
                "{} is a symlink or not a folder; not deleting through it",
                name.to_string_lossy()
            )));
        }
        Err(errno) => return Err(errno.into()),
    };
    if fstat(&fd)?.st_dev != device {
        return Err(Removal::Refused(format!(
            "another filesystem is mounted at {}; not deleting into it",
            name.to_string_lossy()
        )));
    }
    // Names first, then deletes: no unlinking while the directory stream is open.
    let mut entries: Vec<(std::ffi::CString, FileType)> = Vec::new();
    let mut dir = Dir::read_from(&fd)?;
    while let Some(entry) = dir.read() {
        let entry = entry?;
        let bytes = entry.file_name().to_bytes();
        if bytes != b"." && bytes != b".." {
            entries.push((entry.file_name().to_owned(), entry.file_type()));
        }
    }
    drop(dir);
    for (child, kind) in entries {
        if kind == FileType::Directory {
            remove_in(&fd, &child, device)?;
            continue;
        }
        match unlinkat(&fd, &child, AtFlags::empty()) {
            Ok(()) => {}
            // `d_type` unknown, and it's a folder after all.
            Err(Errno::ISDIR) => remove_in(&fd, &child, device)?,
            Err(errno) => return Err(errno.into()),
        }
    }
    drop(fd);
    unlinkat(parent, name, AtFlags::REMOVEDIR)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apsis-prune-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn removes_a_tree_but_not_what_its_symlinks_point_at() {
        let root = temp("links");
        let outside = root.join("outside");
        fs::create_dir_all(outside.join("keep")).unwrap();
        fs::write(outside.join("keep/file"), "precious").unwrap();
        let snap = root.join("snapshots/2026-09-29_14-00-00");
        fs::create_dir_all(snap.join("localhost/etc/deep/er")).unwrap();
        fs::write(snap.join("localhost/etc/deep/er/f"), "x").unwrap();
        symlink(&outside, snap.join("localhost/to-dir")).unwrap();
        symlink(
            outside.join("keep/file"),
            snap.join("localhost/etc/to-file"),
        )
        .unwrap();
        symlink("/nonexistent", snap.join("localhost/dangling")).unwrap();

        remove_tree(&root.join("snapshots"), "2026-09-29_14-00-00").unwrap();

        assert!(!snap.exists());
        assert_eq!(
            fs::read_to_string(outside.join("keep/file")).unwrap(),
            "precious"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_a_symlinked_snapshot_folder() {
        let root = temp("symlinked");
        let elsewhere = root.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("file"), "x").unwrap();
        fs::create_dir_all(root.join("snapshots")).unwrap();
        symlink(&elsewhere, root.join("snapshots/2026-09-29_14-00-00")).unwrap();

        let error = remove_tree(&root.join("snapshots"), "2026-09-29_14-00-00").unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
        assert!(elsewhere.join("file").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_a_parent_reached_through_a_symlink() {
        let root = temp("parent");
        fs::create_dir_all(root.join("real/2026-09-29_14-00-00")).unwrap();
        symlink(root.join("real"), root.join("snapshots")).unwrap();
        assert!(remove_tree(&root.join("snapshots"), "2026-09-29_14-00-00").is_err());
        assert!(root.join("real/2026-09-29_14-00-00").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
