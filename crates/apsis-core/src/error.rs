// SPDX-License-Identifier: GPL-3.0-only

use std::io;

/// Everything that can go wrong with snapshots, the config or the helper.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Not a snapshot name (`YYYY-MM-DD_HH-MM-SS`).
    #[error("invalid snapshot name: {0:?}")]
    InvalidSnapshotName(String),
    /// The comment isn't one Apsis writes (see `validate_comment`).
    #[error("invalid comment: {0}")]
    InvalidComment(&'static str),
    /// No backup device is set in the config yet.
    #[error("no backup device: pick one in settings")]
    NoSnapshotDevice,
    /// Delete named a snapshot that the backup device doesn't have.
    #[error("no snapshot called {0:?} on the backup device")]
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
    /// The backup device isn't connected, e.g. a USB disk that was unplugged or dropped off.
    /// `device` is its UUID.
    #[error("backup device not found: {device}")]
    DeviceNotFound { device: String },
    /// A config file that can't be read safely (`/etc/apsis/config.toml`, or the
    /// `timeshift.json` a first run imports).
    #[error("{0}")]
    InvalidConfig(String),
    /// Settings that mustn't be written (see `config::validate`).
    #[error("{0}")]
    InvalidSettings(String),
    /// The config file changed since it was read.
    #[error("the config changed since it was read; reload the settings")]
    ConfigChanged,
    /// The native backend couldn't do what was asked (rsync failed, the snapshot folder is
    /// taken, the repository is in a state it won't touch, ...).
    #[error("{0}")]
    Native(String),
    /// A request that is refused before anything runs (a restore path, destination, or a
    /// backup name in the way; a delete that isn't a plain snapshot folder). The text says why.
    #[error("{0}")]
    InvalidInput(String),
    /// A file-level restore ran and failed (rsync's exit code and last lines).
    #[error("restore failed: {0}")]
    Restore(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
