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
pub const INTERFACE: &str = "io.github.atraxsrc.Apsis.Helper2";

/// `List() -> ((sssa(sss)asas)a{st})`: the snapshots on the backup device (mounted read-only
/// for the call), leftovers of interrupted creates, and its disk usage. See
/// [`super::WireListWithUsage`].
pub const METHOD_LIST: &str = "List";
/// `Create(s comment)`: a new snapshot (rsync at idle priority); removes leftovers first.
/// Returns once started; [`SIGNAL_FINISHED`] with [`OP_CREATE`] follows.
pub const METHOD_CREATE: &str = "Create";
/// `Delete(s name)`: deletes one snapshot folder (and its tag links), or one leftover of an
/// interrupted create. Returns once started; [`SIGNAL_FINISHED`] with [`OP_DELETE`] follows.
pub const METHOD_DELETE: &str = "Delete";
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
/// `JobChanged((sssxdx) job)`, to everyone on the bus: a job started, got further (at most
/// about twice a second), is stopping, or ended (sent once with `done`, `failed` or
/// `stopped`; then the helper is idle).
pub const SIGNAL_JOB_CHANGED: &str = "JobChanged";
/// `Finished(s op, b ok, s message)`, sent only to the caller that started the operation.
pub const SIGNAL_FINISHED: &str = "Finished";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_CREATE`].
pub const OP_CREATE: &str = "create";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_DELETE`].
pub const OP_DELETE: &str = "delete";

/// Prefix of the helper's D-Bus error names.
pub const ERROR_PREFIX: &str = "io.github.atraxsrc.Apsis.Helper2.Error";
/// polkit refused, or the password dialog was dismissed.
pub const ERROR_NOT_AUTHORIZED: &str = "io.github.atraxsrc.Apsis.Helper2.Error.NotAuthorized";
/// Another operation is running.
pub const ERROR_BUSY: &str = "io.github.atraxsrc.Apsis.Helper2.Error.Busy";
/// A comment, snapshot name or config the helper refuses.
pub const ERROR_INVALID_INPUT: &str = "io.github.atraxsrc.Apsis.Helper2.Error.InvalidInput";
/// The operation failed; the message is from [`super::encode_error`] or a plain reason.
pub const ERROR_FAILED: &str = "io.github.atraxsrc.Apsis.Helper2.Error.Failed";
/// The backup disk isn't there; the message is from [`super::encode_error`].
pub const ERROR_DEVICE_NOT_FOUND: &str = "io.github.atraxsrc.Apsis.Helper2.Error.DeviceNotFound";
/// `WriteConfig`: the config file changed since the caller read it.
pub const ERROR_CHANGED: &str = "io.github.atraxsrc.Apsis.Helper2.Error.Changed";

/// polkit action for [`METHOD_LIST`], [`METHOD_JOB`] and [`METHOD_READ_CONFIG`]: allowed for
/// the active local session, no password.
pub const ACTION_LIST: &str = "io.github.atraxsrc.Apsis.list";
/// polkit action for [`METHOD_CREATE`]: `auth_admin_keep`.
pub const ACTION_CREATE: &str = "io.github.atraxsrc.Apsis.create";
/// polkit action for [`METHOD_DELETE`]: `auth_admin_keep`.
pub const ACTION_DELETE: &str = "io.github.atraxsrc.Apsis.delete";
/// polkit action for [`METHOD_STOP`] from a uid other than the one that started the create
/// (that one isn't asked): `auth_admin_keep`.
pub const ACTION_STOP: &str = "io.github.atraxsrc.Apsis.stop";
/// polkit action for [`METHOD_WRITE_CONFIG`]: `auth_admin_keep`.
pub const ACTION_CONFIGURE: &str = "io.github.atraxsrc.Apsis.configure";

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
        ] {
            assert!(name.starts_with(app_id), "{name}");
        }
        assert_eq!(OBJECT_PATH, format!("/{}", BUS_NAME.replace('.', "/")));
    }
}
