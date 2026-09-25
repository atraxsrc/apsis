// SPDX-License-Identifier: GPL-3.0-only

//! Snapshot model, `Backend` trait and the Timeshift CLI backend for Apsis.
//!
//! This crate has no UI dependencies and must stay testable without root or a COSMIC session.

mod backend;
mod error;
pub mod helper;
mod model;
pub mod native;
mod parse;
mod pkexec;
pub mod settings;
mod timeshift;

pub use backend::Backend;
pub use error::{Error, Result};
pub use model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
pub use parse::parse_list;
pub use pkexec::{PkexecRunner, find_in_path, pkexec_command};
pub use timeshift::{
    MAX_COMMENT_CHARS, MAX_OUTPUT_LINES, RunOutput, Runner, TimeshiftCli, validate_comment,
};
