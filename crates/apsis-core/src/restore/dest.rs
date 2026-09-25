// SPDX-License-Identifier: GPL-3.0-only

//! Folder mode's destination, `<home>/Apsis-restored/<snapshot>/`, made so that root never
//! writes through a path the user could swap for a symlink.
//!
//! 1. Every component of the home folder is opened from `/` with `O_NOFOLLOW | O_DIRECTORY`,
//!    each relative to the one before (`openat`). A symlink anywhere fails the open (`ELOOP`
//!    or `ENOTDIR`). The home must be owned by the caller.
//! 2. `Apsis-restored/` is opened the same way from the home's descriptor, made (and given to
//!    the caller) if missing, and must be owned by the caller.
//! 3. A new `<snapshot>/` (or `<snapshot>-2/`, ...) is made in it with `mkdirat`, opened with
//!    `O_NOFOLLOW`, and must be empty and owned by us (root in the helper). It's mode 0700 while
//!    the copy runs, so nobody else can put anything inside it.
//! 4. rsync writes through the held descriptor, not the path: the descriptor is made
//!    inheritable just for the rsync run, and rsync's destination is `/proc/self/fd/<n>/`.
//!    In rsync's process that magic link is its inherited copy of the descriptor, the folder
//!    made in step 3, wherever it has been moved since. Anything inside it can only be made by
//!    root, since only root can enter it.
//! 5. Afterwards the folder itself is given to the caller with `fchown`/`fchmod` on the
//!    descriptor (its contents were already chowned by rsync's `--chown`). No path is followed
//!    for the hand-over, so a symlink in the snapshot can't redirect it.

use std::ffi::OsStr;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{
    AtFlags, CWD, Dir, Gid, Mode, OFlags, Uid, fchmod, fchown, fstat, mkdirat, openat, statat,
};
use rustix::io::{Errno, FdFlags, fcntl_setfd};

use crate::error::{Error, Result};

/// The folder in the caller's home that folder mode restores into.
pub const RESTORED_DIR: &str = "Apsis-restored";

/// `<snapshot>`, then `<snapshot>-2` up to this.
const MAX_TRIES: u32 = 99;

/// The open flags for every folder on the way: never follow a symlink, must be a folder.
fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// A new, empty folder for one restore, held open (see the module docs).
#[derive(Debug)]
pub struct Pinned {
    fd: OwnedFd,
    /// Where it was made, for showing (never used to reach it).
    pub path: PathBuf,
}

impl Pinned {
    /// rsync's destination: this folder, through the descriptor (see [`Pinned::inherited`]).
    #[must_use]
    pub fn proc_path(&self) -> String {
        format!("/proc/self/fd/{}/", self.fd.as_raw_fd())
    }

    /// Makes the descriptor inheritable until the guard drops, so a program started meanwhile
    /// (rsync) has it at the same number.
    ///
    /// # Errors
    ///
    /// `fcntl` failed.
    pub fn inherited(&self) -> Result<Inherited<'_>> {
        fcntl_setfd(&self.fd, FdFlags::empty()).map_err(io)?;
        Ok(Inherited(self.fd.as_fd()))
    }

    /// Gives the folder to `uid:gid`, mode 0755, through the descriptor.
    ///
    /// # Errors
    ///
    /// `fchown` or `fchmod` failed.
    pub fn hand_over(&self, uid: u32, gid: u32) -> Result<()> {
        fchown(&self.fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid))).map_err(io)?;
        fchmod(&self.fd, Mode::from_raw_mode(0o755)).map_err(io)
    }
}

/// See [`Pinned::inherited`].
pub struct Inherited<'a>(std::os::fd::BorrowedFd<'a>);

impl Drop for Inherited<'_> {
    fn drop(&mut self) {
        let _ = fcntl_setfd(self.0, FdFlags::CLOEXEC);
    }
}

/// Makes and holds `<home>/Apsis-restored/<snapshot>[-n]/` for the caller `uid:gid` (see the
/// module docs).
///
/// # Errors
///
/// [`Error::InvalidInput`] when the home or `Apsis-restored/` isn't a real folder owned by the
/// caller, or a symlink is on the way; I/O errors otherwise.
pub fn prepare(home: &Path, uid: u32, gid: u32, snapshot: &str) -> Result<Pinned> {
    let home_fd = open_home(home, uid)?;
    let restored = open_restored(&home_fd, home, uid, gid)?;
    for n in 1..=MAX_TRIES {
        let name = folder_name(snapshot, n);
        match mkdirat(&restored, name.as_str(), Mode::from_raw_mode(0o700)) {
            Ok(()) => {}
            Err(Errno::EXIST) => continue,
            Err(e) => return Err(io(e)),
        }
        let path = home.join(RESTORED_DIR).join(&name);
        let fd = openat(&restored, name.as_str(), dir_flags(), Mode::empty())
            .map_err(|e| refused_open(&path, e))?;
        let stat = fstat(&fd).map_err(io)?;
        if stat.st_uid != rustix::process::geteuid().as_raw() || !is_empty(&fd)? {
            return Err(Error::InvalidInput(format!(
                "{} changed while it was being made",
                path.display()
            )));
        }
        // Whatever the umask left: only us while the copy runs.
        fchmod(&fd, Mode::from_raw_mode(0o700)).map_err(io)?;
        return Ok(Pinned { fd, path });
    }
    Err(Error::InvalidInput(format!(
        "{} already has {MAX_TRIES} restores of {snapshot}; move some away",
        home.join(RESTORED_DIR).display()
    )))
}

/// Where [`prepare`] would put a restore of `snapshot` now, for a dry run. Looked up by path;
/// nothing is made.
#[must_use]
pub fn would_be(home: &Path, snapshot: &str) -> PathBuf {
    let restored = home.join(RESTORED_DIR);
    (1..=MAX_TRIES)
        .map(|n| restored.join(folder_name(snapshot, n)))
        .find(|path| std::fs::symlink_metadata(path).is_err())
        .unwrap_or_else(|| restored.join(snapshot))
}

fn folder_name(snapshot: &str, n: u32) -> String {
    if n == 1 {
        snapshot.to_owned()
    } else {
        format!("{snapshot}-{n}")
    }
}

/// The home folder, opened one component at a time from `/`, never through a symlink, and
/// owned by `uid`.
fn open_home(home: &Path, uid: u32) -> Result<OwnedFd> {
    if !home.is_absolute() {
        return Err(Error::InvalidInput(format!(
            "home folder {} is not an absolute path",
            home.display()
        )));
    }
    let mut fd = openat(CWD, "/", dir_flags(), Mode::empty()).map_err(io)?;
    let mut here = PathBuf::from("/");
    for component in home.components() {
        let name: &OsStr = match component {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => {
                return Err(Error::InvalidInput(format!(
                    "home folder {} has `.` or `..` in it",
                    home.display()
                )));
            }
        };
        here.push(name);
        fd = openat(&fd, name.as_bytes(), dir_flags(), Mode::empty())
            .map_err(|e| refused_open(&here, e))?;
    }
    let owner = fstat(&fd).map_err(io)?.st_uid;
    if owner != uid {
        return Err(Error::InvalidInput(format!(
            "home folder {} belongs to uid {owner}, not the caller (uid {uid})",
            home.display()
        )));
    }
    Ok(fd)
}

/// `Apsis-restored/` in the home: opened without following a symlink, made (and given to the
/// caller) if missing, and owned by `uid`.
fn open_restored(home_fd: &OwnedFd, home: &Path, uid: u32, gid: u32) -> Result<OwnedFd> {
    let path = home.join(RESTORED_DIR);
    let open = || openat(home_fd, RESTORED_DIR, dir_flags(), Mode::empty());
    let fd = match open() {
        Ok(fd) => fd,
        Err(Errno::NOENT) => {
            match mkdirat(home_fd, RESTORED_DIR, Mode::from_raw_mode(0o755)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(e) => return Err(io(e)),
            }
            let fd = open().map_err(|e| refused_open(&path, e))?;
            // Made by us just now (root in the helper): the caller's, like the home.
            if fstat(&fd).map_err(io)?.st_uid == rustix::process::geteuid().as_raw() {
                fchown(&fd, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid))).map_err(io)?;
            }
            fd
        }
        Err(e) => return Err(refused_open(&path, e)),
    };
    check_owner(&fd, &path, uid)?;
    Ok(fd)
}

fn check_owner(fd: &OwnedFd, path: &Path, uid: u32) -> Result<()> {
    let owner = fstat(fd).map_err(io)?.st_uid;
    if owner != uid {
        return Err(Error::InvalidInput(format!(
            "{} belongs to uid {owner}, not the caller (uid {uid}); move it away",
            path.display()
        )));
    }
    Ok(())
}

fn is_empty(fd: &OwnedFd) -> Result<bool> {
    let mut dir = Dir::read_from(fd).map_err(io)?;
    while let Some(entry) = dir.read() {
        let name = entry.map_err(io)?.file_name().to_bytes().to_vec();
        if name != b"." && name != b".." {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A failed `openat` on the way: a symlink (`ELOOP`) or not a folder (`ENOTDIR`) is refused
/// with the path; anything else is an I/O error.
fn refused_open(path: &Path, errno: Errno) -> Error {
    match errno {
        Errno::LOOP | Errno::NOTDIR => Error::InvalidInput(format!(
            "{} is a symlink or not a folder; it isn't followed",
            path.display()
        )),
        other => Error::Io(std::io::Error::other(format!(
            "{}: {}",
            path.display(),
            std::io::Error::from(other)
        ))),
    }
}

fn io(errno: Errno) -> Error {
    Error::Io(errno.into())
}

/// `st_dev` of `path`, not following a symlink at the end.
///
/// # Errors
///
/// `lstat` failed.
pub fn device_of(path: &Path) -> Result<u64> {
    let stat = statat(CWD, path, AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
    Ok(stat.st_dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/restore-dest")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn me() -> (u32, u32) {
        (
            rustix::process::geteuid().as_raw(),
            rustix::process::getegid().as_raw(),
        )
    }

    #[test]
    fn each_restore_gets_a_new_private_folder() {
        let home = lab("fresh");
        let (uid, gid) = me();
        let first = prepare(&home, uid, gid, "2026-09-25_03-00-01").unwrap();
        assert_eq!(first.path, home.join("Apsis-restored/2026-09-25_03-00-01"));
        let meta = std::fs::metadata(&first.path).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o777,
            0o700
        );
        assert_eq!(
            would_be(&home, "2026-09-25_03-00-01"),
            home.join("Apsis-restored/2026-09-25_03-00-01-2")
        );
        let second = prepare(&home, uid, gid, "2026-09-25_03-00-01").unwrap();
        assert_eq!(
            second.path,
            home.join("Apsis-restored/2026-09-25_03-00-01-2")
        );
        first.hand_over(uid, gid).unwrap();
        let meta = std::fs::metadata(&first.path).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o777,
            0o755
        );
    }

    #[test]
    fn a_symlinked_restore_folder_is_refused() {
        let home = lab("symlinked");
        let elsewhere = home.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(RESTORED_DIR)).unwrap();
        let (uid, gid) = me();
        let result = prepare(&home, uid, gid, "2026-09-25_03-00-01");
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[test]
    fn a_symlink_in_the_home_path_is_refused() {
        let base = lab("symlinked-home");
        std::fs::create_dir(base.join("real")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        let (uid, gid) = me();
        let result = prepare(&base.join("link"), uid, gid, "2026-09-25_03-00-01");
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
        assert!(!base.join("real").join(RESTORED_DIR).exists());
        let relative = prepare(Path::new("home/x"), uid, gid, "2026-09-25_03-00-01");
        assert!(matches!(relative, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn someone_elses_home_or_restore_folder_is_refused() {
        let home = lab("owner");
        let (uid, gid) = me();
        let other = uid.wrapping_add(1);
        let result = prepare(&home, other, gid, "2026-09-25_03-00-01");
        assert!(
            matches!(&result, Err(Error::InvalidInput(m)) if m.contains("belongs to uid")),
            "{result:?}"
        );
        // Apsis-restored/ owned by someone else than the caller.
        std::fs::create_dir(home.join(RESTORED_DIR)).unwrap();
        let home_fd = open_home(&home, uid).unwrap();
        let fd = openat(&home_fd, RESTORED_DIR, dir_flags(), Mode::empty()).unwrap();
        assert!(check_owner(&fd, &home.join(RESTORED_DIR), other).is_err());
        assert!(check_owner(&fd, &home.join(RESTORED_DIR), uid).is_ok());
    }
}
