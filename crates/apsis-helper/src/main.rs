// SPDX-License-Identifier: GPL-3.0-only

//! `apsis-helper`: the root side of Apsis.
//!
//! A system D-Bus service, started as root by D-Bus activation (through
//! `apsis-helper.service`) when the applet calls it, and gone again after a minute idle. It
//! offers exactly `List`, `Create(comment)`, `Delete(name)`, `Stop(snapshot)`, `Job`,
//! `ReadConfig` and `WriteConfig`, and announces every job change with `JobChanged`; each
//! method checks its own polkit action for the caller, checks its input again, and runs `rsync`, `lsblk`, `findmnt` and
//! `mount` with a fixed argv, no shell. `WriteConfig` writes `/etc/apsis/config.toml`, nothing
//! else. See `docs/ARCHITECTURE.md`.

mod native;
mod polkit;
mod runner;
mod service;
mod settings;
mod state;
mod usage;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use apsis_core::helper::names::{BUS_NAME, OBJECT_PATH};

use crate::service::{Helper, announce_jobs};
use crate::state::State;

/// Exit after this long with nothing to do. Never while a job runs.
const IDLE: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> ExitCode {
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("apsis-helper: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> zbus::Result<()> {
    let (state, changes) = State::new();
    // Only root may own the name (the bus policy says so), so this fails for anyone else.
    let connection = zbus::connection::Builder::system()?
        .serve_at(OBJECT_PATH, Helper::new(Arc::clone(&state)))?
        .name(BUS_NAME)?
        .build()
        .await?;
    tokio::spawn(announce_jobs(connection.clone(), changes));
    state.idle_for(IDLE).await;
    Ok(())
}

/// The installed files in `resources/helper/` spell the same names as `apsis_core::helper::names`.
#[cfg(test)]
mod resource_tests {
    use apsis_core::helper::names::{
        ACTION_CONFIGURE, ACTION_CREATE, ACTION_DELETE, ACTION_LIST, ACTION_STOP, BUS_NAME,
        INTERFACE, SYSTEMD_UNIT,
    };

    const ACTIVATION: &str =
        include_str!("../../../resources/helper/io.github.atraxsrc.Apsis.Helper.service.in");
    const UNIT: &str = include_str!("../../../resources/helper/apsis-helper.service.in");
    const BUS_POLICY: &str =
        include_str!("../../../resources/helper/io.github.atraxsrc.Apsis.Helper.conf");
    const POLKIT: &str = include_str!("../../../resources/helper/io.github.atraxsrc.Apsis.policy");

    fn has_line(file: &str, line: &str) {
        assert!(file.lines().any(|l| l == line), "missing {line:?}");
    }

    #[test]
    fn dbus_activation_starts_the_unit_as_root() {
        has_line(ACTIVATION, &format!("Name={BUS_NAME}"));
        has_line(ACTIVATION, "User=root");
        has_line(ACTIVATION, "Exec=@libexecdir@/apsis-helper");
        has_line(ACTIVATION, &format!("SystemdService={SYSTEMD_UNIT}"));
    }

    #[test]
    fn systemd_unit_waits_for_the_bus_name() {
        has_line(UNIT, "Type=dbus");
        has_line(UNIT, &format!("BusName={BUS_NAME}"));
        has_line(UNIT, "ExecStart=@libexecdir@/apsis-helper");
        assert!(!UNIT.contains("[Install]"), "only D-Bus starts it");
    }

    #[test]
    fn bus_policy_lets_root_own_and_anyone_call_the_interface() {
        let root = BUS_POLICY
            .split_once("<policy user=\"root\">")
            .and_then(|(_, rest)| rest.split_once("</policy>"))
            .expect("root policy")
            .0;
        assert!(root.contains(&format!("<allow own=\"{BUS_NAME}\"/>")));
        assert_eq!(
            BUS_POLICY.matches("<allow own=").count(),
            1,
            "only root owns"
        );
        assert!(BUS_POLICY.contains(&format!("send_interface=\"{INTERFACE}\"")));
    }

    /// The `<action>` element for `id`.
    fn action(id: &str) -> &'static str {
        POLKIT
            .split_once(&format!("<action id=\"{id}\">"))
            .and_then(|(_, rest)| rest.split_once("</action>"))
            .unwrap_or_else(|| panic!("no action {id}"))
            .0
    }

    #[test]
    fn polkit_actions_match_and_prompt_as_apsis() {
        assert!(action(ACTION_LIST).contains("<allow_active>yes</allow_active>"));
        for id in [ACTION_CREATE, ACTION_DELETE, ACTION_STOP, ACTION_CONFIGURE] {
            assert!(
                action(id).contains("<allow_active>auth_admin_keep</allow_active>"),
                "{id}"
            );
        }
        for id in [
            ACTION_LIST,
            ACTION_CREATE,
            ACTION_DELETE,
            ACTION_STOP,
            ACTION_CONFIGURE,
        ] {
            let action = action(id);
            assert!(action.contains("<message>Apsis "), "{id}");
            assert!(!action.to_lowercase().contains("timeshift"), "{id}");
            assert!(!action.contains("<allow_any>yes"), "{id}");
            assert!(!action.contains("<allow_inactive>yes"), "{id}");
        }
        assert_eq!(POLKIT.matches("<action id=").count(), 5);
    }
}
