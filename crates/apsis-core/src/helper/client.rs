// SPDX-License-Identifier: GPL-3.0-only

use futures_util::{Stream, StreamExt, stream};
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::{Connection, Proxy};

use super::names::{
    BUS_NAME, ERROR_BUSY, ERROR_CHANGED, ERROR_DEVICE_NOT_FOUND, ERROR_FAILED, ERROR_INVALID_INPUT,
    ERROR_NOT_AUTHORIZED, ERROR_NOT_INSTALLED, INTERFACE, METHOD_CREATE, METHOD_DELETE,
    METHOD_LIST, METHOD_NATIVE_CREATE, METHOD_NATIVE_DRY_RUN, METHOD_NATIVE_LIST,
    METHOD_READ_SETTINGS, METHOD_WRITE_SETTINGS, OBJECT_PATH, OP_CREATE, OP_DELETE,
    SIGNAL_FINISHED,
};
use super::{
    WireList, WireSettingsInfo, decode_error, from_wire, info_from_wire, settings_to_wire,
};
use crate::error::{Error, Result};
use crate::model::SnapshotList;
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

    /// Lists snapshots. No password for the active session.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]), or a bad reply.
    pub async fn list(&self) -> Result<SnapshotList> {
        let wire: WireList = self
            .proxy()
            .await?
            .call(METHOD_LIST, &())
            .await
            .map_err(from_zbus)?;
        from_wire(wire)
    }

    /// Creates a snapshot and waits until it's done (this can take minutes).
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn create(&self, comment: &str) -> Result<()> {
        self.operate(METHOD_CREATE, OP_CREATE, comment).await
    }

    /// Deletes the snapshot `name` and waits until it's done.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]).
    pub async fn delete(&self, name: &str) -> Result<()> {
        self.operate(METHOD_DELETE, OP_DELETE, name).await
    }

    /// Lists snapshots with the native backend (reads the backup device directly). No password
    /// for the active session.
    ///
    /// # Errors
    ///
    /// What the helper reported (see [`Error`]), or a bad reply.
    pub async fn native_list(&self) -> Result<SnapshotList> {
        let wire: WireList = self
            .proxy()
            .await?
            .call(METHOD_NATIVE_LIST, &())
            .await
            .map_err(from_zbus)?;
        from_wire(wire)
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
        self.operate(METHOD_NATIVE_CREATE, OP_CREATE, comment).await
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

    /// Starts `method(argument)`, then waits for its `Finished`, or for the helper to leave the
    /// bus without sending one.
    async fn operate(&self, method: &str, op: &str, argument: &str) -> Result<()> {
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
        let gone = proxy
            .receive_owner_changed()
            .await
            .map_err(from_zbus)?
            .filter_map(|owner| async move { owner.is_none().then_some(()) });
        // The call returns once the helper has checked the input and polkit and started.
        proxy
            .call::<_, _, ()>(method, &(argument,))
            .await
            .map_err(from_zbus)?;
        wait_for_finished(op, finished, gone).await
    }
}

/// What the helper told us while an operation ran.
enum Event {
    Finished(Result<(String, bool, String)>),
    /// The signal stream ended (the connection closed).
    Closed,
    Gone,
}

/// Waits for `Finished(op, ok, message)` for `op`. Fails if the helper leaves the bus (`gone`)
/// or the signal stream ends first.
async fn wait_for_finished(
    op: &str,
    finished: impl Stream<Item = Result<(String, bool, String)>>,
    gone: impl Stream<Item = ()>,
) -> Result<()> {
    // `select` only ends when both streams do, and `gone` never does: mark the end of
    // `finished` with `Closed` instead.
    let finished = finished
        .map(Event::Finished)
        .chain(stream::once(async { Event::Closed }));
    let events = stream::select(finished, gone.map(|()| Event::Gone));
    let mut events = std::pin::pin!(events);
    while let Some(event) = events.next().await {
        match event {
            Event::Finished(Ok((finished_op, ok, message))) if finished_op == op => {
                return if ok {
                    Ok(())
                } else {
                    Err(decode_error(&message))
                };
            }
            // Another operation's signal (e.g. a delete's, arriving late): not ours.
            Event::Finished(Ok(_)) => {}
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
                ERROR_NOT_INSTALLED => Error::NotInstalled,
                ERROR_CHANGED => Error::SettingsChanged,
                // Comments are checked before they're sent, so this is about settings.
                ERROR_INVALID_INPUT => Error::InvalidSettings(message),
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

    async fn wait(events: Vec<Result<(String, bool, String)>>, gone: Vec<()>) -> Result<()> {
        // `pending` keeps the gone stream open, as a live owner-changed stream is.
        let gone = stream::iter(gone).chain(stream::pending());
        wait_for_finished(OP_CREATE, stream::iter(events), gone).await
    }

    #[tokio::test]
    async fn finished_ok_ends_the_wait() {
        let result = wait(vec![finished(OP_CREATE, true, "")], vec![]).await;
        assert!(result.is_ok());
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
    async fn helper_leaving_the_bus_ends_the_wait() {
        let result = wait_for_finished(OP_CREATE, stream::pending(), stream::iter([()])).await;
        assert!(matches!(result, Err(Error::Helper(_))), "{result:?}");
    }

    #[tokio::test]
    async fn closed_signal_stream_ends_the_wait() {
        let result = wait(vec![], vec![]).await;
        assert!(matches!(result, Err(Error::Helper(_))), "{result:?}");
    }
}
