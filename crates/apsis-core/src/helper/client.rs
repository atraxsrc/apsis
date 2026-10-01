// SPDX-License-Identifier: GPL-3.0-only

use futures_util::{Stream, StreamExt, stream};
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::{Connection, Proxy};

use std::future::Future;

use super::names::{
    BUS_NAME, ERROR_BUSY, ERROR_CHANGED, ERROR_DEVICE_NOT_FOUND, ERROR_FAILED, ERROR_INVALID_INPUT,
    ERROR_NOT_AUTHORIZED, INTERFACE, METHOD_CANCEL_RESTORE, METHOD_CHECK_RESTORE, METHOD_CREATE,
    METHOD_DELETE, METHOD_DELETE_MANY, METHOD_JOB, METHOD_LIST, METHOD_READ_CONFIG,
    METHOD_RESTART_TO_RESTORE, METHOD_RESTORE, METHOD_RESTORE_RESULT, METHOD_STOP,
    METHOD_WRITE_CONFIG, OBJECT_PATH, OP_CREATE, OP_DELETE, OP_DELETE_MANY, OP_RESTORE,
    SIGNAL_FINISHED, SIGNAL_JOB_CHANGED,
};
use super::{
    WireCheckRestore, WireConfigInfo, WireListWithUsage3, WireRestoreResult, check_delete_many,
    config_info_from_wire, config_to_wire, decode_error, from_wire_with_usage3,
};
use crate::config::{Config, ConfigInfo};
use crate::error::{Error, Result};
use crate::job::{self, Job, WireJob};
use crate::model::SnapshotList;
use crate::progress::Progress;
use crate::restore::dialog::{self, Dialog};
use crate::restore::state::RestoreResult;

/// The applet's side of `apsis-helper`, on the system bus.
///
/// One per process: it holds the bus connection, so every call from a process comes from
/// one bus name (clones share it). Needs a tokio runtime (zbus runs on the caller's tokio).
#[derive(Debug, Clone)]
pub struct HelperClient {
    connection: Connection,
}

impl HelperClient {
    /// Connects if the helper is installed (D-Bus can start it) or already running. `None`
    /// when it isn't, or the system bus can't be reached: Apsis can't do anything then.
    /// Made once per process and kept: each call opens a connection, with two round trips.
    pub async fn connect() -> Option<Self> {
        let connection = Connection::system().await.ok()?;
        let bus = DBusProxy::new(&connection).await.ok()?;
        let activatable = bus.list_activatable_names().await.ok()?;
        let installed = activatable.iter().any(|name| name.as_str() == BUS_NAME);
        let running = || async {
            let name = BusName::try_from(BUS_NAME).ok()?;
            bus.name_has_owner(name).await.ok()
        };
        (installed || running().await == Some(true)).then_some(Self { connection })
    }

    /// Lists snapshots, with the backup device's disk usage. No password for the active
    /// session.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]), or a bad reply.
    pub async fn list(&self) -> Result<SnapshotList> {
        let wire: WireListWithUsage3 = self
            .proxy()
            .await?
            .call(METHOD_LIST, &())
            .await
            .map_err(from_zbus)?;
        from_wire_with_usage3(wire)
    }

    /// Whether the snapshot `name` can be restored on this computer, and what the Restore
    /// dialog says ([`Dialog`]). No password: the helper mounts read-only, as for a list.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]; `Busy` while a write runs), or a bad reply.
    pub async fn check_restore(&self, name: &str) -> Result<Dialog> {
        let wire: WireCheckRestore = self
            .proxy()
            .await?
            .call(METHOD_CHECK_RESTORE, &(name,))
            .await
            .map_err(from_zbus)?;
        dialog::from_wire(wire)
    }

    /// Prepares a full-system restore of the snapshot `name` and waits until the plan is
    /// ready at the prompt (the checks, the dry runs, the safety snapshot: minutes). The
    /// password is asked every time. `on_progress` gets the safety snapshot's progress.
    /// Afterwards the helper refuses writes until `RestartToRestore` or `CancelRestore`.
    ///
    /// # Errors
    ///
    /// [`Error::RestoreRefused`] with the refusal's word, [`Error::Stopped`], or what the
    /// helper reported (see [`Error`]).
    pub async fn restore(
        &self,
        name: &str,
        restore_home: bool,
        safety_snapshot: bool,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<()> {
        let name = name.to_owned();
        self.operate(
            OP_RESTORE,
            move |proxy| async move {
                proxy
                    .call::<_, _, ()>(
                        METHOD_RESTORE,
                        &(name.as_str(), restore_home, safety_snapshot),
                    )
                    .await
            },
            on_progress,
        )
        .await
        .map(drop)
    }

    /// "Restart now" for the ready plan of the snapshot `name`: the helper re-checks, arms
    /// the next boot and asks logind to restart. Returns once the restart is requested. No
    /// password for the uid that prepared the plan.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] with `restore::plan::TOO_OLD` or `restore::plan::GONE`,
    /// [`Error::RestoreRefused`] with the refusal's word, or what the helper reported. After
    /// any of them the plan is gone.
    pub async fn restart_to_restore(&self, name: &str) -> Result<()> {
        self.proxy()
            .await?
            .call::<_, _, ()>(METHOD_RESTART_TO_RESTORE, &(name,))
            .await
            .map_err(from_zbus)
    }

    /// "Cancel restore" at the ready prompt: the helper removes the plan (nothing is armed
    /// yet; a finished safety snapshot stays). No password for the uid that prepared it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] with `restore::plan::GONE` when there's no plan, or what the
    /// helper reported.
    pub async fn cancel_restore(&self) -> Result<()> {
        self.proxy()
            .await?
            .call::<_, _, ()>(METHOD_CANCEL_RESTORE, &())
            .await
            .map_err(from_zbus)
    }

    /// Where the restore stands: a plan ready at the prompt, the last result, or nothing.
    /// No password. The window asks when it opens.
    ///
    /// # Errors
    ///
    /// What the helper reported, or a bad reply.
    pub async fn restore_result(&self) -> Result<RestoreResult> {
        let wire: WireRestoreResult = self
            .proxy()
            .await?
            .call(METHOD_RESTORE_RESULT, &())
            .await
            .map_err(from_zbus)?;
        RestoreResult::from_wire(wire)
    }

    /// Creates a snapshot and waits until it's done (this can take minutes).
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn create(&self, comment: &str) -> Result<()> {
        self.create_with_progress(comment, &mut |_| {}).await
    }

    /// [`HelperClient::create`], handing its progress (from `JobChanged`) to `on_progress`.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn create_with_progress(
        &self,
        comment: &str,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<()> {
        self.operate_on(METHOD_CREATE, OP_CREATE, comment, on_progress)
            .await
    }

    /// Deletes the snapshot `name` (or the interrupted create's folder `name`) and waits until
    /// it's done.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn delete(&self, name: &str) -> Result<()> {
        self.operate_on(METHOD_DELETE, OP_DELETE, name, &mut |_| {})
            .await
    }

    /// Deletes `names` (snapshots or interrupted creates' folders) as one job, in order, and
    /// waits until it's done. The password is asked once. It stops at the first failure:
    /// [`Error::DeleteManyStopped`] says what was deleted, what failed and why, and what's
    /// left. `on_progress` gets each step (`percent` is done of total).
    ///
    /// # Errors
    ///
    /// The names aren't two or more distinct snapshot names ([`check_delete_many`]), or what
    /// the helper reported.
    pub async fn delete_many(
        &self,
        names: &[String],
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<()> {
        check_delete_many(names)?;
        let names = names.to_vec();
        self.operate(
            OP_DELETE_MANY,
            move |proxy| async move { proxy.call::<_, _, ()>(METHOD_DELETE_MANY, &(names,)).await },
            on_progress,
        )
        .await
        .map(drop)
    }

    /// Stops the running create if it is making `snapshot`. Returns once stopping has begun;
    /// the create's own call ends with [`Error::Stopped`]. No password for the user who
    /// started it; anyone else is asked.
    ///
    /// # Errors
    ///
    /// Nothing to stop, another snapshot, too late ([`Error::InvalidInput`]), or not allowed.
    pub async fn stop(&self, snapshot: &str) -> Result<()> {
        self.proxy()
            .await?
            .call(METHOD_STOP, &(snapshot,))
            .await
            .map_err(from_zbus)
    }

    /// What the helper is doing now; `None` when it's idle. Doesn't start the helper just to
    /// ask: if it isn't running, it's idle.
    ///
    /// # Errors
    ///
    /// What the helper reported, or a bad reply.
    pub async fn job(&self) -> Result<Option<Job>> {
        let bus = DBusProxy::new(&self.connection).await.map_err(from_zbus)?;
        let name = BusName::try_from(BUS_NAME).map_err(|e| Error::Helper(e.to_string()))?;
        let running = bus
            .name_has_owner(name)
            .await
            .map_err(|e| Error::Helper(e.to_string()))?;
        if !running {
            return Ok(None);
        }
        let wire: WireJob = self
            .proxy()
            .await?
            .call(METHOD_JOB, &())
            .await
            .map_err(from_zbus)?;
        Ok(job::from_wire(wire))
    }

    /// Every change to the helper's job, from anyone's: [`JobEvent::Changed`] for each
    /// `JobChanged`, [`JobEvent::HelperGone`] when the helper leaves the bus. Subscribing
    /// doesn't start the helper.
    ///
    /// # Errors
    ///
    /// The subscription couldn't be made.
    pub async fn job_changes(&self) -> Result<impl Stream<Item = JobEvent> + use<>> {
        let proxy = self.proxy().await?;
        let changes = proxy
            .receive_signal(SIGNAL_JOB_CHANGED)
            .await
            .map_err(from_zbus)?
            .filter_map(|message| async move {
                let (wire,) = message.body().deserialize::<(WireJob,)>().ok()?;
                job::from_wire(wire).map(JobEvent::Changed)
            });
        let gone = proxy
            .receive_owner_changed()
            .await
            .map_err(from_zbus)?
            .filter_map(|owner| async move { owner.is_none().then_some(JobEvent::HelperGone) });
        Ok(stream::select(changes, gone))
    }

    /// Apsis's config (converted from the old format, or imported from Timeshift's settings,
    /// until it's saved) and the devices. No password for the active session.
    ///
    /// # Errors
    ///
    /// What the helper reported, or a config file that can't be read.
    pub async fn read_config(&self) -> Result<ConfigInfo> {
        let wire: WireConfigInfo = self
            .proxy()
            .await?
            .call(METHOD_READ_CONFIG, &())
            .await
            .map_err(from_zbus)?;
        config_info_from_wire(wire)
    }

    /// Writes `config` if `config.toml` still reads `expected` (empty: there's none yet). Asks
    /// for the password. The text is a note, or empty.
    ///
    /// # Errors
    ///
    /// What the helper reported: [`Error::ConfigChanged`], [`Error::InvalidSettings`], ...
    pub async fn write_config(&self, expected: &str, config: &Config) -> Result<String> {
        self.proxy()
            .await?
            .call(METHOD_WRITE_CONFIG, &(expected, config_to_wire(config)))
            .await
            .map_err(from_zbus)
    }

    async fn proxy(&self) -> Result<Proxy<'static>> {
        Proxy::new(&self.connection, BUS_NAME, OBJECT_PATH, INTERFACE)
            .await
            .map_err(from_zbus)
    }

    /// [`HelperClient::operate`] for `method(argument)`, whose `Finished` carries no text.
    async fn operate_on(
        &self,
        method: &str,
        op: &str,
        argument: &str,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<()> {
        let (method, argument) = (method.to_owned(), argument.to_owned());
        self.operate(
            op,
            move |proxy| async move { proxy.call::<_, _, ()>(method.as_str(), &(argument,)).await },
            on_progress,
        )
        .await
        .map(drop)
    }

    /// Starts an operation with `start`, then waits for its `Finished` (returning its message),
    /// or for the helper to leave the bus without sending one. Its progress (`JobChanged` for
    /// the same kind of job) goes to `on_progress` meanwhile.
    async fn operate<F, Fut>(
        &self,
        op: &str,
        start: F,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<String>
    where
        F: FnOnce(Proxy<'static>) -> Fut,
        Fut: Future<Output = zbus::Result<()>>,
    {
        let proxy = self.proxy().await?;
        // Subscribe before starting, so a quick `Finished` isn't missed.
        let finished = proxy
            .receive_signal(SIGNAL_FINISHED)
            .await
            .map_err(from_zbus)?
            .map(|message| {
                message
                    .body()
                    .deserialize::<(String, bool, String)>()
                    .map_err(|e| Error::Helper(format!("bad {SIGNAL_FINISHED} signal: {e}")))
            });
        // Undecodable ones are skipped: progress is only for show.
        let progress = proxy
            .receive_signal(SIGNAL_JOB_CHANGED)
            .await
            .map_err(from_zbus)?
            .filter_map(|message| async move {
                let ((kind, state, _, _, percent, eta),) =
                    message.body().deserialize::<(WireJob,)>().ok()?;
                // The end is `Finished`'s to say.
                (state == "running").then_some((kind, percent, eta, String::new()))
            });
        let gone = proxy
            .receive_owner_changed()
            .await
            .map_err(from_zbus)?
            .filter_map(|owner| async move { owner.is_none().then_some(()) });
        // The call returns once the helper has checked the input and polkit and started.
        start(proxy.clone()).await.map_err(from_zbus)?;
        wait_for_finished(op, finished, progress, gone, on_progress).await
    }
}

/// A job's progress: `(kind, percent, eta_seconds, text)`.
type WireProgress = (String, f64, i64, String);

/// What [`HelperClient::job_changes`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    /// A job started, got further, is stopping, or ended (see [`crate::job::JobState`]).
    Changed(Job),
    /// The helper left the bus: whatever it was doing is over.
    HelperGone,
}

/// What the helper told us while an operation ran.
enum Event {
    Finished(Result<(String, bool, String)>),
    Progress(WireProgress),
    /// The signal stream ended (the connection closed).
    Closed,
    Gone,
}

/// Waits for `Finished(op, ok, message)` for `op`: its message. Fails if the helper leaves the
/// bus (`gone`) or the signal stream ends first. `Progress` for `op` goes to `on_progress`.
async fn wait_for_finished(
    op: &str,
    finished: impl Stream<Item = Result<(String, bool, String)>>,
    progress: impl Stream<Item = WireProgress>,
    gone: impl Stream<Item = ()>,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<String> {
    // `select` only ends when both streams do, and `gone` never does: mark the end of
    // `finished` with `Closed` instead.
    let finished = finished
        .map(Event::Finished)
        .chain(stream::once(async { Event::Closed }));
    let others = stream::select(progress.map(Event::Progress), gone.map(|()| Event::Gone));
    let events = stream::select(finished, others);
    let mut events = std::pin::pin!(events);
    while let Some(event) = events.next().await {
        match event {
            Event::Finished(Ok((finished_op, ok, message))) if finished_op == op => {
                return if ok {
                    Ok(message)
                } else {
                    Err(decode_error(&message))
                };
            }
            // Another operation's signal (e.g. a delete's, arriving late): not ours.
            Event::Finished(Ok(_)) => {}
            Event::Progress((progress_op, percent, eta, text)) if progress_op == op => {
                on_progress(Progress {
                    percent: (0.0..=100.0).contains(&percent).then_some(percent),
                    eta_seconds: u64::try_from(eta).ok(),
                    text,
                });
            }
            Event::Progress(_) => {}
            Event::Finished(Err(error)) => return Err(error),
            Event::Gone => {
                return Err(Error::Helper(
                    "the helper stopped before it finished".to_owned(),
                ));
            }
            Event::Closed => break,
        }
    }
    Err(Error::Helper(
        "lost the connection to the helper".to_owned(),
    ))
}

/// Maps the helper's D-Bus errors to [`Error`]s; anything else becomes [`Error::Helper`].
fn from_zbus(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(name, message, _) => {
            let message = message.unwrap_or_default();
            match name.as_str() {
                ERROR_NOT_AUTHORIZED => Error::NotAuthorized,
                ERROR_BUSY => Error::Busy,
                ERROR_CHANGED => Error::ConfigChanged,
                // A config or a refused delete: the message says why.
                ERROR_INVALID_INPUT => Error::InvalidInput(message),
                ERROR_FAILED | ERROR_DEVICE_NOT_FOUND => decode_error(&message),
                _ if message.is_empty() => Error::Helper(name.to_string()),
                _ => Error::Helper(message),
            }
        }
        other => Error::Helper(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::encode_error;

    fn finished(op: &str, ok: bool, message: &str) -> Result<(String, bool, String)> {
        Ok((op.to_owned(), ok, message.to_owned()))
    }

    async fn wait(events: Vec<Result<(String, bool, String)>>, gone: Vec<()>) -> Result<String> {
        // `pending` keeps the gone stream open, as a live owner-changed stream is.
        let gone = stream::iter(gone).chain(stream::pending());
        wait_for_finished(
            OP_CREATE,
            stream::iter(events),
            stream::pending(),
            gone,
            &mut |_| {},
        )
        .await
    }

    #[tokio::test]
    async fn finished_ok_ends_the_wait() {
        let result = wait(vec![finished(OP_CREATE, true, "")], vec![]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn finished_carries_the_message() {
        let result = wait(vec![finished(OP_CREATE, true, "the plan")], vec![]).await;
        assert_eq!(result.unwrap(), "the plan");
    }

    #[tokio::test]
    async fn other_operations_are_skipped() {
        let events = vec![
            finished(OP_DELETE, false, "not ours"),
            finished(OP_CREATE, true, ""),
        ];
        assert!(wait(events, vec![]).await.is_ok());
    }

    #[tokio::test]
    async fn failures_keep_their_kind() {
        let message = encode_error(&Error::DeviceNotFound {
            device: "00000000".to_owned(),
        });
        let result = wait(vec![finished(OP_CREATE, false, &message)], vec![]).await;
        assert!(
            matches!(result, Err(Error::DeviceNotFound { ref device }) if device == "00000000"),
            "{result:?}"
        );
        let result = wait(
            vec![finished(OP_CREATE, false, "rsync exited with code 11")],
            vec![],
        )
        .await;
        assert!(
            matches!(result, Err(Error::Helper(ref m)) if m.ends_with("11")),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn progress_for_the_operation_is_handed_over() {
        let progress =
            |op: &str, percent: f64, eta: i64| (op.to_owned(), percent, eta, "line".to_owned());
        // `Finished` waits until the progress has gone through.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let updates = stream::iter([
            progress(OP_CREATE, -1.0, -1),
            progress(OP_DELETE, 10.0, 5),
            progress(OP_CREATE, 58.0, 180),
        ])
        .chain(stream::once(async move {
            let _ = tx.send(());
            progress(OP_CREATE, 100.0, 0)
        }))
        .chain(stream::pending());
        let finished = stream::once(async move {
            let _ = rx.await;
            finished(OP_CREATE, true, "")
        })
        .chain(stream::pending());
        let mut seen = Vec::new();
        let result = wait_for_finished(OP_CREATE, finished, updates, stream::pending(), &mut |p| {
            seen.push((p.percent, p.eta_seconds))
        })
        .await;
        assert!(result.is_ok());
        assert_eq!(
            seen[..3],
            [
                (None, None),
                (Some(58.0), Some(180)),
                (Some(100.0), Some(0))
            ]
        );
    }

    #[tokio::test]
    async fn helper_leaving_the_bus_ends_the_wait() {
        let result = wait_for_finished(
            OP_CREATE,
            stream::pending(),
            stream::pending(),
            stream::iter([()]),
            &mut |_| {},
        )
        .await;
        assert!(matches!(result, Err(Error::Helper(_))), "{result:?}");
    }

    #[tokio::test]
    async fn closed_signal_stream_ends_the_wait() {
        let result = wait(vec![], vec![]).await;
        assert!(matches!(result, Err(Error::Helper(_))), "{result:?}");
    }
}
