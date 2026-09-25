// SPDX-License-Identifier: GPL-3.0-only

//! Talking to `apsis-helper`, the root D-Bus service that runs Timeshift for the applet.
//!
//! - [`names`]: the shared bus, interface, error and polkit names.
//! - [`WireList`]: what `List` returns, and conversions to and from [`SnapshotList`].
//! - [`encode_error`] / [`decode_error`]: how a Timeshift failure crosses the bus.
//! - [`HelperClient`]: the applet's side.

mod client;
pub mod names;

pub use client::HelperClient;

use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};

/// One snapshot on the bus: `(name, tags, comment)`. Tags are Timeshift's letters (`OB`); an
/// empty comment means none.
pub type WireSnapshot = (String, String, String);

/// A snapshot list on the bus, D-Bus type `(sssa(sss)as)`: `(device, uuid, mode, snapshots,
/// warnings)`. Empty strings mean "none"; mode is `btrfs`, `rsync` or empty.
pub type WireList = (String, String, String, Vec<WireSnapshot>, Vec<String>);

/// A [`SnapshotList`] as the helper sends it.
#[must_use]
pub fn to_wire(list: &SnapshotList) -> WireList {
    let mode = match list.mode {
        Some(Mode::Btrfs) => "btrfs",
        Some(Mode::Rsync) => "rsync",
        None => "",
    };
    let snapshots = list
        .snapshots
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.tags.iter().map(|t| t.letter()).collect(),
                s.comment.clone().unwrap_or_default(),
            )
        })
        .collect();
    (
        list.device.clone().unwrap_or_default(),
        list.uuid.clone().unwrap_or_default(),
        mode.to_owned(),
        snapshots,
        list.warnings.clone(),
    )
}

/// The [`SnapshotList`] the helper sent, checked like Timeshift's own output would be.
///
/// # Errors
///
/// [`Error::Helper`] for an unknown mode or tag, or a snapshot name that isn't
/// `YYYY-MM-DD_HH-MM-SS`.
pub fn from_wire(wire: WireList) -> Result<SnapshotList> {
    let (device, uuid, mode, snapshots, warnings) = wire;
    let mode = match mode.as_str() {
        "btrfs" => Some(Mode::Btrfs),
        "rsync" => Some(Mode::Rsync),
        "" => None,
        other => return Err(Error::Helper(format!("unknown mode {other:?}"))),
    };
    let snapshots = snapshots
        .into_iter()
        .map(|(name, tags, comment)| {
            let created = parse_snapshot_name(&name)
                .ok_or_else(|| Error::Helper(format!("bad snapshot name {name:?}")))?;
            let tags = tags
                .chars()
                .map(|c| Tag::from_char(c).ok_or_else(|| Error::Helper(format!("bad tag {c:?}"))))
                .collect::<Result<_>>()?;
            Ok(Snapshot {
                name,
                created,
                tags,
                comment: non_empty(comment),
            })
        })
        .collect::<Result<_>>()?;
    Ok(SnapshotList {
        device: non_empty(device),
        uuid: non_empty(uuid),
        mode,
        snapshots,
        warnings,
    })
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// First line of an encoded [`Error::Failed`]; the exit code or `signal` follows, then the output.
const FAILED_HEADER: &str = "timeshift exit status: ";
/// An encoded [`Error::DeviceNotFound`]; the device follows.
const DEVICE_NOT_FOUND_HEADER: &str = "timeshift device not found: ";

/// An error as one message for the bus (a D-Bus error's text, or `Finished`'s `message`).
/// [`Error::Failed`] and [`Error::DeviceNotFound`] keep their details, so [`decode_error`] gives
/// them back; anything else is its text.
#[must_use]
pub fn encode_error(error: &Error) -> String {
    match error {
        Error::Failed { code, output } => {
            let status = code.map_or_else(|| "signal".to_owned(), |code| code.to_string());
            format!("{FAILED_HEADER}{status}\n{output}")
        }
        Error::DeviceNotFound { device } => format!("{DEVICE_NOT_FOUND_HEADER}{device}"),
        other => other.to_string(),
    }
}

/// The error a message from [`encode_error`] stands for. A message it didn't encode becomes
/// [`Error::Helper`] with the text.
#[must_use]
pub fn decode_error(message: &str) -> Error {
    if let Some(device) = message.strip_prefix(DEVICE_NOT_FOUND_HEADER) {
        return Error::DeviceNotFound {
            device: device.to_owned(),
        };
    }
    let failed = message.strip_prefix(FAILED_HEADER).and_then(|rest| {
        let (status, output) = rest.split_once('\n').unwrap_or((rest, ""));
        let code = match status {
            "signal" => None,
            code => Some(code.parse().ok()?),
        };
        Some(Error::Failed {
            code,
            output: output.to_owned(),
        })
    });
    failed.unwrap_or_else(|| Error::Helper(message.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_list;

    const DEVICE_LIST: &str = include_str!("../../tests/fixtures/list-rsync-device.txt");

    #[test]
    fn lists_survive_the_bus() {
        let mut list = parse_list(DEVICE_LIST).unwrap();
        list.snapshots[0].tags = vec![Tag::OnDemand, Tag::Boot];
        list.snapshots[1].comment = Some("with \"quotes\" and ünïcode".to_owned());
        list.warnings = vec!["E: Failed to remove directory".to_owned()];
        assert_eq!(from_wire(to_wire(&list)).unwrap(), list);

        let empty = SnapshotList::default();
        assert_eq!(from_wire(to_wire(&empty)).unwrap(), empty);
    }

    #[test]
    fn bad_wire_lists_are_refused() {
        let snapshot = |name: &str, tags: &str| (name.to_owned(), tags.to_owned(), String::new());
        let list = |mode: &str, snapshots| {
            let none = String::new;
            (none(), none(), mode.to_owned(), snapshots, Vec::new())
        };
        for bad in [
            list("zfs", vec![]),
            list("rsync", vec![snapshot("--help", "O")]),
            list("rsync", vec![snapshot("2026-09-25_03-00-01", "X")]),
        ] {
            assert!(matches!(from_wire(bad), Err(Error::Helper(_))));
        }
    }

    #[test]
    fn failures_keep_their_exit_code_and_output() {
        let cases = [
            (Some(1), "E: first\nE: last"),
            (Some(-3), ""),
            (None, "killed"),
        ];
        for (code, output) in cases {
            let error = Error::Failed {
                code,
                output: output.to_owned(),
            };
            match decode_error(&encode_error(&error)) {
                Error::Failed { code: c, output: o } => assert_eq!((c, o.as_str()), (code, output)),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_missing_disk_survives_the_bus() {
        let error = Error::DeviceNotFound {
            device: "/dev/sdX1".to_owned(),
        };
        assert!(matches!(
            decode_error(&encode_error(&error)),
            Error::DeviceNotFound { device } if device == "/dev/sdX1"
        ));
    }

    #[test]
    fn plain_messages_stay_plain() {
        for message in [
            "timeshift is not installed",
            "timeshift exit status: nope\nx",
            "",
        ] {
            assert!(matches!(decode_error(message), Error::Helper(m) if m == message));
        }
        let other = Error::NotInstalled;
        assert_eq!(encode_error(&other), other.to_string());
    }
}
