// SPDX-License-Identifier: GPL-3.0-only

//! What the helper is doing: at most one operation at a time, and when it may exit.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use apsis_core::{Error, Result};
use tokio::sync::Notify;

pub struct State {
    /// A [`Running`] exists: an operation holds the single-operation lock.
    running: AtomicBool,
    /// Method calls in progress, including ones waiting for the polkit dialog, and finished
    /// operations still sending their `Finished` signal.
    calls: AtomicUsize,
    /// Wakes [`State::idle_for`] whenever any of the above changes.
    activity: Notify,
}

impl State {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            activity: Notify::new(),
        })
    }

    /// Counts a call as in progress until the guard drops. The helper doesn't exit meanwhile.
    pub fn call(self: &Arc<Self>) -> Call {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.activity.notify_waiters();
        Call(Arc::clone(self))
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Takes the single-operation lock until the [`Running`] drops. Every use of the backup
    /// device's mount point goes through it.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] if an operation holds it: a second call is refused, not queued.
    pub fn begin(self: &Arc<Self>) -> Result<Running> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Error::Busy)?;
        self.activity.notify_waiters();
        Ok(Running(Arc::clone(self)))
    }

    fn is_idle(&self) -> bool {
        !self.is_running() && self.calls.load(Ordering::SeqCst) == 0
    }

    /// Returns once nothing has happened for `idle` and nothing is in progress. Never while an
    /// operation runs or a call is open, however long that takes.
    pub async fn idle_for(&self, idle: Duration) {
        loop {
            let activity = self.activity.notified();
            let mut activity = std::pin::pin!(activity);
            // Registered before the check below, so a change right after it still wakes us.
            activity.as_mut().enable();
            if tokio::time::timeout(idle, activity).await.is_err() && self.is_idle() {
                return;
            }
        }
    }
}

/// A call in progress (see [`State::call`]).
pub struct Call(Arc<State>);

impl Drop for Call {
    fn drop(&mut self) {
        self.0.calls.fetch_sub(1, Ordering::SeqCst);
        self.0.activity.notify_waiters();
    }
}

/// The single-operation lock (see [`State::begin`]).
pub struct Running(Arc<State>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
        self.0.activity.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_operation_is_refused_not_queued() {
        let state = State::new();
        let running = state.begin().unwrap();
        assert!(matches!(state.begin(), Err(Error::Busy)));
        drop(running);
        assert!(state.begin().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_waits_for_a_running_operation() {
        let idle = Duration::from_secs(60);
        let state = State::new();
        let running = state.begin().unwrap();
        let waiter = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.idle_for(idle).await }
        });
        tokio::time::sleep(idle * 10).await;
        assert!(!waiter.is_finished(), "exited while Timeshift was running");

        drop(running);
        tokio::time::sleep(idle / 2).await;
        assert!(!waiter.is_finished(), "exited before a full idle period");
        tokio::time::sleep(idle).await;
        assert!(waiter.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_waits_for_open_calls() {
        let idle = Duration::from_secs(60);
        let state = State::new();
        let call = state.call();
        let waiter = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.idle_for(idle).await }
        });
        tokio::time::sleep(idle * 3).await;
        assert!(!waiter.is_finished(), "exited during a call");
        drop(call);
        tokio::time::sleep(idle * 2).await;
        assert!(waiter.is_finished());
    }
}
