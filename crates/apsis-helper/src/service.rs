// SPDX-License-Identifier: GPL-3.0-only

//! The D-Bus interface `Helper3`: `List`, `Create`, `Delete`, `DeleteMany`, `Stop`, `Job`,
//! `ReadConfig`, `WriteConfig`, `CheckRestore`, and the `JobChanged` and `Finished` signals.
//! Nothing else.
//!
//! `List` and `CheckRestore` are reads: they share the read-only mount and are never jobs
//! (`state.rs` has the rules). Everything that writes is a job under the one lock.
//!
//! Every call is logged with its result on stderr, which systemd puts in the journal
//! (`journalctl -u apsis-helper`). Comments are cut to [`LOGGED_COMMENT_CHARS`].

use std::path::Path;
use std::sync::Arc;

use apsis_core::helper::names::{
    ACTION_CONFIGURE, ACTION_CREATE, ACTION_DELETE, ACTION_LIST, ACTION_RESTORE, ACTION_STOP,
    OBJECT_PATH, OP_CREATE, OP_DELETE, OP_DELETE_MANY, OP_RESTORE,
};
use apsis_core::helper::{
    WireCheckRestore, WireConfig, WireConfigInfo, WireListWithUsage3, check_delete_many,
    config_from_wire, encode_error, to_wire_with_usage3,
};
use apsis_core::job::{self, JobKind, JobState, WireJob};
use apsis_core::native::{Cancel, TooLate};
use apsis_core::restore::dialog;
use apsis_core::restore::filter::Home;
use apsis_core::status::BY_UUID;
use apsis_core::{Backend, Error, SnapshotList, parse_snapshot_name, validate_comment};
use zbus::fdo::DBusProxy;
use zbus::message::Header;
use zbus::names::{BusName, UniqueName};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, DBusError, interface};

use crate::check::{self, Live, SnapshotFiles};
use crate::native::{self, SharedMount};
use crate::polkit;
use crate::prepare;
use crate::runner::DirectRunner;
use crate::settings::{self, Files};
use crate::state::{Announcement, Running, State};
use crate::usage;

/// Longest part of a comment that goes into the journal.
const LOGGED_COMMENT_CHARS: usize = 40;

/// The helper object at [`OBJECT_PATH`].
pub struct Helper {
    state: Arc<State>,
    /// The read-only mount the lists share.
    mount: Arc<SharedMount<DirectRunner>>,
}

impl Helper {
    pub fn new(state: Arc<State>) -> Self {
        Self {
            state,
            mount: Arc::new(SharedMount::default()),
        }
    }
}

/// The helper's D-Bus errors, named `<ERROR_PREFIX>.<Variant>` (see `apsis_core::helper::names`).
#[derive(Debug, DBusError)]
#[zbus(prefix = "io.github.atraxsrc.Apsis.Helper3.Error")]
pub enum HelperError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotAuthorized(String),
    Busy(String),
    InvalidInput(String),
    /// The backup disk isn't there; the message is from [`encode_error`].
    DeviceNotFound(String),
    /// `WriteConfig`: the config file changed since the caller read it.
    Changed(String),
    /// The message is from [`encode_error`].
    Failed(String),
}

impl From<Error> for HelperError {
    fn from(error: Error) -> Self {
        match error {
            Error::NotAuthorized => Self::NotAuthorized(error.to_string()),
            Error::Busy => Self::Busy(error.to_string()),
            Error::InvalidComment(_)
            | Error::InvalidSnapshotName(_)
            | Error::NoSuchSnapshot(_)
            | Error::InvalidSettings(_)
            | Error::InvalidInput(_) => Self::InvalidInput(error.to_string()),
            Error::ConfigChanged => Self::Changed(error.to_string()),
            Error::DeviceNotFound { .. } => Self::DeviceNotFound(encode_error(&error)),
            _ => Self::Failed(encode_error(&error)),
        }
    }
}

#[interface(name = "io.github.atraxsrc.Apsis.Helper3")]
impl Helper {
    /// The snapshots on the backup device (each one's `info.json`, read on the read-only
    /// mount the lists share, with its format as the raw rsync flags string, `""` for an old
    /// one), leftovers of interrupted creates, and its `statvfs` (polkit:
    /// `list`, no password for the active session). A read, not a job: lists run at the same
    /// time as each other and are never announced; one that arrives while a write runs or
    /// waits is refused `Busy`.
    async fn list(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<WireListWithUsage3, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("list for {caller}");
        let result = async {
            authorize(connection, &caller, ACTION_LIST, false).await?;
            let reading = self.state.read()?;
            let mount = Arc::clone(&self.mount);
            blocking(move || {
                // Dropped last, after the share of the mount: when the readers' count reaches
                // zero the device is unmounted, and a waiting write may mount it read-write.
                let _reading = reading;
                let (backend, _shared) = native::open_shared(&DirectRunner, &mount, log_lines)?;
                let mut list = backend.list()?;
                list.usage = usage::of_mount_point(Path::new(native::MOUNT_POINT));
                Ok(list)
            })
            .await
        }
        .await;
        match &result {
            Ok(list) => log(&format!("{label}: {}", describe_list(list))),
            Err(error) => log(&format!("{label}: {}", describe_error(error))),
        }
        Ok(to_wire_with_usage3(&result?))
    }

    /// Whether `snapshot` can be restored on this computer, and what the Restore dialog says
    /// (PLAN 6b.7, 6b.9; polkit: `list`, no password). A read like `List`: it shares the
    /// read-only mount, is never a job, and is refused `Busy` while a write runs or waits.
    /// `snapshot` must be a snapshot name the fresh list has (not a leftover).
    async fn check_restore(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        snapshot: String,
    ) -> Result<WireCheckRestore, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("check-restore {snapshot:?} for {caller}");
        let result = async {
            if parse_snapshot_name(&snapshot).is_none() {
                return Err(Error::InvalidSnapshotName(snapshot.clone()));
            }
            authorize(connection, &caller, ACTION_LIST, false).await?;
            let reading = self.state.read()?;
            let mount = Arc::clone(&self.mount);
            blocking(move || {
                let _reading = reading;
                let (backend, _shared) = native::open_shared(&DirectRunner, &mount, log_lines)?;
                let list = backend.list()?;
                if !list.snapshots.iter().any(|s| s.name == snapshot) {
                    return Err(Error::NoSuchSnapshot(snapshot));
                }
                let files =
                    SnapshotFiles::read(&check::snapshot_dir(&backend.config().repo, &snapshot));
                let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")?;
                let live = Live::read(Path::new("/"), &DirectRunner, &mountinfo)?;
                Ok(check::dialog(&live, &files))
            })
            .await
        }
        .await;
        match &result {
            Ok(dialog) => log(&format!("{label}: {}", describe_dialog(dialog))),
            Err(error) => log(&format!("{label}: {}", describe_error(error))),
        }
        Ok(dialog::to_wire(&result?))
    }

    /// Prepares a full-system restore of `snapshot` (PLAN 6b.4, 6b.9; polkit: `restore`,
    /// asked every time) and returns; `Finished("restore", ok, ..)` follows, `ok` meaning the
    /// plan is ready at the prompt. The checks again, both dry runs and the space checks, the
    /// safety snapshot (with `/home` when `restore_home`), the plan files, the recovery note.
    /// A `restore` job; stoppable with `Stop(snapshot)` until ready. While the plan is ready,
    /// every write is `Busy` and reads go through; `RestartToRestore` or `CancelRestore` ends
    /// it, as does the starter's connection leaving the bus.
    async fn restore(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        snapshot: String,
        restore_home: bool,
        safety_snapshot: bool,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!(
            "restore {snapshot:?} {} {} for {caller}",
            if restore_home {
                "restore-home"
            } else {
                "keep-home"
            },
            if safety_snapshot {
                "safety"
            } else {
                "no-safety"
            }
        );
        let cancel = Cancel::new();
        let mut request = None;
        let started = async {
            if parse_snapshot_name(&snapshot).is_none() {
                return Err(Error::InvalidSnapshotName(snapshot.clone()));
            }
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_RESTORE, true).await?;
            let uid = unix_user(connection, &caller).await?;
            let running = self.state.begin(JobKind::Restore).await?;
            // Named at once: `Stop(snapshot)` finds it, and `Job()` says what's restored.
            self.state.named(&snapshot);
            self.state.stoppable(Arc::clone(&cancel), uid);
            request = Some(prepare::Request {
                snapshot: snapshot.clone(),
                home: if restore_home {
                    Home::Restore
                } else {
                    Home::Keep
                },
                safety_snapshot,
                starter_uid: uid,
            });
            Ok(running)
        }
        .await;
        let ending = Ending::Ready {
            starter: request.as_ref().map_or(0, |r| r.starter_uid),
            starter_name: caller.to_string(),
        };
        self.start(
            connection,
            caller,
            OP_RESTORE,
            label,
            started,
            ending,
            move |state| {
                let request = request.ok_or_else(|| Error::Helper("not started".to_owned()))?;
                prepare::prepare(&request, state, cancel).map(|_| String::new())
            },
        )
    }

    /// Starts a snapshot (polkit: `create`) and returns; `Finished("create", ..)` follows.
    /// rsync runs at idle I/O priority and nice 19. Interrupted creates' folders are removed
    /// first. The caller's uid may `Stop` it without a password.
    async fn create(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        comment: String,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("create {} for {caller}", logged_comment(&comment));
        let started = async {
            validate_comment(&comment)?;
            let uid = unix_user(connection, &caller).await?;
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_CREATE, true).await?;
            let running = self.state.begin(JobKind::Create).await?;
            let cancel = Cancel::new();
            self.state.stoppable(Arc::clone(&cancel), uid);
            Ok((running, cancel))
        }
        .await;
        let (started, cancel) = match started {
            Ok((running, cancel)) => (Ok(running), Some(cancel)),
            Err(error) => (Err(error), None),
        };
        self.start(
            connection,
            caller,
            OP_CREATE,
            label,
            started,
            Ending::Done,
            move |state| {
                let cancel = cancel.ok_or_else(|| Error::Helper("not started".to_owned()))?;
                let (backend, _mounted) = native::open(&DirectRunner, log_lines)?;
                let device = backend.config().device_uuid.clone();
                let (named, progress) = (Arc::clone(state), Arc::clone(state));
                let backend = backend
                    .with_cancel(cancel)
                    .with_named(move |name| named.named(name))
                    .with_progress(move |p| progress.progress(&p));
                backend
                    .create(&comment)
                    .map(|()| String::new())
                    .map_err(|e| removed_or(e, device.as_deref(), Path::new(BY_UUID)))
            },
        )
    }

    /// Starts deleting snapshot `name`, or the interrupted create's folder `name` (polkit:
    /// `delete`) and returns; `Finished("delete", ..)` follows. The backup device is mounted
    /// read-write for the delete only; the name must be one the list has.
    async fn delete(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        name: String,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        // `{:?}`: whatever was sent, it goes into the journal as one quoted line.
        let label = format!("delete {name:?} for {caller}");
        let started = async {
            if parse_snapshot_name(&name).is_none() {
                return Err(Error::InvalidSnapshotName(name.clone()));
            }
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_DELETE, true).await?;
            self.state.begin(JobKind::Delete).await
        }
        .await;
        self.start(
            connection,
            caller,
            OP_DELETE,
            label,
            started,
            Ending::Done,
            move |state| {
                state.named(&name);
                let (backend, _mounted) = native::open(&DirectRunner, log_lines)?;
                let device = backend.config().device_uuid.clone();
                let list = backend.list()?;
                delete_known(&backend, &list, &name)
                    .map(|()| String::new())
                    .map_err(|e| removed_or(e, device.as_deref(), Path::new(BY_UUID)))
            },
        )
    }

    /// Starts deleting `names` (snapshots or interrupted creates' folders) as one job, in
    /// order (polkit: `delete`, asked once), and returns; `Finished("delete-many", ..)`
    /// follows. At least two names, each a snapshot name, none repeated. It stops at the first
    /// failure; the message then says what was deleted, what failed and what's left.
    async fn delete_many(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        names: Vec<String>,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("delete-many {} for {caller}", logged_names(&names));
        let started = async {
            check_delete_many(&names)?;
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_DELETE, true).await?;
            self.state.begin(JobKind::DeleteMany).await
        }
        .await;
        let journal = label.clone();
        self.start(
            connection,
            caller,
            OP_DELETE_MANY,
            label,
            started,
            Ending::Done,
            move |state| {
                let (backend, _mounted) = native::open(&DirectRunner, log_lines)?;
                let device = backend.config().device_uuid.clone();
                let list = backend.list()?;
                let total = names.len();
                let mut deleted = Vec::with_capacity(total);
                for (done, name) in names.iter().enumerate() {
                    state.step(name, done, total);
                    if let Err(error) = delete_known(&backend, &list, name) {
                        return Err(Error::DeleteManyStopped {
                            deleted,
                            failed: name.clone(),
                            left: names[done + 1..].to_vec(),
                            reason: Box::new(removed_or(
                                error,
                                device.as_deref(),
                                Path::new(BY_UUID),
                            )),
                        });
                    }
                    log(&format!("{journal}: deleted {name}"));
                    deleted.push(name.clone());
                }
                state.step("", total, total);
                Ok(String::new())
            },
        )
    }

    /// Stops the running create if it is making `snapshot`, and returns once stopping has
    /// begun: rsync's process group gets `SIGTERM`, `SIGKILL` 10 s later if it's still there,
    /// and its copy is removed; the create's `Finished` says `stopped`. The uid that started it
    /// isn't asked; anyone else needs polkit `stop`. Refused after the snapshot has started
    /// being put in place.
    async fn stop(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        snapshot: String,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("stop {snapshot:?} for {caller}");
        let result = async {
            if parse_snapshot_name(&snapshot).is_none() {
                return Err(Error::InvalidSnapshotName(snapshot.clone()));
            }
            let (_, starter) = self.state.stop_target(&snapshot)?;
            let uid = unix_user(connection, &caller).await?;
            if stop_needs_auth(starter, uid) {
                authorize(connection, &caller, ACTION_STOP, true).await?;
            }
            // The password dialog may have taken a while: still the same create?
            let (cancel, _) = self.state.stop_target(&snapshot)?;
            cancel.request().map_err(|TooLate| {
                Error::InvalidInput(
                    "the snapshot is being put in place; it can't be stopped now".to_owned(),
                )
            })?;
            self.state.stopping();
            Ok(uid)
        }
        .await;
        match &result {
            Ok(uid) => log(&format!(
                "{label} (uid {uid}): stopping, SIGTERM to rsync's process group (SIGKILL after 10 s)"
            )),
            Err(error) => log(&format!("{label}: {}", describe_error(error))),
        }
        result.map(drop).map_err(HelperError::from)
    }

    /// What the helper is doing now (polkit: `list`, no password for the active session).
    async fn job(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<WireJob, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        authorize(connection, &caller, ACTION_LIST, false).await?;
        Ok(job::to_wire(self.state.job().as_ref()))
    }

    /// Apsis's config (converted from the old format, or imported from Timeshift's settings,
    /// until it's saved) and the block devices, for the settings view (polkit: `list`, no
    /// password for the active session).
    async fn read_config(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<WireConfigInfo, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let result = async {
            authorize(connection, &caller, ACTION_LIST, false).await?;
            blocking(|| settings::info(&Files::system(), &DirectRunner)).await
        }
        .await;
        match &result {
            Ok(info) if !info.3.is_empty() => {
                log(&format!(
                    "read config for {caller}: not saved yet (converted or imported)"
                ));
            }
            Ok(_) => {}
            Err(error) => log(&format!(
                "read config for {caller}: {}",
                describe_error(error)
            )),
        }
        Ok(result?)
    }

    /// Writes `/etc/apsis/config.toml` (polkit: `configure`) if it still reads `expected`
    /// (empty: there's none yet), keeping the previous file as `.bak`. Returns once done; the
    /// text is a note, empty when all went well.
    async fn write_config(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        expected: String,
        config: WireConfig,
    ) -> Result<String, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("write config for {caller}");
        let result = async {
            let config = config_from_wire(config);
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_CONFIGURE, true).await?;
            let running = self.state.begin(JobKind::Configure).await?;
            let (result, announced) = blocking(move || {
                let result = Files::system().write(&DirectRunner, &expected, &config);
                let announced = running.end(if result.is_ok() {
                    JobState::Done
                } else {
                    JobState::Failed
                });
                Ok((result, announced))
            })
            .await?;
            // The end is on the bus before the reply.
            announced.wait().await;
            result
        }
        .await;
        match result {
            Ok(written) => {
                log(&format!(
                    "{label}: {}",
                    if written { "written" } else { "unchanged" }
                ));
                Ok(String::new())
            }
            Err(error) => {
                log(&format!("{label}: {}", describe_error(&error)));
                Err(error.into())
            }
        }
    }

    /// The job started, got further, is stopping, or ended. To everyone on the bus: no
    /// comment, caller, error text or path in it.
    #[zbus(signal)]
    pub async fn job_changed(emitter: &SignalEmitter<'_>, job: WireJob) -> zbus::Result<()>;

    /// An operation ended. Sent only to the caller that started it.
    #[zbus(signal)]
    async fn finished(
        emitter: &SignalEmitter<'_>,
        op: &str,
        ok: bool,
        message: &str,
    ) -> zbus::Result<()>;
}

impl Helper {
    /// Refuses before the password dialog, so nobody types a password for a call that can't
    /// run. [`State::begin`] still decides, after polkit, if two calls race. Readers in don't
    /// refuse a write: `begin` waits for them.
    fn refuse_if_running(&self) -> apsis_core::Result<()> {
        if self.state.is_running() {
            return Err(Error::Busy);
        }
        Ok(())
    }

    /// Logs whether the operation could start. If it did, runs `work` in the background
    /// holding the lock, then releases the lock, announces how it ended (`JobChanged`; or, for
    /// [`Ending::Ready`], that the plan is ready), logs it and tells `caller` with `Finished`:
    /// the text `work` returned, or the error.
    #[allow(clippy::too_many_arguments, reason = "one call site per method")]
    fn start(
        &self,
        connection: &Connection,
        caller: UniqueName<'static>,
        op: &'static str,
        label: String,
        started: apsis_core::Result<Running>,
        ending: Ending,
        work: impl FnOnce(&Arc<State>) -> apsis_core::Result<String> + Send + 'static,
    ) -> Result<(), HelperError> {
        let running = match started {
            Ok(running) => running,
            Err(error) => {
                log(&format!("{label}: {}", describe_error(&error)));
                return Err(error.into());
            }
        };
        log(&format!("{label}: started"));
        let ready = matches!(ending, Ending::Ready { .. });
        // Keeps the helper alive until the signal is out.
        let call = self.state.call();
        let connection = connection.clone();
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let _call = call;
            // The lock is free before the end is announced and before `Finished` arrives, so
            // no refresh they set off is refused as busy (the rule the restore builds on).
            // And the end is on the bus before `Finished` is sent, so the caller (a listener
            // too) sees its job end before it hears the result, never the other way round.
            let result = match blocking(move || {
                let result = work(&state);
                let announced = match (&result, ending) {
                    (
                        Ok(_),
                        Ending::Ready {
                            starter,
                            starter_name,
                        },
                    ) => running.ready(starter, &starter_name),
                    (Ok(_), Ending::Done) => running.end(JobState::Done),
                    (Err(Error::Stopped), _) => running.end(JobState::Stopped),
                    (Err(_), _) => running.end(JobState::Failed),
                };
                Ok((result, announced))
            })
            .await
            {
                Ok((result, announced)) => {
                    announced.wait().await;
                    result
                }
                Err(error) => Err(error),
            };
            let (ok, message) = match result {
                Ok(text) => {
                    log(&format!(
                        "{label}: {}",
                        if ready { "ready" } else { "done" }
                    ));
                    (true, text)
                }
                Err(error) => {
                    log(&format!("{label}: {}", describe_error(&error)));
                    (false, encode_error(&error))
                }
            };
            let sent = match SignalEmitter::new(&connection, OBJECT_PATH) {
                Ok(emitter) => {
                    let emitter = emitter.set_destination(caller.into());
                    Helper::finished(&emitter, op, ok, &message).await
                }
                Err(error) => Err(error),
            };
            if let Err(error) = sent {
                log(&format!("{label}: couldn't send the result: {error}"));
            }
        });
        Ok(())
    }
}

/// How a job that succeeds ends (see [`Helper::start`]).
enum Ending {
    /// `done`, the lock free.
    Done,
    /// A restore's preparation: the plan moves next to the lock as the ready plan
    /// ([`Running::ready`]), announced `running` at 100%.
    Ready { starter: u32, starter_name: String },
}

/// Sends each job change as `JobChanged`, to everyone on the bus, in order. Runs as long as
/// the helper does.
pub async fn announce_jobs(
    connection: Connection,
    mut changes: tokio::sync::mpsc::UnboundedReceiver<Announcement>,
) {
    let Ok(emitter) = SignalEmitter::new(&connection, OBJECT_PATH) else {
        log("couldn't set up JobChanged");
        return;
    };
    while let Some(Announcement { job, sent }) = changes.recv().await {
        // A lost one is fine: the next change, `Job` or `Finished` follows.
        let _ = Helper::job_changed(&emitter, job).await;
        // An end: `Finished` may go out now, behind it on the same connection.
        if let Some(sent) = sent {
            let _ = sent.send(());
        }
    }
}

/// Deletes `name` if the fresh `list` has it as a snapshot or a leftover.
fn delete_known(backend: &impl Backend, list: &SnapshotList, name: &str) -> apsis_core::Result<()> {
    let known =
        list.snapshots.iter().any(|s| s.name == name) || list.leftovers.contains(&name.to_owned());
    if !known {
        return Err(Error::NoSuchSnapshot(name.to_owned()));
    }
    backend.delete(name)
}

/// `Stop` needs a password unless the caller's uid started the create.
fn stop_needs_auth(starter: Option<u32>, caller: u32) -> bool {
    starter != Some(caller)
}

/// `error`, or [`Error::DeviceRemoved`] if the backup device's link in `by_uuid`
/// (`/dev/disk/by-uuid`) is gone: the disk left while the job ran, and `error` is what failed
/// because of it.
fn removed_or(error: Error, device: Option<&str>, by_uuid: &Path) -> Error {
    match (&error, device) {
        (Error::DeviceNotFound { .. } | Error::Stopped, _) | (_, None) => error,
        (_, Some(uuid)) if by_uuid.join(uuid).symlink_metadata().is_err() => Error::DeviceRemoved {
            device: uuid.to_owned(),
            reason: error.to_string(),
        },
        _ => error,
    }
}

fn log(line: &str) {
    eprintln!("apsis-helper: {line}");
}

/// One journal line for a `CheckRestore` result: `ok` or the refusal's word, then the lines
/// that aren't refusals.
fn describe_dialog(dialog: &dialog::Dialog) -> String {
    let verdict = dialog
        .refusal
        .as_ref()
        .map_or_else(|| "ok".to_owned(), |r| format!("refused: {}", r.to_wire()));
    format!(
        "{verdict}; home {}, root {}, {}, apsis {}",
        if dialog.has_home { "yes" } else { "no" },
        if dialog.has_root { "yes" } else { "no" },
        if dialog.old_format {
            "old format"
        } else {
            "current format"
        },
        dialog.apsis.to_wire()
    )
}

/// [`log`] for text that may have several lines: one journal line each.
fn log_lines(text: &str) {
    text.lines().for_each(log);
}

/// `ok, 5 snapshots` (plus the free space, leftovers and warnings, when there are any).
fn describe_list(list: &SnapshotList) -> String {
    let mut text = format!("ok, {} snapshots", list.snapshots.len());
    if let Some(usage) = list.usage {
        text.push_str(&format!(
            ", {} of {} bytes free (statvfs)",
            usage.free, usage.total
        ));
    }
    if !list.leftovers.is_empty() {
        text.push_str(&format!(
            "; interrupted creates: {}",
            list.leftovers.join(", ")
        ));
    }
    if !list.warnings.is_empty() {
        text.push_str(&format!("; warnings: {}", list.warnings.join(" | ")));
    }
    text
}

/// One journal line for an error: `refused: ...` when nothing ran, else `failed: ...`.
fn describe_error(error: &Error) -> String {
    match error {
        Error::NotAuthorized
        | Error::Busy
        | Error::InvalidComment(_)
        | Error::InvalidSnapshotName(_)
        | Error::InvalidSettings(_)
        | Error::InvalidConfig(_)
        | Error::InvalidInput(_)
        | Error::NoSuchSnapshot(_)
        | Error::NoSnapshotDevice
        | Error::ConfigChanged => format!("refused: {error}"),
        Error::Stopped => "stopped".to_owned(),
        other => format!("failed: {other}"),
    }
}

/// `DeleteMany`'s names as they go into the journal: `["a", "b", +2 more]`.
fn logged_names(names: &[String]) -> String {
    let shown: Vec<String> = names.iter().take(2).map(|n| format!("{n:?}")).collect();
    let more = names.len().saturating_sub(shown.len());
    if more == 0 {
        format!("[{}]", shown.join(", "))
    } else {
        format!("[{}, +{more} more]", shown.join(", "))
    }
}

/// The comment as it goes into the journal: quoted, and cut to [`LOGGED_COMMENT_CHARS`].
fn logged_comment(comment: &str) -> String {
    let comment = comment.trim();
    let mut short: String = comment.chars().take(LOGGED_COMMENT_CHARS).collect();
    if short.len() < comment.len() {
        short.push('…');
    }
    format!("{short:?}")
}

/// The caller's unique bus name, from the message header the bus filled in.
fn caller(header: &Header<'_>) -> Result<UniqueName<'static>, HelperError> {
    header
        .sender()
        .map(UniqueName::to_owned)
        .ok_or_else(|| HelperError::NotAuthorized("no sender on the call".to_owned()))
}

/// The caller's uid, as the bus knows it (`GetConnectionUnixUser`): who started a create, and
/// who asks to stop it.
async fn unix_user(connection: &Connection, caller: &UniqueName<'_>) -> apsis_core::Result<u32> {
    let bus = DBusProxy::new(connection)
        .await
        .map_err(|e| Error::Helper(format!("bus: {e}")))?;
    bus.get_connection_unix_user(BusName::Unique(caller.clone()))
        .await
        .map_err(|e| Error::Helper(format!("the caller's uid: {e}")))
}

/// polkit's answer for `caller` and `action`. No answer (polkit unreachable, an error) is a
/// no; the reason goes to the journal.
async fn authorize(
    connection: &Connection,
    caller: &UniqueName<'_>,
    action: &str,
    interactive: bool,
) -> apsis_core::Result<()> {
    match polkit::authorized(connection, caller.as_str(), action, interactive).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(Error::NotAuthorized),
        Err(error) => {
            log(&format!(
                "polkit check of {action} for {caller} failed: {error}"
            ));
            Err(Error::NotAuthorized)
        }
    }
}

/// Runs blocking work (rsync, mounts, file I/O) off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> apsis_core::Result<T> + Send + 'static,
) -> apsis_core::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|join| Err(Error::Helper(format!("worker failed: {join}"))))
}

#[cfg(test)]
mod tests {
    use apsis_core::helper::decode_error;
    use apsis_core::helper::names::{
        ERROR_BUSY, ERROR_CHANGED, ERROR_DEVICE_NOT_FOUND, ERROR_FAILED, ERROR_INVALID_INPUT,
        ERROR_NOT_AUTHORIZED, INTERFACE, METHOD_CHECK_RESTORE, METHOD_CREATE, METHOD_DELETE,
        METHOD_DELETE_MANY, METHOD_JOB, METHOD_LIST, METHOD_READ_CONFIG, METHOD_RESTORE,
        METHOD_STOP, METHOD_WRITE_CONFIG, SIGNAL_FINISHED, SIGNAL_JOB_CHANGED,
    };
    use zbus::object_server::Interface;

    use super::*;

    #[test]
    fn interface_matches_the_shared_names() {
        assert_eq!(Helper::name().as_str(), INTERFACE);
        let mut xml = String::new();
        Helper::new(State::new().0).introspect_to_writer(&mut xml, 0);
        for method in [
            METHOD_LIST,
            METHOD_CREATE,
            METHOD_DELETE,
            METHOD_DELETE_MANY,
            METHOD_STOP,
            METHOD_JOB,
            METHOD_READ_CONFIG,
            METHOD_WRITE_CONFIG,
            METHOD_CHECK_RESTORE,
            METHOD_RESTORE,
        ] {
            assert!(
                xml.contains(&format!("<method name=\"{method}\">")),
                "{method}\n{xml}"
            );
        }
        for signal in [SIGNAL_FINISHED, SIGNAL_JOB_CHANGED] {
            assert!(
                xml.contains(&format!("<signal name=\"{signal}\">")),
                "{signal}\n{xml}"
            );
        }
        // Nothing else: exactly ten methods and two signals.
        assert_eq!(xml.matches("<method ").count(), 10, "{xml}");
        // `Restore(s snapshot, b restore_home, b safety_snapshot)`.
        assert!(
            xml.contains("<arg name=\"restore_home\" type=\"b\" direction=\"in\"/>"),
            "{xml}"
        );
        assert!(
            xml.contains("<arg name=\"safety_snapshot\" type=\"b\" direction=\"in\"/>"),
            "{xml}"
        );
        // `CheckRestore(s snapshot) -> (bsbbbs)`.
        assert!(xml.contains("type=\"(bsbbbs)\""), "{xml}");
        // `DeleteMany(as names)`.
        assert!(
            xml.contains("<arg name=\"names\" type=\"as\" direction=\"in\"/>"),
            "{xml}"
        );
        assert_eq!(xml.matches("<signal ").count(), 2, "{xml}");
        // The list with each snapshot's format, its warnings, leftovers and the disk usage.
        assert!(xml.contains("type=\"((sssa(ssss)asas)a{st})\""), "{xml}");
        // The config types.
        assert!(xml.contains("type=\"(s(sbbas)sas)\""), "{xml}");
        assert!(xml.contains("type=\"(sbbas)\""), "{xml}");
        // The job.
        assert!(xml.contains("type=\"(sssxdx)\""), "{xml}");
    }

    #[test]
    fn error_names_match_the_shared_names() {
        let cases = [
            (
                HelperError::NotAuthorized(String::new()),
                ERROR_NOT_AUTHORIZED,
            ),
            (HelperError::Busy(String::new()), ERROR_BUSY),
            (
                HelperError::InvalidInput(String::new()),
                ERROR_INVALID_INPUT,
            ),
            (
                HelperError::DeviceNotFound(String::new()),
                ERROR_DEVICE_NOT_FOUND,
            ),
            (HelperError::Failed(String::new()), ERROR_FAILED),
            (HelperError::Changed(String::new()), ERROR_CHANGED),
        ];
        for (error, name) in cases {
            assert_eq!(zbus::DBusError::name(&error).as_str(), name);
        }
    }

    #[test]
    fn a_missing_disk_reaches_the_client_as_such() {
        let HelperError::DeviceNotFound(message) = HelperError::from(Error::DeviceNotFound {
            device: "00000000-0000-0000-0000-000000000000".to_owned(),
        }) else {
            panic!("not DeviceNotFound")
        };
        assert!(matches!(
            decode_error(&message),
            Error::DeviceNotFound { .. }
        ));
    }

    #[test]
    fn a_disk_that_left_during_the_job_is_named_as_such() {
        let by_uuid = std::env::temp_dir().join(format!("apsis-by-uuid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&by_uuid);
        std::fs::create_dir_all(&by_uuid).unwrap();
        std::os::unix::fs::symlink("../../sdz1", by_uuid.join("here")).unwrap();
        let io = || Error::Native("rsync exited with code 23: Input/output error".to_owned());
        // Still there: the error as it was.
        assert!(matches!(
            removed_or(io(), Some("here"), &by_uuid),
            Error::Native(_)
        ));
        // Gone: removed, with the reason kept.
        let removed = removed_or(io(), Some("gone"), &by_uuid);
        assert!(
            matches!(&removed, Error::DeviceRemoved { device, reason }
                if device == "gone" && reason.contains("Input/output")),
            "{removed:?}"
        );
        // Travels as a failure the client decodes.
        let HelperError::Failed(message) = HelperError::from(removed) else {
            panic!("not Failed")
        };
        assert!(matches!(
            decode_error(&message),
            Error::DeviceRemoved { .. }
        ));
        // A stop stays a stop; no device, nothing to say.
        assert!(matches!(
            removed_or(Error::Stopped, Some("gone"), &by_uuid),
            Error::Stopped
        ));
        assert!(matches!(removed_or(io(), None, &by_uuid), Error::Native(_)));
        std::fs::remove_dir_all(&by_uuid).unwrap();
    }

    #[test]
    fn only_the_starters_uid_stops_without_a_password() {
        assert!(!stop_needs_auth(Some(1000), 1000));
        assert!(stop_needs_auth(Some(1000), 1001));
        assert!(stop_needs_auth(None, 1000));
        assert!(
            stop_needs_auth(Some(1000), 0),
            "root isn't the starter either"
        );
    }

    #[test]
    fn input_errors_are_invalid_input() {
        for error in [
            Error::InvalidComment("too long"),
            Error::InvalidSnapshotName("x".to_owned()),
            Error::NoSuchSnapshot("2001-01-01_00-00-00".to_owned()),
            Error::InvalidSettings("choose a backup device".to_owned()),
            Error::InvalidInput("not deleting x: no info.json".to_owned()),
        ] {
            assert!(matches!(
                HelperError::from(error),
                HelperError::InvalidInput(_)
            ));
        }
    }

    #[test]
    fn a_stopped_delete_of_several_travels_as_a_failure_with_its_parts() {
        let stopped = Error::DeleteManyStopped {
            deleted: vec!["2026-09-19_09-29-57".to_owned()],
            failed: "2026-09-20_10-00-00".to_owned(),
            left: vec!["2026-09-25_11-28-53".to_owned()],
            reason: Box::new(Error::NoSuchSnapshot("2026-09-20_10-00-00".to_owned())),
        };
        assert_eq!(
            describe_error(&stopped),
            "failed: delete stopped at 2026-09-20_10-00-00: no snapshot called \
             \"2026-09-20_10-00-00\" on the backup device"
        );
        let HelperError::Failed(message) = HelperError::from(stopped) else {
            panic!("not Failed")
        };
        assert!(matches!(
            decode_error(&message),
            Error::DeleteManyStopped { deleted, failed, left, .. }
                if deleted.len() == 1 && failed == "2026-09-20_10-00-00" && left.len() == 1
        ));
    }

    #[test]
    fn journal_gets_short_names_lists() {
        let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            logged_names(&names(&["2026-09-19_09-29-57", "2026-09-20_10-00-00"])),
            "[\"2026-09-19_09-29-57\", \"2026-09-20_10-00-00\"]"
        );
        assert_eq!(
            logged_names(&names(&[
                "2026-09-19_09-29-57",
                "2026-09-20_10-00-00",
                "2026-09-25_11-28-53",
                "2026-09-26_14-02-11"
            ])),
            "[\"2026-09-19_09-29-57\", \"2026-09-20_10-00-00\", +2 more]"
        );
    }

    #[test]
    fn journal_gets_short_comments_and_one_line_errors() {
        assert_eq!(logged_comment(" before update "), "\"before update\"");
        let long = "x".repeat(200);
        let logged = logged_comment(&long);
        assert_eq!(
            logged.chars().filter(|&c| c == 'x').count(),
            LOGGED_COMMENT_CHARS
        );
        assert!(logged.ends_with("…\""));

        let failed = Error::Native("rsync exited with code 11: No space left".to_owned());
        assert_eq!(
            describe_error(&failed),
            "failed: rsync exited with code 11: No space left"
        );
        assert_eq!(
            describe_error(&Error::Busy),
            format!("refused: {}", Error::Busy)
        );
        assert_eq!(describe_error(&Error::Stopped), "stopped");

        let mut list = SnapshotList::default();
        assert_eq!(describe_list(&list), "ok, 0 snapshots");
        list.warnings = vec!["x: incomplete: no info.json".to_owned()];
        assert_eq!(
            describe_list(&list),
            "ok, 0 snapshots; warnings: x: incomplete: no info.json"
        );
        list.warnings.clear();
        list.leftovers = vec!["2026-09-29_14-02-11".to_owned()];
        assert_eq!(
            describe_list(&list),
            "ok, 0 snapshots; interrupted creates: 2026-09-29_14-02-11"
        );
        list.leftovers.clear();
        list.usage = apsis_core::DiskUsage::from_statvfs(1000, 400, 350, 1000);
        assert_eq!(
            describe_list(&list),
            "ok, 0 snapshots, 350000 of 1000000 bytes free (statvfs)"
        );
    }
}
