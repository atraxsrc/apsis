// SPDX-License-Identifier: GPL-3.0-only

//! Stopping a create while it runs.
//!
//! A [`Cancel`] goes with one create. [`Cancel::request`] asks it to stop: rsync's process
//! group gets `SIGTERM`, and `SIGKILL` after a grace period if its leader hasn't exited. The
//! create checks it before rsync starts and after it ends, and [`Cancel::commit`]s right
//! before it renames the finished snapshot into `snapshots/`: whichever of `request` and
//! `commit` comes first wins, so a stop can never race the rename.
//!
//! The group's id is rsync's pid. It is only signalled while that process is known not to be
//! reaped yet: the runner first waits with `WNOWAIT` (the process has exited but stays a
//! zombie, so its pid and group id can't be reused), marks the group gone under the same lock
//! the signals are sent under ([`Cancel::exited`]), and only then reaps it.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process_group};

/// How long rsync gets to end after `SIGTERM`, before `SIGKILL`.
pub const GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Armed,
    Stopping,
    Committed,
}

#[derive(Debug)]
struct Inner {
    phase: Phase,
    /// rsync's process group while its leader isn't reaped.
    group: Option<Pid>,
}

/// Whether a create may still be stopped (see the module docs).
#[derive(Debug)]
pub struct Cancel {
    inner: Mutex<Inner>,
    grace: Duration,
}

/// [`Cancel::request`] came after [`Cancel::commit`]: the snapshot is being finished and is
/// kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLate;

impl Cancel {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Self::with_grace(GRACE)
    }

    /// With another grace period before `SIGKILL` (the tests use a short one).
    #[must_use]
    pub fn with_grace(grace: Duration) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                phase: Phase::Armed,
                group: None,
            }),
            grace,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding it leaves plain data behind; go on with it.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Asks the create to stop. rsync's group (if it runs) gets `SIGTERM` now and `SIGKILL`
    /// after the grace period if it's still there. Asking twice is fine.
    ///
    /// # Errors
    ///
    /// [`TooLate`] after [`Cancel::commit`].
    pub fn request(self: &Arc<Self>) -> Result<(), TooLate> {
        let mut inner = self.lock();
        match inner.phase {
            Phase::Committed => return Err(TooLate),
            Phase::Stopping => return Ok(()),
            Phase::Armed => inner.phase = Phase::Stopping,
        }
        if let Some(group) = inner.group {
            let _ = kill_process_group(group, Signal::TERM);
            let this = Arc::clone(self);
            drop(inner);
            thread::spawn(move || {
                thread::sleep(this.grace);
                this.kill();
            });
        }
        Ok(())
    }

    /// Whether a stop was asked for.
    #[must_use]
    pub fn is_stopping(&self) -> bool {
        self.lock().phase == Phase::Stopping
    }

    /// Right before the finished snapshot is renamed into place: from now on it can't be
    /// stopped. `false` if a stop came first (the create must stop instead).
    #[must_use]
    pub fn commit(&self) -> bool {
        let mut inner = self.lock();
        match inner.phase {
            Phase::Stopping => false,
            Phase::Armed | Phase::Committed => {
                inner.phase = Phase::Committed;
                true
            }
        }
    }

    /// The runner started rsync as the leader of process group `group`. If a stop was asked
    /// for already, it gets `SIGTERM` at once.
    pub fn started(self: &Arc<Self>, group: Pid) {
        let mut inner = self.lock();
        inner.group = Some(group);
        if inner.phase == Phase::Stopping {
            let _ = kill_process_group(group, Signal::TERM);
            let this = Arc::clone(self);
            drop(inner);
            thread::spawn(move || {
                thread::sleep(this.grace);
                this.kill();
            });
        }
    }

    /// The group's leader has exited and is about to be reaped: no more signals to it.
    pub fn exited(&self) {
        self.lock().group = None;
    }

    /// `SIGKILL` to the group, if its leader isn't reaped yet.
    fn kill(&self) {
        let inner = self.lock();
        if let Some(group) = inner.group {
            let _ = kill_process_group(group, Signal::KILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_and_commit_exclude_each_other() {
        let cancel = Cancel::new();
        assert!(cancel.commit());
        assert_eq!(cancel.request(), Err(TooLate));
        assert!(!cancel.is_stopping());

        let cancel = Cancel::new();
        assert_eq!(cancel.request(), Ok(()));
        assert_eq!(cancel.request(), Ok(()), "twice is fine");
        assert!(cancel.is_stopping());
        assert!(!cancel.commit());
    }
}
