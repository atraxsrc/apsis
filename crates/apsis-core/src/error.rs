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
    /// The backup disk left while a create or delete ran (its `/dev/disk/by-uuid` link is
    /// gone). `reason` is what failed because of it (rsync's I/O error, usually).
    #[error("backup disk removed: {reason}")]
    DeviceRemoved { device: String, reason: String },
    /// A create was stopped (`Stop`); what it had copied is gone.
    #[error("stopped")]
    Stopped,
    /// A delete of several (`DeleteMany`) stopped at `failed`: the `deleted` ones are gone,
    /// in order; `left` weren't touched; `reason` is what went wrong with `failed`.
    #[error("delete stopped at {failed}: {reason}")]
    DeleteManyStopped {
        deleted: Vec<String>,
        failed: String,
        left: Vec<String>,
        reason: Box<Error>,
    },
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
    /// A request that is refused before anything runs (a delete that isn't a plain snapshot
    /// folder). The text says why.
    #[error("{0}")]
    InvalidInput(String),
    /// A full-system restore the helper refused (PLAN 6b.7): the refusal's word on the wire
    /// (`restore::refusal::Refusal::to_wire`), which the applet decodes for its dialog.
    #[error("can't restore this snapshot: {0}")]
    RestoreRefused(String),
    /// A delete the helper refused because a restore is armed and waits for the restart
    /// (PLAN 6b.5): no snapshot goes while one, the armed plan's or its safety snapshot
    /// among them. Nothing ran.
    #[error("a restore is armed and waits for the restart")]
    RestoreArmed,
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
