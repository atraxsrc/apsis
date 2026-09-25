// SPDX-License-Identifier: GPL-3.0-only

use std::io;

/// Everything that can go wrong talking to a snapshot backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The output had neither a snapshot table nor "No snapshots found".
    #[error("unrecognised `timeshift --list` output")]
    UnrecognisedOutput,
    /// A line inside the snapshot table didn't look like a snapshot row.
    #[error("line {line}: unexpected snapshot row: {text:?}")]
    BadRow { line: usize, text: String },
    /// Not a Timeshift snapshot name (`YYYY-MM-DD_HH-MM-SS`).
    #[error("invalid snapshot name: {0:?}")]
    InvalidSnapshotName(String),
    /// The comment can't be passed to Timeshift safely.
    #[error("invalid comment: {0}")]
    InvalidComment(&'static str),
    /// Create and delete only run against a device a list has shown; none is known yet.
    #[error("no snapshot device known; list snapshots first")]
    NoSnapshotDevice,
    /// The `timeshift` binary wasn't found.
    #[error("timeshift is not installed")]
    NotInstalled,
    /// Timeshift ran but reported failure.
    #[error("timeshift failed (exit code {code:?}): {stderr}")]
    Failed { code: Option<i32>, stderr: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
