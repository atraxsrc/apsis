// SPDX-License-Identifier: GPL-3.0-only

use std::ffi::OsString;
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::backend::Backend;
use crate::error::{Error, Result};
use crate::model::{SnapshotList, parse_snapshot_name};
use crate::parse::parse_list;

const PROGRAM: &str = "timeshift";

/// Longest comment Apsis passes to `timeshift --comments`, in characters.
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
/// The applet provides a runner that adds privilege escalation; tests provide a fake.
pub trait Runner {
    /// # Errors
    ///
    /// When the program can't be started.
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput>;
}

/// [`Backend`] that drives the `timeshift` command line.
///
/// After each successful [`list`](Backend::list) it remembers the backup device and passes it as
/// `--snapshot-device` on later calls, so every call targets the device the user is looking at.
/// [`create`](Backend::create) and [`delete`](Backend::delete) refuse to run until a list has
/// shown a device ([`Error::NoSnapshotDevice`]), rather than fall back to Timeshift's default.
pub struct TimeshiftCli<R> {
    runner: R,
    snapshot_device: Mutex<Option<String>>,
}

impl<R: Runner> TimeshiftCli<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            snapshot_device: Mutex::new(None),
        }
    }

    /// `timeshift <action...> --scripted [--snapshot-device <dev>]`
    fn command(&self, action: &[&str]) -> Vec<OsString> {
        let device = self.remembered_device().clone();
        build_command(action, device.as_deref())
    }

    /// `timeshift <action...> --scripted --snapshot-device <dev>`, only when a device is known.
    fn targeted_command(&self, action: &[&str]) -> Result<Vec<OsString>> {
        let device = self
            .remembered_device()
            .clone()
            .ok_or(Error::NoSnapshotDevice)?;
        Ok(build_command(action, Some(&device)))
    }

    /// Runs `argv` and returns stdout, or an error if it couldn't start or reported failure.
    fn run(&self, argv: &[OsString]) -> Result<String> {
        let output = self.runner.run(argv).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotInstalled,
            _ => Error::Io(e),
        })?;
        if !output.success {
            return Err(Error::Failed {
                code: output.code,
                stderr: output.stderr,
            });
        }
        Ok(output.stdout)
    }

    fn remembered_device(&self) -> MutexGuard<'_, Option<String>> {
        self.snapshot_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn build_command(action: &[&str], device: Option<&str>) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![PROGRAM.into()];
    argv.extend(action.iter().map(OsString::from));
    argv.push("--scripted".into());
    if let Some(device) = device {
        argv.push("--snapshot-device".into());
        argv.push(device.into());
    }
    argv
}

/// Trims the comment and checks it's safe to pass as one argument and show in `--list`.
///
/// [`Backend::create`] runs this too; the applet calls it first so a bad comment can be fixed
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
    // Timeshift should take the next argv as the `--comments` value regardless, but a comment
    // that looks like an option isn't worth the risk.
    if comment.starts_with('-') {
        return Err(Error::InvalidComment("must not start with '-'"));
    }
    if comment.chars().count() > MAX_COMMENT_CHARS {
        return Err(Error::InvalidComment("too long"));
    }
    Ok(comment)
}

impl<R: Runner> Backend for TimeshiftCli<R> {
    fn list(&self) -> Result<SnapshotList> {
        let stdout = self.run(&self.command(&["--list"]))?;
        let list = parse_list(&stdout)?;
        *self.remembered_device() = list.snapshot_device().map(str::to_owned);
        Ok(list)
    }

    fn create(&self, comment: &str) -> Result<()> {
        // No `--tags`: v24.01.1 rejects `--tags O`, and O is the default (see TIMESHIFT-CLI.md).
        let comment = validate_comment(comment)?;
        let argv = if comment.is_empty() {
            self.targeted_command(&["--create"])?
        } else {
            self.targeted_command(&["--create", "--comments", comment])?
        };
        self.run(&argv).map(drop)
    }

    fn delete(&self, name: &str) -> Result<()> {
        // Only real snapshot names reach argv, so nothing here can be read as an option.
        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.to_owned()));
        }
        self.run(&self.targeted_command(&["--delete", "--snapshot", name])?)
            .map(drop)
    }
}
