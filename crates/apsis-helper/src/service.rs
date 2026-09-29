// SPDX-License-Identifier: GPL-3.0-only

//! The D-Bus interface: `NativeListWithUsage`, `NativeCreate`, `Delete`, `ReadConfig`,
//! `WriteConfig`, file-level restore's `Browse` and `Restore`, and the `Progress` and
//! `Finished` signals. Nothing else.
//!
//! Every call is logged with its result on stderr, which systemd puts in the journal
//! (`journalctl -u apsis-helper`). Comments are cut to [`LOGGED_COMMENT_CHARS`].

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apsis_core::helper::names::{
    ACTION_BROWSE, ACTION_CONFIGURE, ACTION_CREATE, ACTION_DELETE, ACTION_LIST, ACTION_RESTORE,
    ACTION_RESTORE_ORIGINAL, OBJECT_PATH, OP_CREATE, OP_DELETE, OP_RESTORE,
};
use apsis_core::helper::{
    WireConfig, WireConfigInfo, WireListWithUsage, WireListing, config_from_wire, encode_error,
    listing_to_wire, to_wire_with_usage,
};
use apsis_core::progress::Throttle;
use apsis_core::restore::{Destination, Request, SnapPath, check_paths};
use apsis_core::{Backend, Error, Progress, SnapshotList, parse_snapshot_name, validate_comment};
use tokio::sync::mpsc;
use zbus::fdo::DBusProxy;
use zbus::message::Header;
use zbus::names::{BusName, UniqueName};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, DBusError, interface};

use crate::native::{self, Access};
use crate::polkit;
use crate::restore::{self, logged_path, logged_paths};
use crate::runner::DirectRunner;
use crate::settings::{self, Files};
use crate::state::{Running, State};
use crate::usage;

/// Longest part of a comment that goes into the journal.
const LOGGED_COMMENT_CHARS: usize = 40;

/// The helper object at [`OBJECT_PATH`].
pub struct Helper {
    state: Arc<State>,
}

impl Helper {
    pub fn new(state: Arc<State>) -> Self {
        Self { state }
    }
}

/// The helper's D-Bus errors, named `<ERROR_PREFIX>.<Variant>` (see `apsis_core::helper::names`).
#[derive(Debug, DBusError)]
#[zbus(prefix = "io.github.atraxsrc.Apsis.Helper1.Error")]
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

#[interface(name = "io.github.atraxsrc.Apsis.Helper1")]
impl Helper {
    /// Starts deleting snapshot `name` (polkit: `delete`) and returns; `Finished("delete", ..)`
    /// follows. The backup device is mounted read-write for the delete only; the name must be
    /// one the list has, and the folder a plain snapshot folder (see
    /// `NativeRsync::delete_snapshot`).
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
            self.state.begin()
        }
        .await;
        self.start(
            connection,
            caller,
            OP_DELETE,
            label,
            started,
            move |_running, _| {
                let (backend, _mounted) =
                    native::open(&DirectRunner, Access::ReadWrite, log_lines)?;
                if !backend.list()?.snapshots.iter().any(|s| s.name == name) {
                    return Err(Error::NoSuchSnapshot(name.clone()));
                }
                backend.delete_snapshot(&name).map(|()| String::new())
            },
        )
    }

    /// The snapshots on the backup device (each one's `info.json`, the device mounted
    /// read-only for the call), plus its `statvfs` (polkit: `list`, no password for the active
    /// session).
    async fn native_list_with_usage(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<WireListWithUsage, HelperError> {
        Ok(to_wire_with_usage(
            &self.native_snapshot_list(&header, connection).await?,
        ))
    }

    /// Starts a snapshot (polkit: `create`) and returns; `Finished("create", ..)` follows.
    /// rsync runs at idle I/O priority and nice 19.
    async fn native_create(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        comment: String,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("native create {} for {caller}", logged_comment(&comment));
        let started = async {
            validate_comment(&comment)?;
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_CREATE, true).await?;
            self.state.begin()
        }
        .await;
        self.start(
            connection,
            caller,
            OP_CREATE,
            label,
            started,
            move |_running, progress| {
                progress.started();
                let (backend, _mounted) =
                    native::open(&DirectRunner, Access::ReadWrite, log_lines)?;
                let progress = progress.clone();
                let backend = backend.with_progress(move |p| progress.report(p));
                backend.create(&comment).map(|()| String::new())
            },
        )
    }

    /// Apsis's config (or, before it's saved, the import from Timeshift's settings), the block
    /// devices and the users, for the settings view (polkit: `list`, no password for the active
    /// session).
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
            Ok(info) if !info.4.is_empty() => {
                log(&format!(
                    "read config for {caller}: imported from Timeshift's settings"
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
            let running = self.state.begin()?;
            blocking(move || {
                let _running = running;
                Files::system().write(&DirectRunner, &expected, &config)
            })
            .await
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

    /// One folder of snapshot `snapshot`, each entry compared with the running system (polkit:
    /// `browse`, password cached: snapshots hold root-only files). The backup device is mounted
    /// read-only for the call.
    async fn browse(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        snapshot: String,
        path: String,
    ) -> Result<WireListing, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!("browse {snapshot:?} {} for {caller}", logged_path(&path));
        let result = async {
            if parse_snapshot_name(&snapshot).is_none() {
                return Err(Error::InvalidSnapshotName(snapshot.clone()));
            }
            SnapPath::parse(&path)?;
            self.refuse_if_running()?;
            authorize(connection, &caller, ACTION_BROWSE, true).await?;
            let running = self.state.begin()?;
            blocking(move || {
                let _running = running;
                restore::browse(&snapshot, &path)
            })
            .await
        }
        .await;
        match &result {
            Ok(listing) => log(&format!(
                "{label}: ok, {} entries{}",
                listing.entries.len(),
                if listing.truncated { ", truncated" } else { "" }
            )),
            Err(error) => log(&format!("{label}: {}", describe_error(error))),
        }
        Ok(listing_to_wire(&result?))
    }

    /// Copies `paths` of snapshot `snapshot` back (`destination`: `folder` or `original`), or
    /// with `dry_run` only works out what that would do. Returns once started;
    /// `Finished("restore", ok, text)` follows, the text being the plan or the result.
    ///
    /// polkit: a dry run `browse`; folder mode `restore` (cached); original mode
    /// `restore-original` (asked every time). Refused while another operation runs.
    async fn restore(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        snapshot: String,
        paths: Vec<String>,
        destination: String,
        dry_run: bool,
    ) -> Result<(), HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let label = format!(
            "restore {destination:?}{} {snapshot:?} {} for {caller}",
            if dry_run { " dry run" } else { "" },
            logged_paths(&paths)
        );
        let started = async {
            let destination = Destination::from_word(&destination).ok_or_else(|| {
                Error::InvalidInput(format!("unknown destination {destination:?}"))
            })?;
            if parse_snapshot_name(&snapshot).is_none() {
                return Err(Error::InvalidSnapshotName(snapshot.clone()));
            }
            check_paths(&paths)?;
            let uid = unix_user(connection, &caller).await?;
            let action = match (dry_run, destination) {
                (true, _) => ACTION_BROWSE,
                (false, Destination::Folder) => ACTION_RESTORE,
                (false, Destination::Original) => ACTION_RESTORE_ORIGINAL,
            };
            self.refuse_if_running()?;
            authorize(connection, &caller, action, true).await?;
            let running = self.state.begin()?;
            let request = Request {
                snapshot: snapshot.clone(),
                paths: paths.clone(),
                destination,
                dry_run,
            };
            Ok((running, request, uid))
        }
        .await;
        let (started, job) = match started {
            Ok((running, request, uid)) => (Ok(running), Some((request, uid))),
            Err(error) => (Err(error), None),
        };
        let summary_label = label.clone();
        self.start(
            connection,
            caller,
            OP_RESTORE,
            label,
            started,
            move |_running, progress| {
                let (request, uid) = job.ok_or_else(|| Error::Helper("not started".to_owned()))?;
                if !request.dry_run {
                    progress.started();
                }
                let plan = restore::run(&request, uid, &|p| progress.report(p))?;
                log(&format!("{summary_label}: uid {uid}: {}", plan.summary()));
                Ok(plan.to_string())
            },
        )
    }

    /// How far a create or restore is, at most about twice a second (see [`Progress`]); sent
    /// only to the caller that started it, never after its `Finished`. `percent` and
    /// `eta_seconds` are `-1` while unknown.
    #[zbus(signal)]
    async fn progress(
        emitter: &SignalEmitter<'_>,
        op: &str,
        percent: f64,
        eta_seconds: i64,
        text: &str,
    ) -> zbus::Result<()>;

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
    /// The native list, with `statvfs` of [`native::MOUNT_POINT`] while it's mounted, logged.
    async fn native_snapshot_list(
        &self,
        header: &Header<'_>,
        connection: &Connection,
    ) -> Result<SnapshotList, HelperError> {
        let _call = self.state.call();
        let caller = caller(header)?;
        let label = format!("list for {caller}");
        let result = async {
            authorize(connection, &caller, ACTION_LIST, false).await?;
            let running = self.state.begin()?;
            blocking(move || {
                let _running = running;
                let (backend, _mounted) = native::open(&DirectRunner, Access::ReadOnly, log_lines)?;
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
        Ok(result?)
    }

    /// Refuses before the password dialog, so nobody types a password for a call that can't
    /// run. [`State::begin`] still decides, after polkit, if two calls race.
    fn refuse_if_running(&self) -> apsis_core::Result<()> {
        if self.state.is_running() {
            return Err(Error::Busy);
        }
        Ok(())
    }

    /// Logs whether the operation could start. If it did, runs `work` in the background
    /// holding the lock, then logs how it went and tells `caller` with `Finished`: the text
    /// `work` returned, or the error.
    fn start(
        &self,
        connection: &Connection,
        caller: UniqueName<'static>,
        op: &'static str,
        label: String,
        started: apsis_core::Result<Running>,
        work: impl FnOnce(&Running, &ProgressSink) -> apsis_core::Result<String> + Send + 'static,
    ) -> Result<(), HelperError> {
        let running = match started {
            Ok(running) => running,
            Err(error) => {
                log(&format!("{label}: {}", describe_error(&error)));
                return Err(error.into());
            }
        };
        log(&format!("{label}: started"));
        // Keeps the helper alive until the signal is out.
        let call = self.state.call();
        let connection = connection.clone();
        let (sink, mut updates) = ProgressSink::new();
        let progress_to = (connection.clone(), caller.clone());
        // Progress goes out from its own task, so a slow bus never holds up the work.
        let forward = tokio::spawn(async move {
            let (connection, caller) = progress_to;
            let Ok(emitter) = SignalEmitter::new(&connection, OBJECT_PATH) else {
                return;
            };
            let emitter = emitter.set_destination(caller.into());
            while let Some(p) = updates.recv().await {
                let percent = p.percent.unwrap_or(-1.0);
                let eta = p
                    .eta_seconds
                    .and_then(|s| i64::try_from(s).ok())
                    .unwrap_or(-1);
                // A lost update is fine; the next one or `Finished` follows.
                let _ = Helper::progress(&emitter, op, percent, eta, &p.text).await;
            }
        });
        tokio::spawn(async move {
            let _call = call;
            // `running` drops when `work` returns, so the lock is free before `Finished`
            // arrives and the caller's refresh isn't refused as busy. The sink drops with the
            // closure, which ends `forward`; it's awaited so no progress follows `Finished`.
            let result = blocking(move || work(&running, &sink)).await;
            let _ = forward.await;
            let (ok, message) = match result {
                Ok(text) => {
                    log(&format!("{label}: done"));
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

/// Where an operation's work reports its progress: at most one update per
/// [`PROGRESS_INTERVAL`] (the last, `100%`, always) goes to the task that sends the
/// `Progress` signal. Never blocks.
#[derive(Clone)]
pub struct ProgressSink {
    updates: mpsc::UnboundedSender<Progress>,
    throttle: Arc<Mutex<Throttle>>,
}

/// About two updates a second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

impl ProgressSink {
    fn new() -> (Self, mpsc::UnboundedReceiver<Progress>) {
        let (updates, receiver) = mpsc::unbounded_channel();
        let throttle = Arc::new(Mutex::new(Throttle::new(PROGRESS_INTERVAL)));
        (Self { updates, throttle }, receiver)
    }

    /// A first update with no numbers: tells the caller progress will come (the applet shows
    /// "estimating…" until rsync has a number).
    pub fn started(&self) {
        self.report(Progress {
            percent: None,
            eta_seconds: None,
            text: "started".to_owned(),
        });
    }

    pub fn report(&self, progress: Progress) {
        let done = progress.percent.is_some_and(|p| p >= 100.0);
        let ready = self
            .throttle
            .lock()
            .map_or(true, |mut throttle| throttle.ready(Instant::now()));
        if ready || done {
            // The receiver is gone only once the operation is over.
            let _ = self.updates.send(progress);
        }
    }
}

fn log(line: &str) {
    eprintln!("apsis-helper: {line}");
}

/// [`log`] for text that may have several lines (a dry run's plan): one journal line each.
fn log_lines(text: &str) {
    text.lines().for_each(log);
}

/// `ok, 5 snapshots` (plus the free space and the warnings, when known).
fn describe_list(list: &SnapshotList) -> String {
    let mut text = format!("ok, {} snapshots", list.snapshots.len());
    if let Some(usage) = list.usage {
        text.push_str(&format!(
            ", {} of {} bytes free (statvfs)",
            usage.free, usage.total
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
        other => format!("failed: {other}"),
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

/// The caller's uid, as the bus knows it (`GetConnectionUnixUser`): folder mode restores into
/// that user's home, as that user.
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
        ERROR_NOT_AUTHORIZED, INTERFACE, METHOD_BROWSE, METHOD_DELETE, METHOD_NATIVE_CREATE,
        METHOD_NATIVE_LIST_WITH_USAGE, METHOD_READ_CONFIG, METHOD_RESTORE, METHOD_WRITE_CONFIG,
        SIGNAL_FINISHED, SIGNAL_PROGRESS,
    };
    use zbus::object_server::Interface;

    use super::*;

    #[test]
    fn interface_matches_the_shared_names() {
        assert_eq!(Helper::name().as_str(), INTERFACE);
        let mut xml = String::new();
        Helper::new(State::new()).introspect_to_writer(&mut xml, 0);
        for method in [
            METHOD_NATIVE_LIST_WITH_USAGE,
            METHOD_NATIVE_CREATE,
            METHOD_DELETE,
            METHOD_READ_CONFIG,
            METHOD_WRITE_CONFIG,
            METHOD_BROWSE,
            METHOD_RESTORE,
        ] {
            assert!(
                xml.contains(&format!("<method name=\"{method}\">")),
                "{method}\n{xml}"
            );
        }
        for signal in [SIGNAL_FINISHED, SIGNAL_PROGRESS] {
            assert!(
                xml.contains(&format!("<signal name=\"{signal}\">")),
                "{signal}\n{xml}"
            );
        }
        // Progress(s op, d percent, x eta_seconds, s text).
        for arg in [
            "<arg name=\"percent\" type=\"d\"/>",
            "<arg name=\"eta_seconds\" type=\"x\"/>",
        ] {
            assert!(xml.contains(arg), "{arg}\n{xml}");
        }
        // Nothing else: exactly seven methods and two signals.
        assert_eq!(xml.matches("<method ").count(), 7, "{xml}");
        assert_eq!(xml.matches("<signal ").count(), 2, "{xml}");
        // The list with its warnings and the disk usage.
        assert!(xml.contains("type=\"((sssa(sss)as)a{st})\""), "{xml}");
        // The config types.
        assert!(xml.contains("type=\"(s(sas)sa(ssb)as)\""), "{xml}");
        assert!(xml.contains("type=\"(sas)\""), "{xml}");
        // Browse's listing, and Restore's paths.
        assert!(xml.contains("type=\"(a(sstxuuussstx)b)\""), "{xml}");
        assert!(xml.contains("type=\"as\""), "{xml}");
    }

    #[tokio::test]
    async fn progress_is_throttled_but_the_end_always_gets_through() {
        let (sink, mut updates) = ProgressSink::new();
        let at = |percent: f64| Progress {
            percent: Some(percent),
            eta_seconds: None,
            text: String::new(),
        };
        for percent in [1.0, 2.0, 3.0, 100.0] {
            sink.report(at(percent));
        }
        drop(sink);
        let mut got = Vec::new();
        while let Some(p) = updates.recv().await {
            got.extend(p.percent);
        }
        // The first, then nothing within 500 ms, except the end.
        assert_eq!(got, [1.0, 100.0]);
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
    fn input_errors_are_invalid_input() {
        for error in [
            Error::InvalidComment("too long"),
            Error::InvalidSnapshotName("x".to_owned()),
            Error::NoSuchSnapshot("2001-01-01_00-00-00".to_owned()),
            Error::InvalidSettings("choose a backup device".to_owned()),
            Error::InvalidInput("/proc/x: never restored".to_owned()),
        ] {
            assert!(matches!(
                HelperError::from(error),
                HelperError::InvalidInput(_)
            ));
        }
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

        let mut list = SnapshotList::default();
        assert_eq!(describe_list(&list), "ok, 0 snapshots");
        list.warnings = vec!["x: incomplete: no info.json".to_owned()];
        assert_eq!(
            describe_list(&list),
            "ok, 0 snapshots; warnings: x: incomplete: no info.json"
        );
        list.warnings.clear();
        list.usage = apsis_core::DiskUsage::from_statvfs(1000, 400, 350, 1000);
        assert_eq!(
            describe_list(&list),
            "ok, 0 snapshots, 350000 of 1000000 bytes free (statvfs)"
        );
    }
}
