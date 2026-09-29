// SPDX-License-Identifier: GPL-3.0-only

//! Every name the applet and `apsis-helper` share: bus name, object path, interface, methods,
//! signal, errors and polkit actions. Both sides use these constants, so they can't drift.
//!
//! The files in `resources/helper/` spell the same names; `apsis-helper`'s tests check them.

/// Well-known name the helper owns on the system bus.
pub const BUS_NAME: &str = "io.github.atraxsrc.Apsis.Helper";
/// The helper's single object.
pub const OBJECT_PATH: &str = "/io/github/atraxsrc/Apsis/Helper";
/// The helper's interface. The `1` is its version; a breaking change gets a new interface.
pub const INTERFACE: &str = "io.github.atraxsrc.Apsis.Helper1";

/// `NativeListWithUsage() -> ((sssa(sss)as)a{st})`: the snapshots on the backup device
/// (mounted read-only for the call) and its disk usage. See [`super::WireListWithUsage`].
pub const METHOD_NATIVE_LIST_WITH_USAGE: &str = "NativeListWithUsage";
/// `NativeCreate(s comment)`: a new snapshot (rsync at idle priority). Returns once started;
/// [`SIGNAL_FINISHED`] with [`OP_CREATE`] follows.
pub const METHOD_NATIVE_CREATE: &str = "NativeCreate";
/// `Delete(s name)`: deletes one snapshot folder (and its tag links). Returns once started;
/// [`SIGNAL_FINISHED`] with [`OP_DELETE`] follows.
pub const METHOD_DELETE: &str = "Delete";
/// `ReadConfig() -> (s(sas)sa(ssb)as)`: see [`super::WireConfigInfo`].
pub const METHOD_READ_CONFIG: &str = "ReadConfig";
/// `WriteConfig(s expected, (sas) config) -> s note`: writes `/etc/apsis/config.toml` if it
/// still reads `expected` (empty: there's none yet). See [`super::WireConfig`].
pub const METHOD_WRITE_CONFIG: &str = "WriteConfig";
/// `Browse(s snapshot, s path) -> (a(sstxuuussstx)b)`: one folder of a snapshot, each entry
/// compared with the running system. See [`super::WireListing`].
pub const METHOD_BROWSE: &str = "Browse";
/// `Restore(s snapshot, as paths, s destination, b dry_run)`: returns once started;
/// [`SIGNAL_FINISHED`] with [`OP_RESTORE`] follows, its message the plan or the result.
pub const METHOD_RESTORE: &str = "Restore";
/// `Progress(s op, d percent, x eta_seconds, s text)` while a create or restore runs, at most
/// about twice a second, sent only to its caller and never after its [`SIGNAL_FINISHED`].
/// `percent` (0 to 100) and `eta_seconds` are `-1` while unknown; `text` is the line they were
/// read from. An older helper never sends it; the applet then shows a spinner.
pub const SIGNAL_PROGRESS: &str = "Progress";
/// `Finished(s op, b ok, s message)`, sent only to the caller that started the operation.
pub const SIGNAL_FINISHED: &str = "Finished";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_NATIVE_CREATE`].
pub const OP_CREATE: &str = "create";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_DELETE`].
pub const OP_DELETE: &str = "delete";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_RESTORE`].
pub const OP_RESTORE: &str = "restore";

/// Prefix of the helper's D-Bus error names.
pub const ERROR_PREFIX: &str = "io.github.atraxsrc.Apsis.Helper1.Error";
/// polkit refused, or the password dialog was dismissed.
pub const ERROR_NOT_AUTHORIZED: &str = "io.github.atraxsrc.Apsis.Helper1.Error.NotAuthorized";
/// Another operation is running.
pub const ERROR_BUSY: &str = "io.github.atraxsrc.Apsis.Helper1.Error.Busy";
/// A comment, snapshot name, config or restore request the helper refuses.
pub const ERROR_INVALID_INPUT: &str = "io.github.atraxsrc.Apsis.Helper1.Error.InvalidInput";
/// The operation failed; the message is from [`super::encode_error`] or a plain reason.
pub const ERROR_FAILED: &str = "io.github.atraxsrc.Apsis.Helper1.Error.Failed";
/// The backup disk isn't there; the message is from [`super::encode_error`].
pub const ERROR_DEVICE_NOT_FOUND: &str = "io.github.atraxsrc.Apsis.Helper1.Error.DeviceNotFound";
/// `WriteConfig`: the config file changed since the caller read it.
pub const ERROR_CHANGED: &str = "io.github.atraxsrc.Apsis.Helper1.Error.Changed";

/// polkit action for [`METHOD_NATIVE_LIST_WITH_USAGE`] and [`METHOD_READ_CONFIG`]: allowed
/// for the active local session, no password.
pub const ACTION_LIST: &str = "io.github.atraxsrc.Apsis.list";
/// polkit action for [`METHOD_NATIVE_CREATE`]: `auth_admin_keep`.
pub const ACTION_CREATE: &str = "io.github.atraxsrc.Apsis.create";
/// polkit action for [`METHOD_DELETE`]: `auth_admin_keep`.
pub const ACTION_DELETE: &str = "io.github.atraxsrc.Apsis.delete";
/// polkit action for [`METHOD_WRITE_CONFIG`]: `auth_admin_keep`.
pub const ACTION_CONFIGURE: &str = "io.github.atraxsrc.Apsis.configure";

/// polkit action for [`METHOD_BROWSE`] and restore dry runs: `auth_admin_keep` (snapshots
/// hold root-only files).
pub const ACTION_BROWSE: &str = "io.github.atraxsrc.Apsis.browse";
/// polkit action for a real [`METHOD_RESTORE`] in folder mode: `auth_admin_keep`.
pub const ACTION_RESTORE: &str = "io.github.atraxsrc.Apsis.restore";
/// polkit action for a real [`METHOD_RESTORE`] in original mode: `auth_admin`, asked every time.
pub const ACTION_RESTORE_ORIGINAL: &str = "io.github.atraxsrc.Apsis.restore-original";

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
            ACTION_CONFIGURE,
            ACTION_BROWSE,
            ACTION_RESTORE,
            ACTION_RESTORE_ORIGINAL,
        ] {
            assert!(name.starts_with(app_id), "{name}");
        }
        assert_eq!(OBJECT_PATH, format!("/{}", BUS_NAME.replace('.', "/")));
    }
}
