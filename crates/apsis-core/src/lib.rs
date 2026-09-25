// SPDX-License-Identifier: GPL-3.0-only

//! Snapshot model, `Backend` trait and the Timeshift CLI backend for Apsis.
//!
//! This crate has no UI dependencies and must stay testable without root or a COSMIC session.

mod backend;
mod error;
mod model;
mod parse;
mod pkexec;
mod timeshift;

pub use backend::Backend;
pub use error::{Error, Result};
pub use model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
pub use parse::parse_list;
pub use pkexec::{PkexecRunner, find_in_path, pkexec_command};
pub use timeshift::{MAX_COMMENT_CHARS, RunOutput, Runner, TimeshiftCli};
