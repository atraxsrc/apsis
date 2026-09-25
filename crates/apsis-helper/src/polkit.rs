// SPDX-License-Identifier: GPL-3.0-only

//! The one polkit call the helper makes.

use std::collections::HashMap;

use zbus::zvariant::Value;
use zbus::{Connection, Proxy};

/// `CheckAuthorization` flag: polkit may show the password dialog.
const ALLOW_USER_INTERACTION: u32 = 1;

/// Asks polkit whether the D-Bus caller `sender` (its unique bus name, e.g. `:1.42`) may do
/// `action`. With `interactive`, polkit shows the password dialog if the action needs one;
/// this waits until the user answers.
///
/// The subject is the caller's bus name (`system-bus-name`), not a PID, so a PID reused by
/// another process after the caller exits can't inherit the answer.
pub async fn authorized(
    connection: &Connection,
    sender: &str,
    action: &str,
    interactive: bool,
) -> zbus::Result<bool> {
    let authority = Proxy::new(
        connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await?;
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(sender))]),
    );
    let details: HashMap<&str, &str> = HashMap::new();
    let flags = if interactive {
        ALLOW_USER_INTERACTION
    } else {
        0
    };
    // CheckAuthorization(subject, action_id, details, flags, cancellation_id)
    //   -> (is_authorized, is_challenge, details)
    let (is_authorized, _is_challenge, _details): (bool, bool, HashMap<String, String>) = authority
        .call("CheckAuthorization", &(subject, action, details, flags, ""))
        .await?;
    Ok(is_authorized)
}
