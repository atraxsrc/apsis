// SPDX-License-Identifier: GPL-3.0-only

//! What the helper is doing: the one write lock (a job: create, delete, delete-many,
//! configure), the readers that share the backup device's read-only mount meanwhile, and when
//! the helper may exit.
//!
//! The rules (0.4.2):
//! - **Reads share, writes are exclusive.** Any number of readers (`List`) run at once; a
//!   write waits for them (up to [`WRITE_WAIT`]) instead of being refused.
//! - **Writer priority.** A reader that arrives while a write runs or waits is refused `Busy`:
//!   a refresh storm can't starve a write, and the end announcement brings the refresh.
//! - **Reads aren't jobs.** A [`Reading`] announces nothing and never shows in `Job()`.
//! - **The lock is released before the end is announced** ([`Running::end`]), so the refresh
//!   the announcement sets off is never refused by the job it refreshes for.
//! - **A ready restore plan isn't the lock** (6b.9). While a plan waits at the ready prompt
//!   the helper keeps it next to the lock: writes and `Stop` get `Busy`, reads go through, and
//!   `Job()` shows it as a running `restore` at 100%. It ends `stopped` (cancel, the disarm
//!   timer, the starter leaving the bus, too old) or `done` (right before the restart). The
//!   helper doesn't idle-exit while a plan is ready.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use apsis_core::job::{self, Job, JobKind, JobState, WireJob};
use apsis_core::native::Cancel;
use apsis_core::progress::Throttle;
use apsis_core::{Error, Progress, Result};
use tokio::sync::{Notify, mpsc, oneshot};

/// `JobChanged` for progress at most this often (the end always goes out).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// How long [`Announced::wait`] gives the announcing task to put the end on the bus before
/// `Finished` goes out anyway (a stuck bus mustn't hold the caller's result).
const ANNOUNCE_WAIT: Duration = Duration::from_secs(2);

/// One `JobChanged` for the announcing task: the job, and for an end, where to say once it's
/// on the bus (so `Finished` can follow it, not race it).
pub struct Announcement {
    pub job: WireJob,
    pub sent: Option<oneshot::Sender<()>>,
}

/// A job's end, on its way to the bus (see [`Running::end`]).
pub struct Announced(Option<oneshot::Receiver<()>>);

impl Announced {
    /// Returns once the end is on the bus, or after [`ANNOUNCE_WAIT`], or at once if the
    /// announcing task is gone. `Finished` is sent after this, so every listener hears the
    /// end before the caller hears the result.
    pub async fn wait(self) {
        if let Some(sent) = self.0 {
            let _ = tokio::time::timeout(ANNOUNCE_WAIT, sent).await;
        }
    }
}

/// How long a write waits for the readers to finish before it's refused `Busy`. A read is a
/// second or two (mount, a few `info.json` files, `statvfs`, unmount).
pub const WRITE_WAIT: Duration = Duration::from_secs(15);

/// A plan left at the ready prompt longer than this is too old to restart with (6b.5).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the restore methods (PLAN 6b.13 step 3, items 5 to 7) use it"
    )
)]
pub const READY_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// The job holding the lock, and what `Stop` needs about it.
struct Active {
    job: Job,
    /// A create's: how to stop it.
    cancel: Option<Arc<Cancel>>,
    /// The uid that started it; it may stop it without a password.
    starter: Option<u32>,
    throttle: Throttle,
}

/// The restore plan waiting at the ready prompt (see [`Running::ready`]).
struct ReadyPlan {
    job: Job,
    starter: u32,
    starter_name: String,
    prepared: tokio::time::Instant,
}

/// What's known about the ready plan: who may restart or cancel it without a password, and
/// how old it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyInfo {
    pub snapshot: String,
    /// The uid that prepared it.
    pub starter: u32,
    /// The unique bus name of the connection that prepared it: when it leaves the bus, the
    /// plan goes ([`State::starter_left`]).
    pub starter_name: String,
    pub prepared: tokio::time::Instant,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the restore methods (PLAN 6b.13 step 3, items 5 to 7) use it"
    )
)]
impl ReadyInfo {
    /// Older than [`READY_MAX_AGE`]: "The preparation is too old. Start the restore again."
    #[must_use]
    pub fn is_too_old(&self) -> bool {
        self.prepared.elapsed() > READY_MAX_AGE
    }
}

impl ReadyPlan {
    fn info(&self) -> ReadyInfo {
        ReadyInfo {
            snapshot: self.job.snapshot.clone(),
            starter: self.starter,
            starter_name: self.starter_name.clone(),
            prepared: self.prepared,
        }
    }
}

pub struct State {
    /// A write holds the lock, or waits for the readers to leave so it can.
    running: AtomicBool,
    /// Method calls in progress, including ones waiting for the polkit dialog, and finished
    /// operations still sending their `Finished` signal.
    calls: AtomicUsize,
    /// Wakes [`State::idle_for`] whenever any of the above changes.
    activity: Notify,
    job: Mutex<Option<Active>>,
    /// The restore plan at the ready prompt, next to the lock (never while a write runs: it's
    /// set from under the lock, and refuses every write until it ends).
    ready: Mutex<Option<ReadyPlan>>,
    /// Each change of the job, for the task that sends `JobChanged`.
    changes: mpsc::UnboundedSender<Announcement>,
    /// Readers in: [`Reading`] guards alive.
    readers: Mutex<usize>,
    /// Wakes the waiting write when the last reader leaves.
    readers_gone: Notify,
}

/// Why `Stop` can't go ahead.
fn refuse(why: &str) -> Error {
    Error::InvalidInput(why.to_owned())
}

impl State {
    /// The state, and the job changes for the task that sends `JobChanged`.
    pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<Announcement>) {
        let (changes, receiver) = mpsc::unbounded_channel();
        let state = Arc::new(Self {
            running: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            activity: Notify::new(),
            job: Mutex::new(None),
            ready: Mutex::new(None),
            changes,
            readers: Mutex::new(0),
            readers_gone: Notify::new(),
        });
        (state, receiver)
    }

    fn lock_job(&self) -> MutexGuard<'_, Option<Active>> {
        self.job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_readers(&self) -> MutexGuard<'_, usize> {
        self.readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_ready(&self) -> MutexGuard<'_, Option<ReadyPlan>> {
        self.ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sends the job as it is now (nobody listening is fine).
    fn announce(&self, active: &Active) {
        let _ = self.changes.send(Announcement {
            job: job::to_wire(Some(&active.job)),
            sent: None,
        });
    }

    /// Sends a job the caller's `Finished` follows (an end, or the ready plan), with a way to
    /// hear when it's on the bus.
    fn announce_then_finished(&self, job: &Job) -> Announced {
        let (sent, on_bus) = oneshot::channel();
        let announced = self.changes.send(Announcement {
            job: job::to_wire(Some(job)),
            sent: Some(sent),
        });
        Announced(announced.is_ok().then_some(on_bus))
    }

    /// Counts a call as in progress until the guard drops. The helper doesn't exit meanwhile.
    pub fn call(self: &Arc<Self>) -> Call {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.activity.notify_waiters();
        Call(Arc::clone(self))
    }

    /// A write holds the lock or waits for it.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Readers in right now.
    pub fn readers(&self) -> usize {
        *self.lock_readers()
    }

    /// Admits a reader until the [`Reading`] drops. Readers share the read-only mount and
    /// aren't jobs: nothing is announced.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] while a write runs or waits (writer priority).
    pub fn read(self: &Arc<Self>) -> Result<Reading> {
        let mut readers = self.lock_readers();
        // Under the readers lock, so a write that takes `running` sees this reader or refuses
        // it, never neither.
        if self.is_running() {
            return Err(Error::Busy);
        }
        *readers += 1;
        self.activity.notify_waiters();
        Ok(Reading(Arc::clone(self)))
    }

    /// Takes the write lock for a `kind` job until the [`Running`] drops or ends, and
    /// announces it. Every write to the backup device's mount point goes through it. Readers
    /// already in are waited for (new ones are refused meanwhile), up to [`WRITE_WAIT`].
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] if a write holds the lock (a second write is refused, not queued), or
    /// the readers didn't leave in time.
    pub async fn begin(self: &Arc<Self>, kind: JobKind) -> Result<Running> {
        // A plan can only become ready from under the lock, so once this passes none appears
        // while the write runs.
        if self.is_ready() {
            return Err(Error::Busy);
        }
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Error::Busy)?;
        self.activity.notify_waiters();
        if tokio::time::timeout(WRITE_WAIT, self.no_readers())
            .await
            .is_err()
        {
            self.running.store(false, Ordering::SeqCst);
            self.activity.notify_waiters();
            return Err(Error::Busy);
        }
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
        Ok(Running {
            state: Arc::clone(self),
            ended: false,
        })
    }

    /// Returns once no reader is in.
    async fn no_readers(&self) {
        loop {
            let gone = self.readers_gone.notified();
            let mut gone = std::pin::pin!(gone);
            // Registered before the check, so a reader leaving right after it still wakes us.
            gone.as_mut().enable();
            if *self.lock_readers() == 0 {
                return;
            }
            gone.await;
        }
    }

    /// The job holding the lock, else the ready plan (a running `restore` at 100%); `None`
    /// when idle.
    pub fn job(&self) -> Option<Job> {
        self.lock_job()
            .as_ref()
            .map(|a| a.job.clone())
            .or_else(|| self.lock_ready().as_ref().map(|p| p.job.clone()))
    }

    /// A restore plan waits at the ready prompt.
    pub fn is_ready(&self) -> bool {
        self.lock_ready().is_some()
    }

    /// The ready plan, if one waits.
    pub fn ready(&self) -> Option<ReadyInfo> {
        self.lock_ready().as_ref().map(ReadyPlan::info)
    }

    /// Takes the ready plan out (to cancel, disarm or restart with it); the caller removes
    /// its files and then ends it with [`Ready::end`]. Writes are admitted again from here.
    pub fn take_ready(self: &Arc<Self>) -> Option<Ready> {
        let plan = self.lock_ready().take()?;
        self.activity.notify_waiters();
        Some(Ready {
            state: Arc::clone(self),
            plan,
        })
    }

    /// The connection `name` left the bus: if it prepared the ready plan, the plan is taken
    /// out (the window is gone without an answer) for the caller to clean up and end
    /// `stopped`. `None` for any other name, or no plan.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the restore methods (PLAN 6b.13 step 3, items 5 to 7) use it"
        )
    )]
    pub fn starter_left(self: &Arc<Self>, name: &str) -> Option<Ready> {
        {
            let ready = self.lock_ready();
            if ready.as_ref().is_none_or(|p| p.starter_name != name) {
                return None;
            }
        }
        self.take_ready()
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

    /// A delete-many's next step: `snapshot` is being deleted and `done` of `total` are gone.
    /// Always announced (each step is a snapshot, seconds apart).
    pub fn step(&self, snapshot: &str, done: usize, total: usize) {
        if let Some(active) = self.lock_job().as_mut() {
            active.job.snapshot = snapshot.to_owned();
            active.job.percent = Some(percent(done, total));
            self.announce(active);
        }
    }

    /// What `Stop(snapshot)` would stop: the running create's (or restore preparation's)
    /// cancel and who started it.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] while a plan is ready (it's cancelled, not stopped).
    /// [`Error::InvalidInput`]: nothing runs, it isn't a create or a restore, or it's about
    /// another snapshot (or the name isn't known yet).
    pub fn stop_target(&self, snapshot: &str) -> Result<(Arc<Cancel>, Option<u32>)> {
        if self.is_ready() {
            return Err(Error::Busy);
        }
        let guard = self.lock_job();
        let Some(active) = guard.as_ref() else {
            return Err(refuse("nothing is running"));
        };
        if !matches!(active.job.kind, JobKind::Create | JobKind::Restore) {
            return Err(refuse("only a create or a restore can be stopped"));
        }
        if active.job.snapshot.is_empty() || active.job.snapshot != snapshot {
            return Err(refuse("the running job is about another snapshot"));
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

    /// Frees the write lock: the job is taken out and the next write may begin.
    fn release(&self) {
        *self.lock_job() = None;
        self.running.store(false, Ordering::SeqCst);
        self.activity.notify_waiters();
    }

    fn is_idle(&self) -> bool {
        !self.is_running()
            && !self.is_ready()
            && self.readers() == 0
            && self.calls.load(Ordering::SeqCst) == 0
    }

    /// Returns once nothing has happened for `idle` and nothing is in progress. Never while an
    /// operation runs, a plan is ready or a call is open, however long that takes.
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

/// `done` of `total` as a percentage; `total` of 0 is 100.
fn percent(done: usize, total: usize) -> f64 {
    if total == 0 {
        return 100.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "a handful of snapshots")]
    let fraction = done.min(total) as f64 / total as f64;
    fraction * 100.0
}

/// A call in progress (see [`State::call`]).
pub struct Call(Arc<State>);

impl Drop for Call {
    fn drop(&mut self) {
        self.0.calls.fetch_sub(1, Ordering::SeqCst);
        self.0.activity.notify_waiters();
    }
}

/// A reader in (see [`State::read`]): the read-only mount is shared until this drops.
pub struct Reading(Arc<State>);

impl Drop for Reading {
    fn drop(&mut self) {
        let mut readers = self.0.lock_readers();
        *readers = readers.saturating_sub(1);
        if *readers == 0 {
            self.0.readers_gone.notify_waiters();
        }
        self.0.activity.notify_waiters();
    }
}

/// The write lock (see [`State::begin`]). Ends with [`Running::end`]; dropping it unended
/// (a panic) frees the lock without an announcement.
pub struct Running {
    state: Arc<State>,
    ended: bool,
}

impl Running {
    /// The job ended: the lock is released **first**, then the end is announced once with
    /// `state` (done, failed or stopped). So the refresh the announcement sets off is never
    /// refused by this job. The caller sends `Finished` after [`Announced::wait`], so the
    /// end is on the bus before the result is: a listener that is also the caller sees its
    /// own job end before it hears how it went, never the other way round.
    pub fn end(mut self, state: JobState) -> Announced {
        let ended = self.state.lock_job().take().map(|mut active| {
            active.job.state = state;
            active
        });
        self.ended = true;
        self.state.release();
        match ended {
            Some(active) => self.state.announce_then_finished(&active.job),
            None => Announced(None),
        }
    }

    /// The restore's preparation is done: the plan moves next to the lock as the ready plan
    /// (a running job at 100%), the lock is released, and the plan is announced. The caller
    /// sends `Finished("restore", true, ..)` after [`Announced::wait`]. `starter` (uid) and
    /// `starter_name` (unique bus name) are who prepared it.
    pub fn ready(mut self, starter: u32, starter_name: &str) -> Announced {
        let plan = self.state.lock_job().take().map(|active| {
            let mut job = active.job;
            job.state = JobState::Running;
            job.percent = Some(100.0);
            job.eta_seconds = None;
            ReadyPlan {
                job,
                starter,
                starter_name: starter_name.to_owned(),
                prepared: tokio::time::Instant::now(),
            }
        });
        self.ended = true;
        let announced = plan.map(|plan| {
            let announced = self.state.announce_then_finished(&plan.job);
            *self.state.lock_ready() = Some(plan);
            announced
        });
        self.state.release();
        announced.unwrap_or(Announced(None))
    }
}

/// The ready plan, taken out of the state (see [`State::take_ready`]) to be ended.
pub struct Ready {
    state: Arc<State>,
    plan: ReadyPlan,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the restore methods (PLAN 6b.13 step 3, items 5 to 7) use it"
    )
)]
impl Ready {
    #[must_use]
    pub fn info(&self) -> ReadyInfo {
        self.plan.info()
    }

    /// The plan is over: `stopped` (cancelled, disarmed, the starter gone, too old) or `done`
    /// (right before the restart). Announced once.
    pub fn end(mut self, state: JobState) -> Announced {
        self.plan.job.state = state;
        self.state.activity.notify_waiters();
        self.state.announce_then_finished(&self.plan.job)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.ended {
            self.state.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn state() -> (Arc<State>, mpsc::UnboundedReceiver<Announcement>) {
        State::new()
    }

    pub(super) fn drain(
        changes: &mut mpsc::UnboundedReceiver<Announcement>,
    ) -> Vec<(String, String, String)> {
        let mut seen = Vec::new();
        while let Ok(Announcement {
            job: (kind, state, snapshot, ..),
            ..
        }) = changes.try_recv()
        {
            seen.push((kind, state, snapshot));
        }
        seen
    }

    /// The end's announcement waits for the bus before `Finished` may follow: the caller
    /// hears its own job end first. A stuck or gone announcer doesn't hold `Finished`.
    #[tokio::test(start_paused = true)]
    async fn finished_follows_the_end_on_the_bus() {
        let (state, mut changes) = state();
        let running = state.begin(JobKind::Delete).await.unwrap();
        drain(&mut changes);
        let announced = running.end(JobState::Done);
        let Announcement { job, sent } = changes.try_recv().unwrap();
        assert_eq!(job.1, "done");
        let sent = sent.expect("an end says when it's on the bus");
        let waiter = tokio::spawn(announced.wait());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_finished(), "Finished would race the end");
        sent.send(()).unwrap();
        waiter.await.unwrap();
        // A start or progress announcement has no such hook.
        let running = state.begin(JobKind::Create).await.unwrap();
        assert!(changes.try_recv().unwrap().sent.is_none());
        // The announcer dropped the sender (gone): no wait. Never sent: the timeout.
        let announced = running.end(JobState::Failed);
        drop(changes.try_recv().unwrap().sent);
        announced.wait().await;
        let running = state.begin(JobKind::Create).await.unwrap();
        drain(&mut changes);
        let announced = running.end(JobState::Done);
        let kept = changes.try_recv().unwrap().sent;
        let waiter = tokio::spawn(announced.wait());
        tokio::time::sleep(ANNOUNCE_WAIT + Duration::from_millis(1)).await;
        assert!(waiter.is_finished(), "gave up waiting");
        drop(kept);
    }

    #[tokio::test]
    async fn a_second_write_is_refused_not_queued() {
        let (state, _changes) = state();
        let running = state.begin(JobKind::Delete).await.unwrap();
        assert!(matches!(
            state.begin(JobKind::Create).await,
            Err(Error::Busy)
        ));
        running.end(JobState::Done);
        assert!(state.begin(JobKind::Delete).await.is_ok());
    }

    #[tokio::test]
    async fn readers_share_and_are_not_jobs() {
        let (state, mut changes) = state();
        // As many as displays plus a window, say five: all admitted, none refused.
        let readers: Vec<Reading> = (0..5).map(|_| state.read().unwrap()).collect();
        assert_eq!(state.readers(), 5);
        assert_eq!(state.job(), None, "a read never shows in Job()");
        assert!(drain(&mut changes).is_empty(), "no JobChanged for a read");
        drop(readers);
        assert_eq!(state.readers(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_write_waits_for_the_readers_in_and_refuses_new_ones() {
        let (state, mut changes) = state();
        // A job just ended and every process lists at once.
        let readers: Vec<Reading> = (0..5).map(|_| state.read().unwrap()).collect();
        // The next delete of a bulk delete arrives while they hold the mount.
        let write = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.begin(JobKind::Delete).await }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!write.is_finished(), "waits for the readers");
        assert!(state.is_running(), "and holds the lock meanwhile");
        // A sixth reader arriving while it waits is refused: writer priority.
        assert!(matches!(state.read(), Err(Error::Busy)));
        assert!(drain(&mut changes).is_empty(), "nothing announced yet");
        // The readers finish.
        drop(readers);
        let running = write.await.unwrap().unwrap();
        assert_eq!(state.job().unwrap().kind, JobKind::Delete);
        // A reader during a write is refused.
        assert!(matches!(state.read(), Err(Error::Busy)));
        running.end(JobState::Done);
        assert!(state.read().is_ok(), "free again once it ended");
    }

    #[tokio::test(start_paused = true)]
    async fn a_write_gives_up_on_readers_that_never_leave() {
        let (state, _changes) = state();
        let _stuck = state.read().unwrap();
        let write = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.begin(JobKind::Create).await }
        });
        tokio::time::sleep(WRITE_WAIT / 2).await;
        assert!(!write.is_finished());
        tokio::time::sleep(WRITE_WAIT).await;
        assert!(matches!(write.await.unwrap(), Err(Error::Busy)));
        assert!(!state.is_running(), "the lock is free again");
        assert_eq!(state.readers(), 1, "the reader is untouched");
    }

    #[tokio::test]
    async fn the_lock_is_free_before_the_end_is_announced() {
        let (state, mut changes) = state();
        let running = state.begin(JobKind::Delete).await.unwrap();
        state.named("2026-09-30_14-02-11");
        drain(&mut changes);
        running.end(JobState::Done);
        // The announcement is the last thing: by the time anyone hears it, a read goes
        // through and a write takes the lock.
        let seen = drain(&mut changes);
        assert_eq!(
            seen,
            [(
                "delete".to_owned(),
                "done".to_owned(),
                "2026-09-30_14-02-11".to_owned()
            )]
        );
        assert_eq!(state.job(), None);
        assert!(!state.is_running());
        let _reader = state.read().unwrap();
    }

    /// The bulk delete's shape: a job end, then every process lists at once and the window
    /// sends the next delete. Every read is answered and the write goes through.
    #[tokio::test(start_paused = true)]
    async fn a_job_end_then_reads_and_a_write_all_go_through() {
        let (state, _changes) = state();
        let first = state.begin(JobKind::Delete).await.unwrap();
        first.end(JobState::Done);
        let reads: Vec<_> = (0..3)
            .map(|_| {
                let reading = state.read().expect("a read right after the end");
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    drop(reading);
                })
            })
            .collect();
        let write = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.begin(JobKind::Delete).await }
        });
        for read in reads {
            read.await.unwrap();
        }
        let running = write.await.unwrap().expect("the write after the reads");
        running.end(JobState::Done);
        assert!(!state.is_running() && state.readers() == 0);
    }

    #[tokio::test]
    async fn a_create_is_announced_from_start_to_end() {
        let (state, mut changes) = state();
        assert_eq!(state.job(), None);
        let running = state.begin(JobKind::Create).await.unwrap();
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
        running.end(JobState::Done);
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

    #[tokio::test]
    async fn a_delete_of_several_announces_each_step() {
        let (state, mut changes) = state();
        let running = state.begin(JobKind::DeleteMany).await.unwrap();
        drain(&mut changes);
        state.step("2026-09-30_14-02-11", 0, 4);
        state.step("2026-09-30_14-02-12", 1, 4);
        let job = state.job().unwrap();
        assert_eq!(job.snapshot, "2026-09-30_14-02-12");
        assert_eq!(job.percent, Some(25.0));
        // Each step goes out, throttle or not.
        assert_eq!(drain(&mut changes).len(), 2);
        running.end(JobState::Failed);
        assert_eq!(
            drain(&mut changes),
            [(
                "delete-many".to_owned(),
                "failed".to_owned(),
                "2026-09-30_14-02-12".to_owned()
            )]
        );
        assert_eq!(percent(4, 4), 100.0);
        assert_eq!(percent(0, 0), 100.0);
    }

    #[tokio::test]
    async fn only_the_named_running_create_can_be_stopped() {
        let (state, mut changes) = state();
        assert!(state.stop_target("x").is_err(), "nothing runs");
        let delete = state.begin(JobKind::Delete).await.unwrap();
        assert!(state.stop_target("x").is_err(), "not a create");
        delete.end(JobState::Done);
        let _create = state.begin(JobKind::Create).await.unwrap();
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
    async fn idle_exit_waits_for_a_running_operation_and_for_readers() {
        let idle = Duration::from_secs(60);
        let (state, _changes) = state();
        let running = state.begin(JobKind::Create).await.unwrap();
        let waiter = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.idle_for(idle).await }
        });
        tokio::time::sleep(idle * 10).await;
        assert!(!waiter.is_finished(), "exited while a create was running");

        running.end(JobState::Done);
        let reader = state.read().unwrap();
        tokio::time::sleep(idle * 3).await;
        assert!(!waiter.is_finished(), "exited while a reader was in");
        drop(reader);
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

#[cfg(test)]
mod ready_tests {
    use super::tests::{drain, state};
    use super::*;

    const NAME: &str = "2026-09-30_14-02-11";
    const STARTER: &str = ":1.42";

    /// A restore prepared to the ready prompt: the lock held while preparing, then released
    /// with the plan kept next to it.
    async fn ready_plan(state: &Arc<State>) -> Announced {
        let running = state.begin(JobKind::Restore).await.unwrap();
        state.named(NAME);
        running.ready(1000, STARTER)
    }

    #[tokio::test]
    async fn a_ready_plan_refuses_writes_and_stop_but_not_reads() {
        let (state, mut changes) = state();
        ready_plan(&state).await;
        assert!(
            !state.is_running(),
            "the lock is free while a plan is ready"
        );
        assert!(state.is_ready());
        for kind in [
            JobKind::Create,
            JobKind::Delete,
            JobKind::DeleteMany,
            JobKind::Configure,
            JobKind::Restore,
        ] {
            assert!(
                matches!(state.begin(kind).await, Err(Error::Busy)),
                "{kind:?}"
            );
        }
        assert!(matches!(state.stop_target(NAME), Err(Error::Busy)));
        let _reader = state.read().expect("reads aren't refused");
        // The plan is a job: running, the snapshot's name, 100%.
        let job = state.job().unwrap();
        assert_eq!(job.kind, JobKind::Restore);
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.snapshot, NAME);
        assert_eq!(job.percent, Some(100.0));
        let seen = drain(&mut changes);
        assert_eq!(
            seen.last().unwrap(),
            &("restore".to_owned(), "running".to_owned(), NAME.to_owned())
        );
        let info = state.ready().unwrap();
        assert_eq!(info.snapshot, NAME);
        assert_eq!(info.starter, 1000);
        assert_eq!(info.starter_name, STARTER);
    }

    /// `Finished("restore", true)` follows the ready announcement on the bus, as an end does.
    #[tokio::test(start_paused = true)]
    async fn finished_follows_the_ready_announcement() {
        let (state, mut changes) = state();
        let announced = ready_plan(&state).await;
        let sent = drain_last_sent(&mut changes).expect("the ready announcement has a hook");
        let waiter = tokio::spawn(announced.wait());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_finished());
        sent.send(()).unwrap();
        waiter.await.unwrap();
    }

    fn drain_last_sent(
        changes: &mut mpsc::UnboundedReceiver<Announcement>,
    ) -> Option<oneshot::Sender<()>> {
        let mut last = None;
        while let Ok(Announcement { sent, .. }) = changes.try_recv() {
            last = sent;
        }
        last
    }

    #[tokio::test]
    async fn a_ready_plan_ends_stopped_or_done_and_frees_the_writes() {
        let (state, mut changes) = state();
        ready_plan(&state).await;
        drain(&mut changes);
        let ready = state.take_ready().expect("the plan");
        assert!(state.take_ready().is_none(), "taken once");
        ready.end(JobState::Stopped);
        assert_eq!(
            drain(&mut changes),
            [("restore".to_owned(), "stopped".to_owned(), NAME.to_owned())]
        );
        assert_eq!(state.job(), None);
        assert!(!state.is_ready());
        let running = state.begin(JobKind::Create).await.expect("writes again");
        running.end(JobState::Done);
        // Right before the restart: done.
        ready_plan(&state).await;
        drain(&mut changes);
        state.take_ready().unwrap().end(JobState::Done);
        assert_eq!(
            drain(&mut changes),
            [("restore".to_owned(), "done".to_owned(), NAME.to_owned())]
        );
    }

    #[tokio::test]
    async fn the_plan_goes_when_its_starter_leaves_the_bus() {
        let (state, _changes) = state();
        assert!(state.starter_left(STARTER).is_none(), "no plan");
        ready_plan(&state).await;
        assert!(state.starter_left(":1.43").is_none(), "another name");
        assert!(state.is_ready());
        let ready = state.starter_left(STARTER).expect("the starter's own");
        assert_eq!(ready.info().starter_name, STARTER);
        ready.end(JobState::Stopped);
        assert!(!state.is_ready());
    }

    #[tokio::test(start_paused = true)]
    async fn a_ready_plan_is_too_old_after_half_an_hour() {
        let (state, _changes) = state();
        ready_plan(&state).await;
        tokio::time::sleep(READY_MAX_AGE - Duration::from_secs(1)).await;
        assert!(!state.ready().unwrap().is_too_old());
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(state.ready().unwrap().is_too_old());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_waits_while_a_plan_is_ready() {
        let idle = Duration::from_secs(60);
        let (state, _changes) = state();
        ready_plan(&state).await;
        let waiter = tokio::spawn({
            let state = Arc::clone(&state);
            async move { state.idle_for(idle).await }
        });
        tokio::time::sleep(idle * 10).await;
        assert!(!waiter.is_finished(), "exited while a plan was ready");
        state.take_ready().unwrap().end(JobState::Stopped);
        tokio::time::sleep(idle * 2).await;
        assert!(waiter.is_finished());
    }

    /// Until ready, a restore's preparation is stopped like a create.
    #[tokio::test]
    async fn a_restore_preparation_can_be_stopped_until_ready() {
        let (state, _changes) = state();
        let _running = state.begin(JobKind::Restore).await.unwrap();
        state.stoppable(Cancel::new(), 1000);
        assert!(state.stop_target(NAME).is_err(), "name not known yet");
        state.named(NAME);
        let (_, starter) = state.stop_target(NAME).unwrap();
        assert_eq!(starter, Some(1000));
    }
}
