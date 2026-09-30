// SPDX-License-Identifier: GPL-3.0-only

//! What the helper is doing: at most one operation at a time (the one lock), the job that
//! holds it (for `Job` and `JobChanged`), and when the helper may exit.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use apsis_core::job::{self, Job, JobKind, JobState, WireJob};
use apsis_core::native::Cancel;
use apsis_core::progress::Throttle;
use apsis_core::{Error, Progress, Result};
use tokio::sync::{Notify, mpsc};

/// `JobChanged` for progress at most this often (the end always goes out).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// The job holding the lock, and what `Stop` needs about it.
struct Active {
    job: Job,
    /// A create's: how to stop it.
    cancel: Option<Arc<Cancel>>,
    /// The uid that started it; it may stop it without a password.
    starter: Option<u32>,
    throttle: Throttle,
}

pub struct State {
    /// A [`Running`] exists: an operation holds the single-operation lock.
    running: AtomicBool,
    /// Method calls in progress, including ones waiting for the polkit dialog, and finished
    /// operations still sending their `Finished` signal.
    calls: AtomicUsize,
    /// Wakes [`State::idle_for`] whenever any of the above changes.
    activity: Notify,
    job: Mutex<Option<Active>>,
    /// Each change of the job, for the task that sends `JobChanged`.
    changes: mpsc::UnboundedSender<WireJob>,
}

/// Why `Stop` can't go ahead.
fn refuse(why: &str) -> Error {
    Error::InvalidInput(why.to_owned())
}

impl State {
    /// The state, and the job changes for the task that sends `JobChanged`.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<WireJob>) {
        let (changes, receiver) = mpsc::unbounded_channel();
        let state = Arc::new(Self {
            running: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            activity: Notify::new(),
            job: Mutex::new(None),
            changes,
        });
        (state, receiver)
    }

    fn lock_job(&self) -> MutexGuard<'_, Option<Active>> {
        self.job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sends the job as it is now (nobody listening is fine).
    fn announce(&self, active: &Active) {
        let _ = self.changes.send(job::to_wire(Some(&active.job)));
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

    /// Takes the single-operation lock for a `kind` job until the [`Running`] drops, and
    /// announces it. Every use of the backup device's mount point goes through it.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] if an operation holds it: a second call is refused, not queued.
    pub fn begin(self: &Arc<Self>, kind: JobKind) -> Result<Running> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Error::Busy)?;
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        let active = Active {
            job: Job::new(kind, started),
            cancel: None,
            starter: None,
            throttle: Throttle::new(PROGRESS_INTERVAL),
        };
        self.announce(&active);
        *self.lock_job() = Some(active);
        self.activity.notify_waiters();
        Ok(Running(Arc::clone(self)))
    }

    /// The job holding the lock; `None` when idle.
    pub fn job(&self) -> Option<Job> {
        self.lock_job().as_ref().map(|a| a.job.clone())
    }

    /// The running create can be stopped with `cancel`; `starter` started it.
    pub fn stoppable(&self, cancel: Arc<Cancel>, starter: u32) {
        if let Some(active) = self.lock_job().as_mut() {
            active.cancel = Some(cancel);
            active.starter = Some(starter);
        }
    }

    /// The running create's snapshot name is known.
    pub fn named(&self, snapshot: &str) {
        if let Some(active) = self.lock_job().as_mut() {
            active.job.snapshot = snapshot.to_owned();
            self.announce(active);
        }
    }

    /// How far the job is: announced at most every [`PROGRESS_INTERVAL`], `100%` always.
    pub fn progress(&self, progress: &Progress) {
        if let Some(active) = self.lock_job().as_mut() {
            active.job.percent = progress.percent;
            active.job.eta_seconds = progress.eta_seconds;
            let done = progress.percent.is_some_and(|p| p >= 100.0);
            if active.throttle.ready(Instant::now()) || done {
                self.announce(active);
            }
        }
    }

    /// What `Stop(snapshot)` would stop: the running create's cancel and who started it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`]: nothing runs, it isn't a create, or it's making another
    /// snapshot (or its name isn't known yet).
    pub fn stop_target(&self, snapshot: &str) -> Result<(Arc<Cancel>, Option<u32>)> {
        let guard = self.lock_job();
        let Some(active) = guard.as_ref() else {
            return Err(refuse("nothing is running"));
        };
        if active.job.kind != JobKind::Create {
            return Err(refuse("only a create can be stopped"));
        }
        if active.job.snapshot.is_empty() || active.job.snapshot != snapshot {
            return Err(refuse("the running create is making another snapshot"));
        }
        let cancel = active
            .cancel
            .clone()
            .ok_or_else(|| refuse("this create can't be stopped"))?;
        Ok((cancel, active.starter))
    }

    /// The running create is being stopped.
    pub fn stopping(&self) {
        if let Some(active) = self.lock_job().as_mut() {
            active.job.state = JobState::Stopping;
            self.announce(active);
        }
    }

    /// The job ended: announced once with `state` (done, failed or stopped). The lock is
    /// released when its [`Running`] drops.
    pub fn end(&self, state: JobState) {
        if let Some(active) = self.lock_job().as_mut() {
            active.job.state = state;
            self.announce(active);
        }
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
        *self.0.lock_job() = None;
        self.0.running.store(false, Ordering::SeqCst);
        self.0.activity.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (Arc<State>, mpsc::UnboundedReceiver<WireJob>) {
        State::new()
    }

    fn drain(changes: &mut mpsc::UnboundedReceiver<WireJob>) -> Vec<(String, String, String)> {
        let mut seen = Vec::new();
        while let Ok((kind, state, snapshot, ..)) = changes.try_recv() {
            seen.push((kind, state, snapshot));
        }
        seen
    }

    #[test]
    fn a_second_operation_is_refused_not_queued() {
        let (state, _changes) = state();
        let running = state.begin(JobKind::List).unwrap();
        assert!(matches!(state.begin(JobKind::Create), Err(Error::Busy)));
        drop(running);
        assert!(state.begin(JobKind::Delete).is_ok());
    }

    #[test]
    fn a_create_is_announced_from_start_to_end() {
        let (state, mut changes) = state();
        assert_eq!(state.job(), None);
        let running = state.begin(JobKind::Create).unwrap();
        state.named("2026-09-30_14-02-11");
        state.progress(&Progress {
            percent: Some(10.0),
            eta_seconds: Some(60),
            text: String::new(),
        });
        // Within the throttle: kept, not announced; the end always is.
        state.progress(&Progress {
            percent: Some(11.0),
            eta_seconds: None,
            text: String::new(),
        });
        assert_eq!(state.job().unwrap().percent, Some(11.0));
        state.end(JobState::Done);
        drop(running);
        assert_eq!(state.job(), None);
        let running = |s: &str| ("create".to_owned(), s.to_owned());
        let seen: Vec<(String, String)> = drain(&mut changes)
            .into_iter()
            .map(|(kind, state, _)| (kind, state))
            .collect();
        assert_eq!(
            seen,
            [
                running("running"),
                running("running"),
                running("running"),
                running("done")
            ]
        );
    }

    #[test]
    fn only_the_named_running_create_can_be_stopped() {
        let (state, mut changes) = state();
        assert!(state.stop_target("x").is_err(), "nothing runs");
        let list = state.begin(JobKind::List).unwrap();
        assert!(state.stop_target("x").is_err(), "not a create");
        drop(list);
        let _create = state.begin(JobKind::Create).unwrap();
        state.stoppable(Cancel::new(), 1000);
        let name = "2026-09-30_14-02-11";
        assert!(state.stop_target(name).is_err(), "name not known yet");
        state.named(name);
        assert!(
            state.stop_target("2026-09-30_14-02-12").is_err(),
            "another one"
        );
        let (_, starter) = state.stop_target(name).unwrap();
        assert_eq!(starter, Some(1000));
        state.stopping();
        assert_eq!(state.job().unwrap().state, JobState::Stopping);
        assert!(
            drain(&mut changes)
                .iter()
                .any(|(_, s, n)| s == "stopping" && n == name)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_waits_for_a_running_operation() {
        let idle = Duration::from_secs(60);
        let (state, _changes) = state();
        let running = state.begin(JobKind::Create).unwrap();
        let waiter = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.idle_for(idle).await }
        });
        tokio::time::sleep(idle * 10).await;
        assert!(!waiter.is_finished(), "exited while a create was running");

        drop(running);
        tokio::time::sleep(idle / 2).await;
        assert!(!waiter.is_finished(), "exited before a full idle period");
        tokio::time::sleep(idle).await;
        assert!(waiter.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_waits_for_open_calls() {
        let idle = Duration::from_secs(60);
        let (state, _changes) = state();
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
