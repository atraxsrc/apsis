// SPDX-License-Identifier: GPL-3.0-only

//! The D-Bus interface: `List`, `Create`, `Delete` and the `Finished` signal. Nothing else.
//!
//! Every call is logged with its result on stderr, which systemd puts in the journal
//! (`journalctl -u apsis-helper`). Comments are cut to [`LOGGED_COMMENT_CHARS`].

use std::sync::Arc;

use apsis_core::helper::names::{
    ACTION_CREATE, ACTION_DELETE, ACTION_LIST, OBJECT_PATH, OP_CREATE, OP_DELETE,
};
use apsis_core::helper::{WireList, encode_error, to_wire};
use apsis_core::{Error, SnapshotList, parse_snapshot_name, validate_comment};
use zbus::message::Header;
use zbus::names::UniqueName;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, DBusError, interface};

use crate::polkit;
use crate::runner::DirectRunner;
use crate::state::{Running, State};

/// Longest part of a comment that goes into the journal.
const LOGGED_COMMENT_CHARS: usize = 40;

/// The helper object at [`OBJECT_PATH`].
pub struct Helper {
    state: Arc<State<DirectRunner>>,
}

impl Helper {
    pub fn new(state: Arc<State<DirectRunner>>) -> Self {
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
    NotInstalled(String),
    /// The backup disk isn't there; the message is from [`encode_error`].
    DeviceNotFound(String),
    /// The message is from [`encode_error`] (exit code and Timeshift's last lines).
    Failed(String),
}

impl From<Error> for HelperError {
    fn from(error: Error) -> Self {
        match error {
            Error::NotAuthorized => Self::NotAuthorized(error.to_string()),
            Error::Busy => Self::Busy(error.to_string()),
            Error::InvalidComment(_) | Error::InvalidSnapshotName(_) | Error::NoSuchSnapshot(_) => {
                Self::InvalidInput(error.to_string())
            }
            Error::NotInstalled => Self::NotInstalled(error.to_string()),
            Error::DeviceNotFound { .. } => Self::DeviceNotFound(encode_error(&error)),
            _ => Self::Failed(encode_error(&error)),
        }
    }
}

#[interface(name = "io.github.atraxsrc.Apsis.Helper1")]
impl Helper {
    /// Lists snapshots (polkit: `list`, no password for the active session).
    async fn list(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<WireList, HelperError> {
        let _call = self.state.call();
        let caller = caller(&header)?;
        let device = self.state.snapshot_device();
        let label = format!(
            "list for {caller} (device {})",
            device.as_deref().unwrap_or("from Timeshift's config")
        );
        let result = async {
            authorize(connection, &caller, ACTION_LIST, false).await?;
            let running = self.state.begin()?;
            blocking(move || running.list()).await
        }
        .await;
        match &result {
            Ok(list) => log(&format!("{label}: {}", describe_list(list))),
            Err(error) => log(&format!("{label}: {}", describe_error(error))),
        }
        Ok(to_wire(&result?))
    }

    /// Starts an on-demand snapshot (polkit: `create`) and returns; `Finished("create", ..)`
    /// follows.
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
            // The applet checked it too; the helper doesn't rely on that.
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
            move |running| running.create(&comment),
        )
    }

    /// Starts deleting snapshot `name` (polkit: `delete`) and returns; `Finished("delete", ..)`
    /// follows.
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
            move |running| running.delete(&name),
        )
    }

    /// A create or delete ended. Sent only to the caller that started it.
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
    /// run. [`State::begin`] still decides, after polkit, if two calls race.
    fn refuse_if_running(&self) -> apsis_core::Result<()> {
        if self.state.is_running() {
            return Err(Error::Busy);
        }
        Ok(())
    }

    /// Logs whether the operation could start. If it did, runs `work` in the background
    /// holding the lock, then logs how it went and tells `caller` with `Finished`.
    fn start(
        &self,
        connection: &Connection,
        caller: UniqueName<'static>,
        op: &'static str,
        label: String,
        started: apsis_core::Result<Running<DirectRunner>>,
        work: impl FnOnce(&Running<DirectRunner>) -> apsis_core::Result<()> + Send + 'static,
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
        tokio::spawn(async move {
            let _call = call;
            // `running` drops when `work` returns, so the lock is free before `Finished`
            // arrives and the caller's refresh isn't refused as busy.
            let result = blocking(move || work(&running)).await;
            let (ok, message) = match &result {
                Ok(()) => {
                    log(&format!("{label}: done"));
                    (true, String::new())
                }
                Err(error) => {
                    log(&format!("{label}: {}", describe_error(error)));
                    (false, encode_error(error))
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

fn log(line: &str) {
    eprintln!("apsis-helper: {line}");
}

/// `ok, 5 snapshots` (plus the warnings, if Timeshift had any).
fn describe_list(list: &SnapshotList) -> String {
    let mut text = format!("ok, {} snapshots", list.snapshots.len());
    if !list.warnings.is_empty() {
        text.push_str(&format!("; warnings: {}", list.warnings.join(" | ")));
    }
    text
}

/// One journal line for an error: `refused: ...` when nothing ran, else `failed: ...` with the
/// exit code and Timeshift's last lines.
fn describe_error(error: &Error) -> String {
    match error {
        Error::NotAuthorized
        | Error::Busy
        | Error::InvalidComment(_)
        | Error::InvalidSnapshotName(_) => format!("refused: {error}"),
        Error::Failed { code, output } => {
            let code = code.map_or_else(|| "none (signal)".to_owned(), |c| c.to_string());
            let output: Vec<&str> = output.lines().collect();
            format!("failed, exit code {code}: {}", output.join(" | "))
        }
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

/// Runs blocking Timeshift work off the async runtime.
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
        ERROR_BUSY, ERROR_DEVICE_NOT_FOUND, ERROR_FAILED, ERROR_INVALID_INPUT,
        ERROR_NOT_AUTHORIZED, ERROR_NOT_INSTALLED, INTERFACE, METHOD_CREATE, METHOD_DELETE,
        METHOD_LIST, SIGNAL_FINISHED,
    };
    use zbus::object_server::Interface;

    use super::*;

    #[test]
    fn interface_matches_the_shared_names() {
        assert_eq!(Helper::name().as_str(), INTERFACE);
        let mut xml = String::new();
        Helper::new(State::new(DirectRunner)).introspect_to_writer(&mut xml, 0);
        for method in [METHOD_LIST, METHOD_CREATE, METHOD_DELETE] {
            assert!(
                xml.contains(&format!("<method name=\"{method}\">")),
                "{method}\n{xml}"
            );
        }
        assert!(
            xml.contains(&format!("<signal name=\"{SIGNAL_FINISHED}\">")),
            "{xml}"
        );
        // Nothing else: exactly three methods and one signal.
        assert_eq!(xml.matches("<method ").count(), 3, "{xml}");
        assert_eq!(xml.matches("<signal ").count(), 1, "{xml}");
        // List returns the list with its warnings.
        assert!(xml.contains("type=\"(sssa(sss)as)\""), "{xml}");
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
                HelperError::NotInstalled(String::new()),
                ERROR_NOT_INSTALLED,
            ),
            (
                HelperError::DeviceNotFound(String::new()),
                ERROR_DEVICE_NOT_FOUND,
            ),
            (HelperError::Failed(String::new()), ERROR_FAILED),
        ];
        for (error, name) in cases {
            assert_eq!(zbus::DBusError::name(&error).as_str(), name);
        }
    }

    #[test]
    fn failures_reach_the_client_with_timeshift_output() {
        let HelperError::Failed(message) = HelperError::from(Error::Failed {
            code: Some(1),
            output: "E: first\nE: second".to_owned(),
        }) else {
            panic!("not Failed")
        };
        assert!(matches!(
            decode_error(&message),
            Error::Failed { code: Some(1), output } if output == "E: first\nE: second"
        ));

        let HelperError::DeviceNotFound(message) = HelperError::from(Error::DeviceNotFound {
            device: "/dev/sdX1".to_owned(),
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

        let failed = Error::Failed {
            code: Some(1),
            output: "E: Device busy\nE: Failed to remove directory".to_owned(),
        };
        assert_eq!(
            describe_error(&failed),
            "failed, exit code 1: E: Device busy | E: Failed to remove directory"
        );
        assert_eq!(
            describe_error(&Error::Busy),
            format!("refused: {}", Error::Busy)
        );

        let mut list = SnapshotList::default();
        assert_eq!(describe_list(&list), "ok, 0 snapshots");
        list.warnings = vec!["E: Failed to remove directory".to_owned()];
        assert_eq!(
            describe_list(&list),
            "ok, 0 snapshots; warnings: E: Failed to remove directory"
        );
    }
}
