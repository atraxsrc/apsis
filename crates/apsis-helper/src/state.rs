// SPDX-License-Identifier: GPL-3.0-only

//! What the helper is doing: at most one Timeshift run at a time, and when it may exit.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use apsis_core::{Backend, Error, Result, Runner, SnapshotList, TimeshiftCli, parse_snapshot_name};
use tokio::sync::Notify;

pub struct State<R> {
    cli: TimeshiftCli<R>,
    /// A [`Running`] exists: a Timeshift run holds the single-operation lock.
    running: AtomicBool,
    /// Method calls in progress, including ones waiting for the polkit dialog, and finished
    /// operations still sending their `Finished` signal.
    calls: AtomicUsize,
    /// Wakes [`State::idle_for`] whenever any of the above changes.
    activity: Notify,
}

impl<R: Runner> State<R> {
    pub fn new(runner: R) -> Arc<Self> {
        Arc::new(Self {
            cli: TimeshiftCli::new(runner),
            running: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            activity: Notify::new(),
        })
    }

    /// Counts a call as in progress until the guard drops. The helper doesn't exit meanwhile.
    pub fn call(self: &Arc<Self>) -> Call<R> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.activity.notify_waiters();
        Call(Arc::clone(self))
    }

    /// The device the next Timeshift run targets (see `TimeshiftCli::snapshot_device`).
    pub fn snapshot_device(&self) -> Option<String> {
        self.cli.snapshot_device()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Takes the single-operation lock until the [`Running`] drops.
    ///
    /// # Errors
    ///
    /// [`Error::Busy`] if a Timeshift run holds it: a second call is refused, not queued.
    pub fn begin(self: &Arc<Self>) -> Result<Running<R>> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| Error::Busy)?;
        self.activity.notify_waiters();
        Ok(Running(Arc::clone(self)))
    }

    fn is_idle(&self) -> bool {
        !self.is_running() && self.calls.load(Ordering::SeqCst) == 0
    }

    /// Returns once nothing has happened for `idle` and nothing is in progress. Never while
    /// Timeshift runs or a call is open, however long that takes.
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
pub struct Call<R>(Arc<State<R>>);

impl<R> Drop for Call<R> {
    fn drop(&mut self) {
        self.0.calls.fetch_sub(1, Ordering::SeqCst);
        self.0.activity.notify_waiters();
    }
}

/// The single-operation lock (see [`State::begin`]). Timeshift only runs through it. Calls
/// block; run them off the async runtime.
pub struct Running<R>(Arc<State<R>>);

impl<R: Runner> Running<R> {
    pub fn list(&self) -> Result<SnapshotList> {
        self.0.cli.list()
    }

    /// Lists first, so `--snapshot-device` is the device Timeshift reports right now, then
    /// creates. Refuses (like the CLI backend) when no device is selected.
    pub fn create(&self, comment: &str) -> Result<()> {
        self.0.cli.list()?;
        self.0.cli.create(comment)
    }

    /// Lists first, and only deletes a snapshot that list has.
    pub fn delete(&self, name: &str) -> Result<()> {
        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.to_owned()));
        }
        let list = self.0.cli.list()?;
        if !list.snapshots.iter().any(|s| s.name == name) {
            return Err(Error::NoSuchSnapshot(name.to_owned()));
        }
        self.0.cli.delete(name)
    }
}

impl<R> Drop for Running<R> {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
        self.0.activity.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::io;
    use std::sync::Mutex;

    use apsis_core::RunOutput;

    use super::*;

    const DEVICE_LIST: &str = include_str!("../../apsis-core/tests/fixtures/list-rsync-device.txt");
    const UNCONFIGURED_LIST: &str =
        include_str!("../../apsis-core/tests/fixtures/list-unconfigured.txt");

    /// Records each argv and replies with canned stdout.
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        replies: Mutex<VecDeque<&'static str>>,
    }

    /// A [`Fake`] shared with the test, which reads what it recorded.
    struct Shared(Arc<Fake>);

    impl Runner for Shared {
        fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
            let argv = argv
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.0.calls.lock().unwrap().push(argv);
            let stdout = self
                .0
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("extra command");
            Ok(RunOutput {
                success: true,
                code: Some(0),
                stdout: stdout.to_owned(),
                stderr: String::new(),
            })
        }
    }

    fn fake_state(replies: &[&'static str]) -> (Arc<State<Shared>>, Arc<Fake>) {
        let fake = Arc::new(Fake {
            calls: Mutex::default(),
            replies: Mutex::new(replies.iter().copied().collect()),
        });
        (State::new(Shared(Arc::clone(&fake))), fake)
    }

    fn actions(fake: &Fake) -> Vec<String> {
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .map(|a| a[1].clone())
            .collect()
    }

    fn first_snapshot() -> String {
        apsis_core::parse_list(DEVICE_LIST).unwrap().snapshots[0]
            .name
            .clone()
    }

    #[test]
    fn a_second_operation_is_refused_not_queued() {
        let (state, _) = fake_state(&[]);
        let running = state.begin().unwrap();
        assert!(matches!(state.begin(), Err(Error::Busy)));
        drop(running);
        assert!(state.begin().is_ok());
    }

    #[test]
    fn create_lists_first_and_targets_the_listed_uuid() {
        let (state, fake) = fake_state(&[DEVICE_LIST, ""]);
        state.begin().unwrap().create("before update").unwrap();
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(actions(&fake), ["--list", "--create"]);
        let create = &calls[1];
        assert!(
            create
                .windows(2)
                .any(|w| w[0] == "--comments" && w[1] == "before update")
        );
        // The UUID, never the /dev path: that can change when a USB disk reconnects.
        let uuid = "00000000-0000-0000-0000-000000000000";
        assert!(
            create
                .windows(2)
                .any(|w| w[0] == "--snapshot-device" && w[1] == uuid)
        );
        assert!(!create.iter().any(|a| a.starts_with("/dev/")));
        assert_eq!(state.snapshot_device().as_deref(), Some(uuid));
    }

    #[test]
    fn create_without_a_device_is_refused_after_the_list() {
        let (state, fake) = fake_state(&[UNCONFIGURED_LIST]);
        let result = state.begin().unwrap().create("");
        assert!(matches!(result, Err(Error::NoSnapshotDevice)), "{result:?}");
        assert_eq!(actions(&fake), ["--list"]);
    }

    #[test]
    fn bad_input_never_reaches_timeshift() {
        let (state, fake) = fake_state(&[]);
        let running = state.begin().unwrap();
        assert!(matches!(
            running.delete("--help"),
            Err(Error::InvalidSnapshotName(_))
        ));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn delete_only_deletes_a_listed_snapshot() {
        let (state, fake) = fake_state(&[DEVICE_LIST]);
        let result = state.begin().unwrap().delete("2001-01-01_00-00-00");
        assert!(
            matches!(result, Err(Error::NoSuchSnapshot(_))),
            "{result:?}"
        );
        assert_eq!(actions(&fake), ["--list"]);

        let (state, fake) = fake_state(&[DEVICE_LIST, ""]);
        let name = first_snapshot();
        state.begin().unwrap().delete(&name).unwrap();
        assert_eq!(actions(&fake), ["--list", "--delete"]);
        assert!(fake.calls.lock().unwrap()[1].contains(&name));
    }

    #[tokio::test(start_paused = true)]
    async fn idle_exit_waits_for_a_running_operation() {
        let idle = Duration::from_secs(60);
        let (state, _) = fake_state(&[]);
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
        let (state, _) = fake_state(&[]);
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
