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

mod apply;
mod arm;
mod check;
#[cfg(test)]
mod maintainer_scripts;
mod native;
mod polkit;
mod prepare;
mod runner;
mod service;
mod settings;
mod state;
mod usage;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use apsis_core::helper::names::{BUS_NAME, OBJECT_PATH};

use crate::service::{Helper, announce_jobs, watch_starters};
use crate::state::State;

/// Exit after this long with nothing to do. Never while a job runs.
const IDLE: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--apply-restore") {
        // The offline boot's entry point (PLAN 6b.6): no D-Bus.
        return apply::apply_restore();
    }
    if args.iter().any(|a| a == "--disarm") {
        // The disarm timer's service (PLAN 6b.5) and the package's prerm (6b.9): no D-Bus.
        return match arm::disarm_system() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("apsis-helper: disarm: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("apsis-helper: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> zbus::Result<()> {
    // Leftovers of an arm or a plan with no link arm nothing and go (PLAN 6b.5).
    match arm::clean_at_start(&arm::Paths::system()) {
        Ok(removed) if !removed.is_empty() => {
            eprintln!(
                "apsis-helper: leftovers removed at start: {}",
                removed.join(", ")
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("apsis-helper: couldn't remove leftovers at start: {error}"),
    }
    let (state, changes) = State::new(state::JOB_LOCK, 0);
    // Only root may own the name (the bus policy says so), so this fails for anyone else.
    let connection = zbus::connection::Builder::system()?
        .serve_at(OBJECT_PATH, Helper::new(Arc::clone(&state)))?
        .name(BUS_NAME)?
        .build()
        .await?;
    tokio::spawn(announce_jobs(connection.clone(), changes));
    tokio::spawn(watch_starters(connection.clone(), Arc::clone(&state)));
    state.idle_for(IDLE).await;
    Ok(())
}

/// The installed files in `resources/helper/` spell the same names as `apsis_core::helper::names`.
#[cfg(test)]
mod resource_tests {
    use apsis_core::helper::names::{
        ACTION_CONFIGURE, ACTION_CREATE, ACTION_DELETE, ACTION_LIST, ACTION_RESTORE, ACTION_STOP,
        BUS_NAME, INTERFACE, SYSTEMD_UNIT,
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
        // The restore asks every time, from any session (PLAN 6b.9): no `_keep`.
        let restore = action(ACTION_RESTORE);
        for setting in ["allow_any", "allow_inactive", "allow_active"] {
            assert!(
                restore.contains(&format!("<{setting}>auth_admin</{setting}>")),
                "{setting}"
            );
        }
        for id in [
            ACTION_LIST,
            ACTION_CREATE,
            ACTION_DELETE,
            ACTION_STOP,
            ACTION_CONFIGURE,
            ACTION_RESTORE,
        ] {
            let action = action(id);
            assert!(action.contains("<message>Apsis "), "{id}");
            assert!(!action.to_lowercase().contains("timeshift"), "{id}");
            assert!(!action.contains("<allow_any>yes"), "{id}");
            assert!(!action.contains("<allow_inactive>yes"), "{id}");
        }
        assert_eq!(POLKIT.matches("<action id=").count(), 6);
    }
}
