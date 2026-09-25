// SPDX-License-Identifier: GPL-3.0-only

//! Talking to `apsis-helper`, the root D-Bus service that runs Timeshift for the applet.
//!
//! - [`names`]: the shared bus, interface, error and polkit names.
//! - [`WireList`]: what `List` returns, and conversions to and from [`SnapshotList`].
//! - [`WireSettingsInfo`] and [`WireSettings`]: what `ReadSettings` returns and `WriteSettings`
//!   takes.
//! - [`encode_error`] / [`decode_error`]: how a Timeshift failure crosses the bus.
//! - [`HelperClient`]: the applet's side.

mod client;
pub mod names;

pub use client::HelperClient;

use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
use crate::settings::{Config, Settings, SettingsInfo, User, parse_lsblk};

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

/// A user on the bus: `(name, home, encrypted_home)`.
pub type WireUser = (String, String, bool);

/// What `ReadSettings` returns, D-Bus type `(ssa(ssb)b)`: `(settings file, lsblk JSON, users,
/// timeshift-gtk open)`. The applet parses the file and lsblk's output itself.
pub type WireSettingsInfo = (String, String, Vec<WireUser>, bool);

/// [`Settings`] on the bus, D-Bus type `(sbbabauas)`: `(backup device UUID, btrfs mode, include
/// @home, schedule per level, count per level, filters)`, levels in `Level::ALL` order.
pub type WireSettings = (String, bool, bool, Vec<bool>, Vec<u32>, Vec<String>);

/// The [`SettingsInfo`] the helper sent.
///
/// # Errors
///
/// [`Error::InvalidConfig`] for a settings file Apsis can't edit safely, [`Error::Helper`] for
/// output that isn't lsblk's.
pub fn info_from_wire(wire: WireSettingsInfo) -> Result<SettingsInfo> {
    let (text, lsblk, users, timeshift_gui_open) = wire;
    Ok(SettingsInfo {
        config: Config::parse(&text)?,
        text,
        devices: parse_lsblk(&lsblk)?,
        users: users
            .into_iter()
            .map(|(name, home, encrypted_home)| User {
                name,
                home,
                encrypted_home,
            })
            .collect(),
        timeshift_gui_open,
    })
}

#[must_use]
pub fn settings_to_wire(settings: &Settings) -> WireSettings {
    (
        settings.backup_device_uuid.clone(),
        settings.btrfs_mode,
        settings.include_btrfs_home,
        settings.schedule.to_vec(),
        settings.counts.to_vec(),
        settings.exclude.clone(),
    )
}

/// The [`Settings`] a caller sent.
///
/// # Errors
///
/// [`Error::InvalidSettings`] unless there are exactly five schedules and five counts.
pub fn settings_from_wire(wire: WireSettings) -> Result<Settings> {
    let (backup_device_uuid, btrfs_mode, include_btrfs_home, schedule, counts, exclude) = wire;
    let five = || Error::InvalidSettings("expected five schedule levels".to_owned());
    Ok(Settings {
        backup_device_uuid,
        btrfs_mode,
        include_btrfs_home,
        schedule: schedule.try_into().map_err(|_| five())?,
        counts: counts.try_into().map_err(|_| five())?,
        exclude,
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
    fn settings_survive_the_bus() {
        let settings = Settings {
            backup_device_uuid: "uuid".to_owned(),
            btrfs_mode: true,
            include_btrfs_home: false,
            schedule: [true, false, true, false, true],
            counts: [1, 2, 3, 4, 999],
            exclude: vec!["+ /root/**".to_owned(), "*.mp3".to_owned()],
        };
        assert_eq!(
            settings_from_wire(settings_to_wire(&settings)).unwrap(),
            settings
        );
        let mut short = settings_to_wire(&settings);
        short.4.pop();
        assert!(matches!(
            settings_from_wire(short),
            Err(Error::InvalidSettings(_))
        ));
    }

    #[test]
    fn settings_info_is_parsed_on_arrival() {
        let config = include_str!("../../tests/fixtures/config-rsync.json");
        let lsblk = include_str!("../../tests/fixtures/lsblk.json");
        let users = vec![("user1".to_owned(), "/home/user1".to_owned(), false)];
        let info =
            info_from_wire((config.to_owned(), lsblk.to_owned(), users.clone(), true)).unwrap();
        assert_eq!(info.text, config);
        assert_eq!(info.devices.len(), 10);
        assert_eq!(info.users[0].home, "/home/user1");
        assert!(info.timeshift_gui_open);
        let bad = info_from_wire(("[]".to_owned(), lsblk.to_owned(), users, false));
        assert!(matches!(bad, Err(Error::InvalidConfig(_))));
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
