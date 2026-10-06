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
//! - **A ready restore plan isn't the lock**. While a plan waits at the ready prompt
//!   the helper keeps it next to the lock: writes and `Stop` get `Busy`, reads go through, and
//!   `Job()` shows it as a running `restore` at 100%. It ends `stopped` (cancel, the disarm
//!   timer, the starter leaving the bus, too old) or `done` (right before the restart). The
//!   helper doesn't idle-exit while a plan is ready.
//! - **The job lock on disk** (0.5.0). A write also holds an exclusive `flock` on
//!   [`JOB_LOCK`] from its start to its end ([`JobLock`]). The package's `prerm` takes the
//!   same lock without waiting: it refuses while a job runs, and while it holds the lock a
//!   write is `Busy` before anything is announced. Reads hold none, and neither does a plan
//!   waiting at the prompt.
//! - **A ready plan ends where it is.** [`State::take_ready`] marks it and takes the job
//!   lock on disk in one step; it leaves its slot only at [`Ready::end`]. So from "Restart
//!   now" to the link, and while a cancelled plan's files go, every write is still `Busy`
//!   and a package script is refused.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use apsis_core::job::{self, Job, JobKind, JobState, WireJob};
use apsis_core::native::Cancel;
use apsis_core::progress::Throttle;
use apsis_core::{Error, Progress, Result};
use rustix::fs::{FlockOperation, Mode, OFlags};
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

/// The job lock on disk: the package's `prerm` takes it too, so the path is
/// spelled the same in `resources/deb/prerm`. On tmpfs: a restart clears it. Nobody removes
/// it, and its being there means nothing; only the `flock` on it does.
pub const JOB_LOCK: &str = "/run/apsis/job.lock";

/// The job lock on disk, held: an exclusive `flock` on the file until this drops.
struct JobLock(OwnedFd);

impl JobLock {
    /// Takes the lock on `path` without waiting. `None`: someone else holds it (the
    /// package's `prerm`).
    ///
    /// The file is made if it isn't there, root's alone, with its folder. It's opened
    /// `O_NOFOLLOW`, so a symlink at its name is refused and never followed, and
    /// `O_CLOEXEC`, so nothing the helper starts (rsync, `mount`, `systemd-run`) inherits
    /// it: the lock dies with the helper, not with its children. Once the lock is taken the
    /// open file must still be the one at the path, or the file was removed and made again
    /// meanwhile and a script would lock the new one and find it free; then it's opened
    /// once more.
    ///
    /// Before the lock is asked for, the open file must be `owner`'s (root's; the tests'
    /// own uid in the tests): anyone else's is refused and left as it is. And it must be
    /// its owner's alone: `flock` works on a read-only descriptor, so a file others can
    /// open lets any user hold the lock and make every job `Busy` and every package
    /// operation refuse. `O_CREAT` doesn't change the mode of a file that's already there
    /// (one the `prerm` of another build made, say), so any other mode is set to 0600 here.
    fn take(path: &Path, owner: u32) -> io::Result<Option<Self>> {
        if let Some(folder) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o755)
                .create(folder)?;
        }
        for _ in 0..2 {
            let file = rustix::fs::open(
                path,
                OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_raw_mode(0o600),
            )?;
            let found = rustix::fs::fstat(&file)?;
            if found.st_uid != owner {
                return Err(io::Error::other(format!(
                    "it belongs to uid {}, not to uid {owner}",
                    found.st_uid
                )));
            }
            if found.st_mode & 0o7777 != 0o600 {
                rustix::fs::fchmod(&file, Mode::from_raw_mode(0o600))?;
            }
            match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => {}
                Err(rustix::io::Errno::WOULDBLOCK) => return Ok(None),
                Err(error) => return Err(error.into()),
            }
            let lock = Self(file);
            let ours = rustix::fs::fstat(&lock.0)?;
            if rustix::fs::lstat(path)
                .is_ok_and(|there| there.st_dev == ours.st_dev && there.st_ino == ours.st_ino)
            {
                return Ok(Some(lock));
            }
        }
        Err(io::Error::other(
            "the file was replaced each time the lock was taken",
        ))
    }
}

impl Drop for JobLock {
    /// Unlocked, not only closed: a child forked a moment ago has a copy of the descriptor
    /// until it execs, and a lock that waited for that copy to close would refuse the next
    /// job.
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, FlockOperation::Unlock);
    }
}

/// How long a write waits for the readers to finish before it's refused `Busy`. A read is a
/// second or two (mount, a few `info.json` files, `statvfs`, unmount).
pub const WRITE_WAIT: Duration = Duration::from_secs(15);

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
    /// Taken to be ended ([`State::take_ready`]): nobody else gets it.
    taken: bool,
}

/// What's known about the ready plan: who may restart or cancel it without a password. Its
/// age is `request.json`'s to say (`Plan::is_too_old`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyInfo {
    pub snapshot: String,
    /// The uid that prepared it.
    pub starter: u32,
    /// The unique bus name of the connection that prepared it: when it leaves the bus, the
    /// plan goes ([`State::starter_left`]).
    pub starter_name: String,
}

impl ReadyPlan {
    fn info(&self) -> ReadyInfo {
        ReadyInfo {
            snapshot: self.job.snapshot.clone(),
            starter: self.starter,
            starter_name: self.starter_name.clone(),
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
    /// The job lock's file on disk ([`JOB_LOCK`], or a temp file in the tests).
    job_lock: PathBuf,
    /// The uid the job lock's file must belong to: root, or the tests' own.
    lock_owner: u32,
}

/// Why `Stop` can't go ahead.
fn refuse(why: &str) -> Error {
    Error::InvalidInput(why.to_owned())
}

impl State {
    /// The state, and the job changes for the task that sends `JobChanged`. `job_lock` is
    /// the job lock's file.
    pub fn new(
        job_lock: impl Into<PathBuf>,
        lock_owner: u32,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Announcement>) {
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
            job_lock: job_lock.into(),
            lock_owner,
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

    /// Takes the job lock on disk, for a job or for a plan that's being ended.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`]: a package script holds it, or it couldn't be taken (a symlink at
    /// its name, say). The journal says which.
    fn take_job_lock(&self) -> Result<JobLock> {
        let file = self.job_lock.display();
        match JobLock::take(&self.job_lock, self.lock_owner) {
            Ok(Some(lock)) => Ok(lock),
            Ok(None) => {
                eprintln!("apsis-helper: the job lock {file} is held: a package script runs");
                Err(Error::Busy)
            }
            Err(error) => {
                eprintln!("apsis-helper: the job lock {file} couldn't be taken: {error}");
                Err(Error::Busy)
            }
        }
    }

    /// Takes the write lock for a `kind` job until the [`Running`] drops or ends, and
    /// announces it. Every write to the backup device's mount point goes through it. Readers
    /// already in are waited for (new ones are refused meanwhile), up to [`WRITE_WAIT`]. The
    /// job lock on disk is held as long, so a package script is refused while the job runs.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] if a write holds the lock (a second write is refused, not queued),
    /// a package script holds the job lock on disk, or the readers didn't leave in time.
    pub async fn begin(self: &Arc<Self>, kind: JobKind) -> Result<Running> {
        // A plan can only become ready from under the lock, so once this passes none appears
        // while the write runs.
        if self.is_ready() {
            return Err(Error::Busy);
        }
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Error::Busy)?;
        // Before anything is announced: a job refused by a package script was never a job.
        let lock = match self.take_job_lock() {
            Ok(lock) => lock,
            Err(error) => {
                self.running.store(false, Ordering::SeqCst);
                self.activity.notify_waiters();
                return Err(error);
            }
        };
        self.activity.notify_waiters();
        if tokio::time::timeout(WRITE_WAIT, self.no_readers())
            .await
            .is_err()
        {
            drop(lock);
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
            lock: Some(lock),
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

    /// Takes the ready plan to end it (to cancel it, or to restart with it): the plan is
    /// marked where it is and the job lock on disk is taken, in one step. The caller arms
    /// with it or removes its files, then ends it with [`Ready::end`]. Until then the plan
    /// stays in its slot: every write is still `Busy`, reads go on, `Job()` still shows it,
    /// and a package script is refused. `None`: no
    /// plan waits.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`]: the plan is taken already, or a package script holds the job lock
    /// on disk. The plan stays as it was.
    pub fn take_ready(self: &Arc<Self>) -> Result<Option<Ready>> {
        self.take_plan(None)
    }

    /// The connection `name` left the bus: if it prepared the ready plan, the plan is taken
    /// as [`State::take_ready`] takes it (the window is gone without an answer), for the
    /// caller to clean up and end `stopped`. `None` for any other name, for no plan, and
    /// for a plan that's taken already (whoever took it ends it).
    ///
    /// # Errors
    ///
    /// [`Error::Busy`]: a package script holds the job lock on disk. The plan stays ready
    /// (the script stops the helper next, and the next start removes the plan's files).
    pub fn starter_left(self: &Arc<Self>, name: &str) -> Result<Option<Ready>> {
        self.take_plan(Some(name))
    }

    /// [`State::take_ready`], or with `starter` [`State::starter_left`]. The mark and the
    /// job lock on disk are both under the slot's mutex.
    fn take_plan(self: &Arc<Self>, starter: Option<&str>) -> Result<Option<Ready>> {
        let mut ready = self.lock_ready();
        let Some(plan) = ready.as_mut() else {
            return Ok(None);
        };
        match starter {
            Some(name) if plan.taken || plan.starter_name != name => return Ok(None),
            None if plan.taken => return Err(Error::Busy),
            _ => {}
        }
        let lock = self.take_job_lock()?;
        plan.taken = true;
        let info = plan.info();
        drop(ready);
        self.activity.notify_waiters();
        Ok(Some(Ready {
            state: Arc::clone(self),
            info,
            lock: Some(lock),
            ended: false,
        }))
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
    /// The job lock on disk: held until the job ends, or until its plan is ready.
    lock: Option<JobLock>,
}

impl Running {
    /// The job ended: the lock is released **first** (the one on disk with it), then the
    /// end is announced once with `state` (done, failed or stopped). So the refresh the
    /// announcement sets off is never refused by this job. The caller sends `Finished` after
    /// [`Announced::wait`], so the
    /// end is on the bus before the result is: a listener that is also the caller sees its
    /// own job end before it hears how it went, never the other way round.
    pub fn end(mut self, state: JobState) -> Announced {
        let ended = self.state.lock_job().take().map(|mut active| {
            active.job.state = state;
            active
        });
        self.ended = true;
        self.lock = None;
        self.state.release();
        match ended {
            Some(active) => self.state.announce_then_finished(&active.job),
            None => Announced(None),
        }
    }

    /// The restore's preparation is done: the plan moves next to the lock as the ready plan
    /// (a running job at 100%), the lock is released, and the plan is announced. The caller
    /// sends `Finished("restore", true, ..)` after [`Announced::wait`]. `starter` (uid) and
    /// `starter_name` (unique bus name) are who prepared it. A ready plan holds no job lock
    /// on disk (an upgrade at the prompt goes on): it's released before the plan is in its
    /// slot, so whoever takes the plan next finds it free.
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
                taken: false,
            }
        });
        self.ended = true;
        self.lock = None;
        let announced = plan.map(|plan| {
            let announced = self.state.announce_then_finished(&plan.job);
            *self.state.lock_ready() = Some(plan);
            announced
        });
        self.state.release();
        announced.unwrap_or(Announced(None))
    }
}

/// The ready plan, taken to be ended (see [`State::take_ready`]): marked in its slot, with
/// the job lock on disk. Ends with [`Ready::end`]; dropping it unended (a panic) takes the
/// plan out and frees the lock without an announcement.
pub struct Ready {
    state: Arc<State>,
    info: ReadyInfo,
    /// The job lock on disk: held until the plan ends.
    lock: Option<JobLock>,
    ended: bool,
}

impl Ready {
    #[must_use]
    pub fn info(&self) -> ReadyInfo {
        self.info.clone()
    }

    /// The plan is over: `stopped` (cancelled, disarmed, the starter gone, too old) or `done`
    /// (right before the restart). The plan leaves its slot, then the job lock on disk is
    /// released, then the end is announced, once: writes are admitted again from here.
    pub fn end(mut self, state: JobState) -> Announced {
        let plan = self.state.lock_ready().take();
        self.lock = None;
        self.ended = true;
        self.state.activity.notify_waiters();
        match plan {
            Some(mut plan) => {
                plan.job.state = state;
                self.state.announce_then_finished(&plan.job)
            }
            None => Announced(None),
        }
    }
}

impl Drop for Ready {
    fn drop(&mut self) {
        if !self.ended {
            *self.state.lock_ready() = None;
            self.lock = None;
            self.state.activity.notify_waiters();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.ended {
            self.lock = None;
            self.state.release();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use super::*;

    /// A state with a job lock of its own: a file in a temp folder that goes when this drops.
    pub(crate) struct TestState {
        state: Arc<State>,
        folder: PathBuf,
    }

    impl TestState {
        /// The job lock's file. `begin` makes it, with its folder.
        pub(crate) fn lock(&self) -> PathBuf {
            self.folder.join("job.lock")
        }
    }

    impl std::ops::Deref for TestState {
        type Target = Arc<State>;

        fn deref(&self) -> &Arc<State> {
            &self.state
        }
    }

    impl Drop for TestState {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    pub(crate) fn state() -> (TestState, mpsc::UnboundedReceiver<Announcement>) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let folder = std::env::temp_dir().join(format!(
            "apsis-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&folder);
        let (state, changes) =
            State::new(folder.join("job.lock"), rustix::process::geteuid().as_raw());
        (TestState { state, folder }, changes)
    }

    /// What the package's `prerm` asks, with the real tool: `flock -n -E 75 <file> true`.
    /// 75: someone holds the lock. 0: it's free.
    pub(crate) fn flock_exit(lock: &Path) -> i32 {
        std::process::Command::new("flock")
            .args(["-n", "-E", "75"])
            .arg(lock)
            .arg("true")
            .status()
            .expect("flock(1), from util-linux")
            .code()
            .expect("an exit status")
    }

    /// The lock held as the `prerm` holds it: through another open file.
    pub(crate) struct Held(OwnedFd);

    impl Drop for Held {
        /// Unlocked, not only closed: a child that another test's thread forked a moment
        /// ago still has a copy of the descriptor until it execs.
        fn drop(&mut self) {
            let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
        }
    }

    /// Holds the lock as the `prerm` does until the result drops.
    pub(crate) fn hold(lock: &Path) -> Held {
        use rustix::fs::{FlockOperation, Mode, OFlags};
        std::fs::create_dir_all(lock.parent().expect("a folder")).unwrap();
        let file = rustix::fs::open(
            lock,
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o644),
        )
        .unwrap();
        rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive).unwrap();
        Held(file)
    }

    pub(crate) fn drain(
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
        let ready = state.take_ready().unwrap().expect("the plan");
        assert!(matches!(state.take_ready(), Err(Error::Busy)), "taken once");
        ready.end(JobState::Stopped);
        assert!(matches!(state.take_ready(), Ok(None)), "and gone");
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
        state.take_ready().unwrap().unwrap().end(JobState::Done);
        assert_eq!(
            drain(&mut changes),
            [("restore".to_owned(), "done".to_owned(), NAME.to_owned())]
        );
    }

    #[tokio::test]
    async fn the_plan_goes_when_its_starter_leaves_the_bus() {
        let (state, _changes) = state();
        assert!(matches!(state.starter_left(STARTER), Ok(None)), "no plan");
        ready_plan(&state).await;
        assert!(
            matches!(state.starter_left(":1.43"), Ok(None)),
            "another name"
        );
        assert!(state.is_ready());
        let ready = state
            .starter_left(STARTER)
            .unwrap()
            .expect("the starter's own");
        assert_eq!(ready.info().starter_name, STARTER);
        ready.end(JobState::Stopped);
        assert!(!state.is_ready());
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
        state.take_ready().unwrap().unwrap().end(JobState::Stopped);
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

/// The job lock on disk: what a package
/// script's `flock -n` finds, and what the helper does while a script holds the lock.
#[cfg(test)]
mod lock_tests {
    use std::io::{BufRead, BufReader};
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::tests::{drain, flock_exit, hold, state};
    use super::*;

    const NAME: &str = "2026-09-30_14-02-11";
    const STARTER: &str = ":1.42";

    const WRITES: [JobKind; 5] = [
        JobKind::Create,
        JobKind::Delete,
        JobKind::DeleteMany,
        JobKind::Configure,
        JobKind::Restore,
    ];

    async fn ready_plan(state: &Arc<State>) {
        let running = state.begin(JobKind::Restore).await.unwrap();
        state.named(NAME);
        running.ready(1000, STARTER);
    }

    async fn every_write_is_busy(state: &Arc<State>) {
        for kind in WRITES {
            assert!(
                matches!(state.begin(kind).await, Err(Error::Busy)),
                "{kind:?}"
            );
            assert!(
                !state.is_running(),
                "{kind:?}: the write lock is free again"
            );
        }
    }

    fn restore_ended(how: &str) -> [(String, String, String); 1] {
        [("restore".to_owned(), how.to_owned(), NAME.to_owned())]
    }

    /// Test 10. A job holds the lock from its start to its end, however it ends; the file
    /// is root's alone and is made with its folder; its being there means nothing.
    #[tokio::test]
    async fn a_job_holds_the_file_lock_until_it_ends() {
        let (state, _changes) = state();
        let lock = state.lock();
        let running = state.begin(JobKind::Create).await.unwrap();
        assert_eq!(flock_exit(&lock), 75, "held while the job runs");
        let mode = std::fs::metadata(&lock).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        running.end(JobState::Done);
        assert_eq!(flock_exit(&lock), 0, "free once it ended");
        // A job dropped unended (a panic) frees it too.
        let running = state.begin(JobKind::Delete).await.unwrap();
        assert_eq!(flock_exit(&lock), 75);
        drop(running);
        assert_eq!(flock_exit(&lock), 0);
        // Removed by hand between two jobs: the next job makes it again.
        std::fs::remove_file(&lock).unwrap();
        let running = state.begin(JobKind::Create).await.unwrap();
        assert_eq!(flock_exit(&lock), 75);
        running.end(JobState::Failed);
    }

    /// A lock file that others could open (the `prerm` of a build before the
    /// `umask` made it 0644, and `flock` works on a read-only descriptor, so any user could
    /// have held the lock) is closed to them when the helper takes it.
    #[tokio::test]
    async fn a_lock_file_made_too_open_is_closed_to_others() {
        let (state, _changes) = state();
        let lock = state.lock();
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
        let running = state.begin(JobKind::Create).await.unwrap();
        let mode = std::fs::metadata(&lock).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o600);
        assert_eq!(flock_exit(&lock), 75);
        running.end(JobState::Done);
    }

    /// A lock file that isn't the expected owner's (root's outside the tests) is
    /// someone else's file: the job is refused `Busy`, and the file is left as it is. No
    /// test can make a file owned by another uid without root, so the expected owner is the
    /// one that differs here.
    #[tokio::test]
    async fn a_lock_file_owned_by_someone_else_refuses_the_job() {
        let ours = rustix::process::geteuid().as_raw();
        let folder = std::env::temp_dir().join(format!("apsis-state-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let lock = folder.join("job.lock");
        std::fs::write(&lock, "").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (state, mut changes) = State::new(&lock, ours + 1);
        every_write_is_busy(&state).await;
        assert!(drain(&mut changes).is_empty(), "nothing is announced");
        let mode = std::fs::metadata(&lock).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o644, "not ours to change");
        assert_eq!(flock_exit(&lock), 0, "and not locked");
        // The same file with its owner expected: taken.
        let (state, _changes) = State::new(&lock, ours);
        let running = state.begin(JobKind::Create).await.expect("ours");
        running.end(JobState::Done);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// Test 11. While a package script holds the lock no job begins: `Busy`, the write lock
    /// free again, nothing announced. A read takes no file lock.
    #[tokio::test]
    async fn begin_is_busy_while_the_file_lock_is_held_elsewhere() {
        let (state, mut changes) = state();
        let script = hold(&state.lock());
        every_write_is_busy(&state).await;
        assert_eq!(state.job(), None);
        assert!(drain(&mut changes).is_empty(), "nothing is announced");
        drop(state.read().expect("a read takes no file lock"));
        drop(script);
        let running = state
            .begin(JobKind::Create)
            .await
            .expect("released: the job begins");
        assert_eq!(drain(&mut changes).len(), 1, "and is announced");
        running.end(JobState::Done);
    }

    /// Test 12. Held while the restore prepares, free once the plan is ready: an upgrade at
    /// the prompt goes on.
    #[tokio::test]
    async fn a_ready_plan_holds_no_file_lock() {
        let (state, _changes) = state();
        let lock = state.lock();
        let running = state.begin(JobKind::Restore).await.unwrap();
        state.named(NAME);
        assert_eq!(flock_exit(&lock), 75, "held while it prepares");
        running.ready(1000, STARTER);
        assert!(state.is_ready());
        assert_eq!(flock_exit(&lock), 0, "free at the prompt");
    }

    /// "Restart now" (and a cancel) takes the plan by marking it where it
    /// is and taking the file lock, in one step: until the plan ends every write is still
    /// `Busy`, a package script is refused, nobody else gets the plan, and reads and `Job()`
    /// go on as at the prompt.
    #[tokio::test]
    async fn arming_marks_the_plan_and_takes_the_lock_in_one_step() {
        let (state, mut changes) = state();
        let lock = state.lock();
        ready_plan(&state).await;
        drain(&mut changes);
        let arming = state.take_ready().unwrap().expect("the plan");
        assert_eq!(arming.info().snapshot, NAME);
        every_write_is_busy(&state).await;
        assert!(matches!(state.stop_target(NAME), Err(Error::Busy)));
        assert_eq!(flock_exit(&lock), 75, "a package script is refused");
        assert!(
            matches!(state.take_ready(), Err(Error::Busy)),
            "a second Restart now, or a cancel"
        );
        assert!(
            matches!(state.starter_left(STARTER), Ok(None)),
            "the starter leaving"
        );
        drop(state.read().expect("reads go on"));
        let job = state.job().expect("the plan is still the job");
        assert_eq!(job.kind, JobKind::Restore);
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.percent, Some(100.0));
        assert!(state.is_ready() && !state.is_running());
        assert!(
            drain(&mut changes).is_empty(),
            "nothing announced meanwhile"
        );
        // The link is made: the plan leaves its slot, the lock is free, one announcement.
        arming.end(JobState::Done);
        assert_eq!(flock_exit(&lock), 0);
        assert_eq!(drain(&mut changes), restore_ended("done"));
        assert_eq!(state.job(), None);
        assert!(matches!(state.take_ready(), Ok(None)));
        let running = state.begin(JobKind::Create).await.expect("writes again");
        running.end(JobState::Done);
    }

    /// Test 14. A package script holds the lock at "Restart now": `Busy`, and the plan
    /// stays ready and unmarked.
    #[tokio::test]
    async fn arming_is_busy_while_the_file_lock_is_held_elsewhere() {
        let (state, mut changes) = state();
        ready_plan(&state).await;
        drain(&mut changes);
        let script = hold(&state.lock());
        assert!(matches!(state.take_ready(), Err(Error::Busy)));
        assert!(state.is_ready(), "the plan stays");
        assert!(drain(&mut changes).is_empty(), "nothing is announced");
        drop(script);
        let ready = state
            .take_ready()
            .unwrap()
            .expect("it wasn't marked: taken now");
        ready.end(JobState::Stopped);
        assert_eq!(drain(&mut changes), restore_ended("stopped"));
    }

    /// The starter leaving the bus takes its plan the same way: marked where
    /// it is, with the file lock, until its files are gone and it ends `stopped`.
    #[tokio::test]
    async fn a_gone_starter_marks_the_plan_and_takes_the_lock_too() {
        let (state, mut changes) = state();
        let lock = state.lock();
        ready_plan(&state).await;
        drain(&mut changes);
        let gone = state
            .starter_left(STARTER)
            .unwrap()
            .expect("the starter's own");
        every_write_is_busy(&state).await;
        assert_eq!(flock_exit(&lock), 75, "a package script is refused");
        assert!(matches!(state.take_ready(), Err(Error::Busy)));
        assert!(matches!(state.starter_left(STARTER), Ok(None)));
        assert!(state.job().is_some() && drain(&mut changes).is_empty());
        gone.end(JobState::Stopped);
        assert_eq!(flock_exit(&lock), 0);
        assert_eq!(drain(&mut changes), restore_ended("stopped"));
        let running = state.begin(JobKind::Restore).await.expect("writes again");
        running.end(JobState::Stopped);
    }

    /// A package script holds the lock when the starter leaves: the plan stays
    /// ready and unmarked (the script stops the helper next), and the watcher hears `Busy`.
    #[tokio::test]
    async fn a_gone_starters_plan_stays_while_the_file_lock_is_held_elsewhere() {
        let (state, mut changes) = state();
        ready_plan(&state).await;
        drain(&mut changes);
        let script = hold(&state.lock());
        assert!(matches!(state.starter_left(STARTER), Err(Error::Busy)));
        assert!(
            matches!(state.starter_left(":1.43"), Ok(None)),
            "another name: nothing asked of the lock"
        );
        assert!(state.is_ready() && drain(&mut changes).is_empty());
        drop(script);
        let gone = state.starter_left(STARTER).unwrap().expect("taken now");
        gone.end(JobState::Stopped);
    }

    /// A plan taken and then dropped unended (a panic on the way) is gone, with the lock
    /// free and writes admitted, as a job dropped unended is.
    #[tokio::test]
    async fn a_plan_taken_and_dropped_unended_frees_the_lock_and_the_writes() {
        let (state, mut changes) = state();
        let lock = state.lock();
        ready_plan(&state).await;
        drain(&mut changes);
        let taken = state.take_ready().unwrap().expect("the plan");
        assert_eq!(flock_exit(&lock), 75);
        assert!(state.is_ready());
        drop(taken);
        assert_eq!(flock_exit(&lock), 0);
        assert!(!state.is_ready());
        assert!(drain(&mut changes).is_empty(), "no announcement");
        let running = state.begin(JobKind::Create).await.expect("writes again");
        running.end(JobState::Done);
    }

    /// Test 15. What the helper starts mid-job (rsync, `mount`, `systemd-run`) never holds
    /// a copy of the lock, so the lock dies with the helper and not with its children.
    /// The child says `ready` before its open files are read: during exec the kernel lets
    /// the spawning parent go before it closes the close-on-exec descriptors, so
    /// `/proc/<pid>/fd` read straight after `spawn` can still list the test's own. The shell
    /// that wrote the line is past its exec, and `exec sleep` after it opens nothing.
    #[tokio::test]
    async fn a_child_never_keeps_the_lock() {
        let (state, _changes) = state();
        let lock = state.lock();
        let running = state.begin(JobKind::Create).await.unwrap();
        assert_eq!(flock_exit(&lock), 75, "held while the job runs");
        let mut child = std::process::Command::new("sh")
            .args(["-c", "echo ready; exec sleep 30"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line, "ready\n", "the child is past its exec");
        // What the child has open: a copy of the lock would outlive a helper that's killed.
        let open: Vec<PathBuf> = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
            .unwrap()
            .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
            .collect();
        running.end(JobState::Done);
        let free = flock_exit(&lock);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!open.is_empty(), "the child's open files were read");
        assert!(
            !open.contains(&lock),
            "the child has the lock open: {open:?}"
        );
        assert_eq!(free, 0, "free at the job's end though the child still runs");
    }

    /// Test 16. A symlink where the lock file should be isn't followed: the job is refused
    /// `Busy`, and the link's target is neither locked, changed nor made.
    #[tokio::test]
    async fn a_symlinked_lock_file_refuses_the_job() {
        let (state, mut changes) = state();
        let lock = state.lock();
        let folder = lock.parent().unwrap();
        std::fs::create_dir_all(folder).unwrap();
        let target = folder.join("elsewhere");
        std::fs::write(&target, "not a lock\n").unwrap();
        symlink(&target, &lock).unwrap();
        every_write_is_busy(&state).await;
        assert!(drain(&mut changes).is_empty(), "nothing is announced");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "not a lock\n");
        assert_eq!(flock_exit(&target), 0, "its target isn't locked");
        // A link to nothing: nothing is made at its target.
        std::fs::remove_file(&target).unwrap();
        every_write_is_busy(&state).await;
        assert!(!target.exists());
        // The plan's way to the lock is the same.
        std::fs::remove_file(&lock).unwrap();
        ready_plan(&state).await;
        std::fs::remove_file(&lock).unwrap();
        symlink(&target, &lock).unwrap();
        assert!(matches!(state.take_ready(), Err(Error::Busy)));
        assert!(state.is_ready() && !target.exists());
    }
}
