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

/// `List() -> (sssa(sss)as)`: see [`super::WireList`].
pub const METHOD_LIST: &str = "List";
/// `Create(s comment)`: returns once the create has started; [`SIGNAL_FINISHED`] follows.
pub const METHOD_CREATE: &str = "Create";
/// `Delete(s name)`: returns once the delete has started; [`SIGNAL_FINISHED`] follows.
pub const METHOD_DELETE: &str = "Delete";
/// `Finished(s op, b ok, s message)`, sent only to the caller that started the operation.
pub const SIGNAL_FINISHED: &str = "Finished";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_CREATE`].
pub const OP_CREATE: &str = "create";
/// `op` in [`SIGNAL_FINISHED`] after [`METHOD_DELETE`].
pub const OP_DELETE: &str = "delete";

/// Prefix of the helper's D-Bus error names.
pub const ERROR_PREFIX: &str = "io.github.atraxsrc.Apsis.Helper1.Error";
/// polkit refused, or the password dialog was dismissed.
pub const ERROR_NOT_AUTHORIZED: &str = "io.github.atraxsrc.Apsis.Helper1.Error.NotAuthorized";
/// Another list, create or delete is running.
pub const ERROR_BUSY: &str = "io.github.atraxsrc.Apsis.Helper1.Error.Busy";
/// A comment or snapshot name the helper refuses.
pub const ERROR_INVALID_INPUT: &str = "io.github.atraxsrc.Apsis.Helper1.Error.InvalidInput";
/// `timeshift` isn't installed.
pub const ERROR_NOT_INSTALLED: &str = "io.github.atraxsrc.Apsis.Helper1.Error.NotInstalled";
/// Timeshift failed; the message is from [`super::encode_error`] or a plain reason.
pub const ERROR_FAILED: &str = "io.github.atraxsrc.Apsis.Helper1.Error.Failed";
/// The backup disk isn't there; the message is from [`super::encode_error`].
pub const ERROR_DEVICE_NOT_FOUND: &str = "io.github.atraxsrc.Apsis.Helper1.Error.DeviceNotFound";

/// polkit action for [`METHOD_LIST`]: allowed for the active local session, no password.
pub const ACTION_LIST: &str = "io.github.atraxsrc.Apsis.list";
/// polkit action for [`METHOD_CREATE`]: `auth_admin_keep`.
pub const ACTION_CREATE: &str = "io.github.atraxsrc.Apsis.create";
/// polkit action for [`METHOD_DELETE`]: `auth_admin_keep`.
pub const ACTION_DELETE: &str = "io.github.atraxsrc.Apsis.delete";

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
            ERROR_NOT_INSTALLED,
            ERROR_FAILED,
            ERROR_DEVICE_NOT_FOUND,
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
        ] {
            assert!(name.starts_with(app_id), "{name}");
        }
        assert_eq!(OBJECT_PATH, format!("/{}", BUS_NAME.replace('.', "/")));
    }
}
