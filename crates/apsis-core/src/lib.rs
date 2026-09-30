// SPDX-License-Identifier: GPL-3.0-only

//! Snapshot model, the native rsync backend and Apsis's config.
//!
//! This crate has no UI dependencies and must stay testable without root or a COSMIC session.

mod backend;
pub mod config;
mod error;
pub mod helper;
pub mod job;
mod model;
pub mod native;
pub mod progress;
mod runner;
pub mod settings;
pub mod status;
pub mod usage;

pub use backend::Backend;
pub use error::{Error, Result};
pub use model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
pub use progress::Progress;
pub use runner::{MAX_COMMENT_CHARS, RunOutput, Runner, find_in_path, validate_comment};
pub use status::{ApsisStatus, DiskStatus, Due, Severity};
pub use usage::DiskUsage;
