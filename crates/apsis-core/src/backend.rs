// SPDX-License-Identifier: GPL-3.0-only

use crate::error::Result;
use crate::model::SnapshotList;

/// A snapshot backend. Calls block; the applet runs them on a background task.
pub trait Backend {
    /// Lists snapshots on the backup device.
    ///
    /// # Errors
    ///
    /// When the backend can't be run, fails, or its output can't be parsed.
    fn list(&self) -> Result<SnapshotList>;

    /// Creates an on-demand snapshot. An empty comment means no comment.
    ///
    /// # Errors
    ///
    /// When the comment is invalid, or the backend can't be run or fails.
    fn create(&self, comment: &str) -> Result<()>;

    /// Deletes the snapshot called `name` (`YYYY-MM-DD_HH-MM-SS`).
    ///
    /// # Errors
    ///
    /// When the name is invalid, or the backend can't be run or fails.
    fn delete(&self, name: &str) -> Result<()>;
}
