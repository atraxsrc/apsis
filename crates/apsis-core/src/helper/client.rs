// SPDX-License-Identifier: GPL-3.0-only

use futures_util::{Stream, StreamExt, stream};
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::{Connection, Proxy};

use std::future::Future;

use super::names::{
    BUS_NAME, ERROR_BUSY, ERROR_CHANGED, ERROR_DEVICE_NOT_FOUND, ERROR_FAILED, ERROR_INVALID_INPUT,
    ERROR_NOT_AUTHORIZED, ERROR_NOT_INSTALLED, INTERFACE, METHOD_BROWSE, METHOD_CREATE,
    METHOD_DELETE, METHOD_LIST, METHOD_LIST_WITH_USAGE, METHOD_NATIVE_CREATE,
    METHOD_NATIVE_DRY_RUN, METHOD_NATIVE_LIST, METHOD_NATIVE_LIST_WITH_USAGE, METHOD_READ_SETTINGS,
    METHOD_RESTORE, METHOD_WRITE_SETTINGS, OBJECT_PATH, OP_CREATE, OP_DELETE, OP_RESTORE,
    SIGNAL_FINISHED, SIGNAL_PROGRESS,
};
use super::{
    WireList, WireListWithUsage, WireListing, WireSettingsInfo, decode_error, from_wire,
    from_wire_with_usage, info_from_wire, listing_from_wire, settings_to_wire,
};
use crate::error::{Error, Result};
use crate::model::SnapshotList;
use crate::progress::Progress;
use crate::restore::{Listing, Request};
use crate::settings::{Settings, SettingsInfo};

/// The applet's side of `apsis-helper`, on the system bus.
///
/// Needs a tokio runtime (zbus runs on the caller's tokio).
#[derive(Debug, Clone)]
pub struct HelperClient {
    connection: Connection,
}

impl HelperClient {
    /// Connects if the helper is installed (D-Bus can start it) or already running. `None`
    /// when it isn't, or the system bus can't be reached: use the pkexec path instead.
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
        self.list_with_usage(METHOD_LIST_WITH_USAGE, METHOD_LIST)
            .await
    }

    /// Calls `method` (a `...WithUsage` list). A helper from before it existed (one still
    /// running across an upgrade) doesn't know it: then `fallback`, the plain list, without
    /// usage.
    async fn list_with_usage(&self, method: &str, fallback: &str) -> Result<SnapshotList> {
        let proxy = self.proxy().await?;
        match proxy.call::<_, _, WireListWithUsage>(method, &()).await {
            Ok(wire) => from_wire_with_usage(wire),
            Err(error) if is_unknown_method(&error) => {
                let wire: WireList = proxy.call(fallback, &()).await.map_err(from_zbus)?;
                from_wire(wire)
            }
            Err(error) => Err(from_zbus(error)),
        }
    }

    /// Creates a snapshot and waits until it's done (this can take minutes).
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn create(&self, comment: &str) -> Result<()> {
        self.create_with_progress(comment, &mut |_| {}).await
    }

    /// [`HelperClient::create`], handing each `Progress` signal to `on_progress`. An older
    /// helper sends none.
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

    /// Deletes the snapshot `name` and waits until it's done.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn delete(&self, name: &str) -> Result<()> {
        self.operate_on(METHOD_DELETE, OP_DELETE, name, &mut |_| {})
            .await
    }

    /// One folder of snapshot `snapshot`, compared with the running system. Asks for the
    /// password (cached a few minutes).
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]), or a bad reply.
    pub async fn browse(&self, snapshot: &str, path: &str) -> Result<Listing> {
        let wire: WireListing = self
            .proxy()
            .await?
            .call(METHOD_BROWSE, &(snapshot, path))
            .await
            .map_err(from_zbus)?;
        listing_from_wire(wire)
    }

    /// Runs `request` (a dry run, or for real) and waits until it's done. Returns the plan's or
    /// the result's text. Asks for the password: cached for dry runs and folder mode, every
    /// time for original mode.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn restore(&self, request: &Request) -> Result<String> {
        self.restore_with_progress(request, &mut |_| {}).await
    }

    /// [`HelperClient::restore`], handing each `Progress` signal to `on_progress` (a real run
    /// only; a dry run copies nothing).
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn restore_with_progress(
        &self,
        request: &Request,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<String> {
        let args = (
            request.snapshot.clone(),
            request.paths.clone(),
            request.destination.word().to_owned(),
            request.dry_run,
        );
        self.operate(
            OP_RESTORE,
            move |proxy| async move { proxy.call::<_, _, ()>(METHOD_RESTORE, &args).await },
            on_progress,
        )
        .await
    }

    /// Lists snapshots with the native backend (reads the backup device directly). No password
    /// for the active session.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]), or a bad reply.
    pub async fn native_list(&self) -> Result<SnapshotList> {
        self.list_with_usage(METHOD_NATIVE_LIST_WITH_USAGE, METHOD_NATIVE_LIST)
            .await
    }

    /// What a native create with `comment` would do, as text. Nothing is written. No password
    /// for the active session.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn native_dry_run(&self, comment: &str) -> Result<String> {
        self.proxy()
            .await?
            .call(METHOD_NATIVE_DRY_RUN, &(comment,))
            .await
            .map_err(from_zbus)
    }

    /// Creates a native rsync snapshot and waits until it's done (this can take minutes).
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn native_create(&self, comment: &str) -> Result<()> {
        self.native_create_with_progress(comment, &mut |_| {}).await
    }

    /// [`HelperClient::native_create`], handing each `Progress` signal to `on_progress`.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn native_create_with_progress(
        &self,
        comment: &str,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<()> {
        self.operate_on(METHOD_NATIVE_CREATE, OP_CREATE, comment, on_progress)
            .await
    }

    /// Reads Timeshift's settings, the devices and the users. No password for the active
    /// session.
    ///
    /// # Errors
    ///
    /// What the helper reported, or a settings file Apsis can't edit safely.
    pub async fn read_settings(&self) -> Result<SettingsInfo> {
        let wire: WireSettingsInfo = self
            .proxy()
            .await?
            .call(METHOD_READ_SETTINGS, &())
            .await
            .map_err(from_zbus)?;
        info_from_wire(wire)
    }

    /// Writes `settings` if the file still reads `expected` (asks for the password). Returns
    /// once written; the text is a problem Timeshift reported afterwards, or empty.
    ///
    /// # Errors
    ///
    /// What the helper reported: [`Error::SettingsChanged`], [`Error::InvalidSettings`], ...
    pub async fn write_settings(&self, expected: &str, settings: &Settings) -> Result<String> {
        self.proxy()
            .await?
            .call(
                METHOD_WRITE_SETTINGS,
                &(expected, settings_to_wire(settings)),
            )
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
    /// or for the helper to leave the bus without sending one. Its `Progress` signals go to
    /// `on_progress` meanwhile.
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
            .receive_signal(SIGNAL_PROGRESS)
            .await
            .map_err(from_zbus)?
            .filter_map(|message| async move {
                message
                    .body()
                    .deserialize::<(String, f64, i64, String)>()
                    .ok()
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

/// A `Progress` signal's body: `(op, percent, eta_seconds, text)`.
type WireProgress = (String, f64, i64, String);

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

/// The bus or the helper said it has no such method.
fn is_unknown_method(error: &zbus::Error) -> bool {
    matches!(error, zbus::Error::MethodError(name, _, _)
        if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod")
}

/// Maps the helper's D-Bus errors to [`Error`]s; anything else becomes [`Error::Helper`].
fn from_zbus(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(name, message, _) => {
            let message = message.unwrap_or_default();
            match name.as_str() {
                ERROR_NOT_AUTHORIZED => Error::NotAuthorized,
                ERROR_BUSY => Error::Busy,
                ERROR_NOT_INSTALLED => Error::NotInstalled,
                ERROR_CHANGED => Error::SettingsChanged,
                // Settings or a restore request: the message says why.
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
    async fn failures_carry_timeshift_stderr() {
        let message = encode_error(&Error::Failed {
            code: Some(1),
            output: "E: disk full".to_owned(),
        });
        let result = wait(vec![finished(OP_CREATE, false, &message)], vec![]).await;
        assert!(
            matches!(result, Err(Error::Failed { code: Some(1), ref output }) if output == "E: disk full"),
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
