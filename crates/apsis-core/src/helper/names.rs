// SPDX-License-Identifier: GPL-3.0-only

//! Every name the applet and `apsis-helper` share: bus name, object path, interface, methods,
//! signal, errors and polkit actions. Both sides use these constants, so they can't drift.
//!
//! The files in `resources/helper/` spell the same names; `apsis-helper`'s tests check them.

/// Well-known name the helper owns on the system bus.
pub const BUS_NAME: &str = "io.github.atraxsrc.Apsis.Helper";
/// The helper's single object.
pub const OBJECT_PATH: &str = "/io/github/atraxsrc/Apsis/Helper";
/// The helper's interface. The number is its version; a breaking change gets a new interface.
/// 2 (Apsis 0.4): clean method names, `Stop`, `Job` and `JobChanged`; no file-level restore.
/// 3 (Apsis 0.5): `List` carries each snapshot's format (`(ssss)`), `DeleteMany`, and the
/// full-system restore (`CheckRestore`, `Restore`, `RestartToRestore`, `CancelRestore`,
/// `RestoreResult`). A 0.4.x panel process still running after the upgrade gets
/// `UnknownInterface` until it's re-added (log out and in); `just deb-install` says so.
pub const INTERFACE: &str = "io.github.atraxsrc.Apsis.Helper3";

/// `List() -> ((sssa(ssss)asas)a{st})`: the snapshots on the backup device (mounted read-only
/// for the call), each with its format, leftovers of interrupted creates, and its disk usage.
/// See [`super::WireListWithUsage3`].
pub const METHOD_LIST: &str = "List";
/// `Create(s comment)`: a new snapshot (rsync at idle priority); removes leftovers first.
/// Returns once started; [`SIGNAL_FINISHED`] with [`OP_CREATE`] follows.
pub const METHOD_CREATE: &str = "Create";
/// `Delete(s name)`: deletes one snapshot folder (and its tag links), or one leftover of an
/// interrupted create. Returns once started; [`SIGNAL_FINISHED`] with [`OP_DELETE`] follows.
pub const METHOD_DELETE: &str = "Delete";
/// `DeleteMany(as names)` (0.4.2): deletes several snapshots or leftovers as one job, each in
/// order, stopping at the first failure. At least two names, each a snapshot name, none
/// repeated. Returns once started; [`SIGNAL_FINISHED`] with [`OP_DELETE_MANY`] follows, its
/// message on failure from [`super::encode_error`] (what was deleted, what failed, what's
/// left).
pub const METHOD_DELETE_MANY: &str = "DeleteMany";
/// `Stop(s snapshot)`: stops the running create if it is making `snapshot`. Returns once
/// stopping has begun; the create's [`SIGNAL_FINISHED`] follows.
pub const METHOD_STOP: &str = "Stop";
/// `Job() -> (sssxdx)`: what the helper is doing. See [`crate::job::WireJob`].
pub const METHOD_JOB: &str = "Job";
/// `ReadConfig() -> (s(sbbas)sas)`: see [`super::WireConfigInfo`].
pub const METHOD_READ_CONFIG: &str = "ReadConfig";
/// `WriteConfig(s expected, (sbbas) config) -> s note`: writes `/etc/apsis/config.toml` if it
/// still reads `expected` (empty: there's none yet). See [`super::WireConfig`].
pub const METHOD_WRITE_CONFIG: &str = "WriteConfig";
/// `CheckRestore(s snapshot) -> (b ok, s refusal, b has_home, b has_root, b old_format,
/// s apsis_note)` (0.5.0): the 6b.7 checks for the Restore dialog, on the shared read-only
/// mount. A read, like `List`. `refusal` is a stable word per refusal, decoded by core.
pub const METHOD_CHECK_RESTORE: &str = "CheckRestore";
/// `Restore(s snapshot, b restore_home, b safety_snapshot)` (0.5.0): prepares a full-system
/// restore (the checks, the dry runs, the safety snapshot, the plan on disk) and holds it at
/// the ready prompt. Returns once started; [`SIGNAL_FINISHED`] with [`OP_RESTORE`] follows,
/// `ok` meaning "ready". Stoppable with [`METHOD_STOP`] until ready.
pub const METHOD_RESTORE: &str = "Restore";
/// `RestartToRestore(s snapshot)` (0.5.0): re-checks the ready plan, arms the next boot and
/// restarts. No password for the uid that prepared the plan; [`ACTION_RESTORE`] for another.
pub const METHOD_RESTART_TO_RESTORE: &str = "RestartToRestore";
/// `CancelRestore()` (0.5.0): removes the ready plan (nothing is armed yet). No password for
/// the uid that prepared it; [`ACTION_RESTORE`] for another.
pub const METHOD_CANCEL_RESTORE: &str = "CancelRestore";
/// `RestoreResult() -> (s state, s snapshot, s message, x when, s home, s safety_snapshot)`
/// (0.5.0): how the last restore went, from `result.json`; `state` is `ready`, `done`,
/// `problems`, `boot-kept`, `boot-broken`, `not-started`, `failed` or `""` for none; `home`
/// is `keep` or `restore`, and `""` stands for none.
pub const METHOD_RESTORE_RESULT: &str = "RestoreResult";
/// `JobChanged((sssxdx) job)`, to everyone on the bus: a job started, got further (at most
/// about twice a second), is stopping, or ended (sent once with `done`, `failed` or
/// `stopped`; then the helper is idle). Only writes are jobs (create, delete, delete-many,
/// configure, restore); a `List` or `CheckRestore` is never announced (0.4.2). A ready
/// restore plan is a running `restore` job at 100% until the restart or a cancel.
pub const SIGNAL_JOB_CHANGED: &str = "JobChanged";
/// `Finished(s op, b ok, s message)`, sent only to the caller that started the operation.
pub const SIGNAL_FINISHED: &str = "Finished";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_CREATE`].
pub const OP_CREATE: &str = "create";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_DELETE`].
pub const OP_DELETE: &str = "delete";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_DELETE_MANY`].
pub const OP_DELETE_MANY: &str = "delete-many";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_RESTORE`]; `ok` means the plan is ready.
pub const OP_RESTORE: &str = "restore";

/// Prefix of the helper's D-Bus error names.
pub const ERROR_PREFIX: &str = "io.github.atraxsrc.Apsis.Helper3.Error";
/// polkit refused, or the password dialog was dismissed.
pub const ERROR_NOT_AUTHORIZED: &str = "io.github.atraxsrc.Apsis.Helper3.Error.NotAuthorized";
/// Another operation is running.
pub const ERROR_BUSY: &str = "io.github.atraxsrc.Apsis.Helper3.Error.Busy";
/// A comment, snapshot name or config the helper refuses.
pub const ERROR_INVALID_INPUT: &str = "io.github.atraxsrc.Apsis.Helper3.Error.InvalidInput";
/// The operation failed; the message is from [`super::encode_error`] or a plain reason.
pub const ERROR_FAILED: &str = "io.github.atraxsrc.Apsis.Helper3.Error.Failed";
/// The backup disk isn't there; the message is from [`super::encode_error`].
pub const ERROR_DEVICE_NOT_FOUND: &str = "io.github.atraxsrc.Apsis.Helper3.Error.DeviceNotFound";
/// `WriteConfig`: the config file changed since the caller read it.
pub const ERROR_CHANGED: &str = "io.github.atraxsrc.Apsis.Helper3.Error.Changed";

/// polkit action for [`METHOD_LIST`], [`METHOD_JOB`] and [`METHOD_READ_CONFIG`]: allowed for
/// the active local session, no password.
pub const ACTION_LIST: &str = "io.github.atraxsrc.Apsis.list";
/// polkit action for [`METHOD_CREATE`]: `auth_admin_keep`.
pub const ACTION_CREATE: &str = "io.github.atraxsrc.Apsis.create";
/// polkit action for [`METHOD_DELETE`] and [`METHOD_DELETE_MANY`] (asked once for the whole
/// job): `auth_admin_keep`.
pub const ACTION_DELETE: &str = "io.github.atraxsrc.Apsis.delete";
/// polkit action for [`METHOD_STOP`] from a uid other than the one that started the create
/// (that one isn't asked): `auth_admin_keep`.
pub const ACTION_STOP: &str = "io.github.atraxsrc.Apsis.stop";
/// polkit action for [`METHOD_WRITE_CONFIG`]: `auth_admin_keep`.
pub const ACTION_CONFIGURE: &str = "io.github.atraxsrc.Apsis.configure";
/// polkit action for [`METHOD_RESTORE`], and for [`METHOD_RESTART_TO_RESTORE`] and
/// [`METHOD_CANCEL_RESTORE`] from a uid other than the plan's starter: `auth_admin`, asked
/// every time (no `_keep`).
pub const ACTION_RESTORE: &str = "io.github.atraxsrc.Apsis.restore";

/// The systemd unit D-Bus activation starts.
pub const SYSTEMD_UNIT: &str = "apsis-helper.service";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_names_share_the_prefix() {
        for name in [
            ERROR_NOT_AUTHORIZED,
            ERROR_BUSY,
            ERROR_INVALID_INPUT,
            ERROR_FAILED,
            ERROR_DEVICE_NOT_FOUND,
            ERROR_CHANGED,
        ] {
            let rest = name.strip_prefix(ERROR_PREFIX).expect(name);
            assert!(rest.starts_with('.') && !rest[1..].contains('.'), "{name}");
        }
        assert!(ERROR_PREFIX.starts_with(INTERFACE));
    }

    /// 0.5.0's interface: the list carries each snapshot's format, a breaking wire change.
    #[test]
    fn the_interface_is_helper3_and_names_the_restore() {
        assert_eq!(INTERFACE, "io.github.atraxsrc.Apsis.Helper3");
        assert_eq!(ERROR_PREFIX, "io.github.atraxsrc.Apsis.Helper3.Error");
        assert_eq!(METHOD_CHECK_RESTORE, "CheckRestore");
        assert_eq!(METHOD_RESTORE, "Restore");
        assert_eq!(METHOD_RESTART_TO_RESTORE, "RestartToRestore");
        assert_eq!(METHOD_CANCEL_RESTORE, "CancelRestore");
        assert_eq!(METHOD_RESTORE_RESULT, "RestoreResult");
        assert_eq!(OP_RESTORE, crate::job::JobKind::Restore.word());
        assert_eq!(OP_DELETE_MANY, crate::job::JobKind::DeleteMany.word());
        assert_eq!(ACTION_RESTORE, "io.github.atraxsrc.Apsis.restore");
    }

    #[test]
    fn names_hang_off_the_app_id() {
        let app_id = "io.github.atraxsrc.Apsis";
        for name in [
            BUS_NAME,
            INTERFACE,
            ACTION_LIST,
            ACTION_CREATE,
            ACTION_DELETE,
            ACTION_STOP,
            ACTION_CONFIGURE,
            ACTION_RESTORE,
        ] {
            assert!(name.starts_with(app_id), "{name}");
        }
        assert_eq!(OBJECT_PATH, format!("/{}", BUS_NAME.replace('.', "/")));
    }
}
