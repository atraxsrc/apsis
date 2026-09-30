// SPDX-License-Identifier: GPL-3.0-only

//! Running programs without a shell, and checking what goes into their argv.

use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::native::Cancel;

/// Longest snapshot comment, in characters (Apsis's own limit).
pub const MAX_COMMENT_CHARS: usize = 200;

/// What a finished command produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs an argv (`argv[0]` is the program) without a shell.
///
/// The helper provides runners with a fixed `PATH`; tests provide fakes.
pub trait Runner {
    /// # Errors
    ///
    /// When the program can't be started.
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput>;

    /// Like [`Runner::run`], but hands stdout to `on_segment` while the command runs, in pieces
    /// ending at `\r` or `\n` (see [`crate::progress::read_segments`]). Pieces it returns
    /// `true` for (progress lines) are left out of [`RunOutput::stdout`].
    ///
    /// The default doesn't stream: it runs the command and hands over nothing. Runners that
    /// can't show progress (test fakes) keep it.
    ///
    /// # Errors
    ///
    /// When the program can't be started.
    fn run_streaming(
        &self,
        argv: &[OsString],
        on_segment: &mut dyn FnMut(&str) -> bool,
    ) -> io::Result<RunOutput> {
        let _ = on_segment;
        self.run(argv)
    }

    /// Like [`Runner::run_streaming`], but `cancel` can stop the command while it runs (see
    /// [`crate::native::Cancel`]).
    ///
    /// The default only looks before starting (test fakes finish at once).
    ///
    /// # Errors
    ///
    /// When the program can't be started, or `cancel` was asked to stop first
    /// ([`io::ErrorKind::Interrupted`]).
    fn run_cancellable(
        &self,
        argv: &[OsString],
        on_segment: &mut dyn FnMut(&str) -> bool,
        cancel: &Arc<Cancel>,
    ) -> io::Result<RunOutput> {
        if cancel.is_stopping() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.run_streaming(argv, on_segment)
    }
}

/// Trims the comment and checks it's safe to pass as one argument and show in `--list`.
///
/// [`crate::Backend::create`] runs this too; the applet calls it first so a bad comment can be fixed
/// before any password prompt.
///
/// # Errors
///
/// [`Error::InvalidComment`] for control characters, a leading `-`, or more than
/// [`MAX_COMMENT_CHARS`] characters.
pub fn validate_comment(comment: &str) -> Result<&str> {
    let comment = comment.trim();
    if comment.chars().any(char::is_control) {
        return Err(Error::InvalidComment("must not contain control characters"));
    }
    // A comment goes into info.json, not an argv; the rule is kept from when it went to
    // `timeshift --comments`, so every comment Apsis ever wrote follows the same rules.
    if comment.starts_with('-') {
        return Err(Error::InvalidComment("must not start with '-'"));
    }
    if comment.chars().count() > MAX_COMMENT_CHARS {
        return Err(Error::InvalidComment("too long"));
    }
    Ok(comment)
}

/// Finds an executable file called `name` in a `PATH`-style list. A `name` containing a `/` is
/// used as given.
#[must_use]
pub fn find_in_path(name: &OsStr, path: &OsStr) -> Option<PathBuf> {
    let name = Path::new(name);
    if name.components().count() > 1 {
        return is_executable(name).then(|| name.to_owned());
    }
    env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
