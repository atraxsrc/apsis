// SPDX-License-Identifier: GPL-3.0-only

use std::io;

/// Everything that can go wrong talking to a snapshot backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The output had neither a snapshot table nor "No snapshots found".
    #[error("unrecognised `timeshift --list` output")]
    UnrecognisedOutput,
    /// Not a Timeshift snapshot name (`YYYY-MM-DD_HH-MM-SS`).
    #[error("invalid snapshot name: {0:?}")]
    InvalidSnapshotName(String),
    /// The comment can't be passed to Timeshift safely.
    #[error("invalid comment: {0}")]
    InvalidComment(&'static str),
    /// Create and delete only run against a device a list has shown; none is known yet.
    #[error("no snapshot device known; list snapshots first")]
    NoSnapshotDevice,
    /// Delete named a snapshot that the backup device doesn't have.
    #[error("no snapshot called {0:?} on the snapshot device")]
    NoSuchSnapshot(String),
    /// polkit refused, or the password dialog was dismissed.
    #[error("not authorised")]
    NotAuthorized,
    /// The helper is already running a snapshot operation.
    #[error("busy with another snapshot operation")]
    Busy,
    /// Talking to `apsis-helper` failed, or it reported an error not covered above.
    #[error("apsis-helper: {0}")]
    Helper(String),
    /// The `timeshift` binary wasn't found.
    #[error("timeshift is not installed")]
    NotInstalled,
    /// The backup device isn't there (Timeshift: `E: Device not found: '<device>'`), e.g. a USB
    /// disk that was unplugged or dropped off. `device` is what Timeshift named.
    #[error("backup device not found: {device}")]
    DeviceNotFound { device: String },
    /// Timeshift ran but reported failure. `output` is the last [`MAX_OUTPUT_LINES`] lines of
    /// what it said about it: its `E:`/`W:` lines (Timeshift prints those on stdout) and stderr.
    ///
    /// [`MAX_OUTPUT_LINES`]: crate::MAX_OUTPUT_LINES
    #[error("timeshift failed (exit code {code:?}): {output}")]
    Failed { code: Option<i32>, output: String },
    /// Timeshift's settings file can't be edited safely (not JSON, or a field Timeshift reads
    /// as a string isn't one).
    #[error("timeshift.json: {0}")]
    InvalidConfig(String),
    /// Settings that mustn't be written (see `settings::validate`).
    #[error("{0}")]
    InvalidSettings(String),
    /// Timeshift's settings file changed since it was read (Timeshift itself saved it).
    #[error("timeshift.json changed since it was read; reload the settings")]
    SettingsChanged,
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
