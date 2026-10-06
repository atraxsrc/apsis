// SPDX-License-Identifier: GPL-3.0-only

//! Talking to `apsis-helper`, the root D-Bus service that does the snapshot work for the applet.
//!
//! - [`names`]: the shared bus, interface, error and polkit names.
//! - [`WireList`] and [`WireListWithUsage`]: a snapshot list and the backup disk's usage
//!   (`Helper2`); [`WireList3`] and [`WireListWithUsage3`] carry each snapshot's format too
//!   (`Helper3`, 0.5.0).
//! - [`WireConfigInfo`] and [`WireConfig`]: what `ReadConfig` returns and `WriteConfig` takes.
//! - [`crate::job::WireJob`]: what `Job` returns and `JobChanged` carries.
//! - [`encode_error`] / [`decode_error`]: how errors keep their kind across the bus.
//! - [`check_delete_many`]: what `DeleteMany`'s names must be.
//! - [`HelperClient`]: the applet's side.

mod client;
pub mod names;

pub use crate::restore::dialog::WireCheckRestore;
pub use crate::restore::state::WireRestoreResult;
pub use client::{HelperClient, JobEvent};

use std::collections::HashMap;

use crate::config::{Config, ConfigInfo};
use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
use crate::settings::parse_lsblk;
use crate::usage::DiskUsage;

/// One snapshot on the bus: `(name, tags, comment)`. Tags are the letters Timeshift's list
/// showed (`OB`); an
/// empty comment means none.
pub type WireSnapshot = (String, String, String);

/// One snapshot on the bus as `Helper3` (0.5.0) lists it: `(name, tags, comment,
/// rsync_flags)`. The fourth is the raw `apsis-rsync-flags` string from its `info.json`,
/// `""` when the key is missing (Timeshift's, Apsis 0.4.0 and older); the applet asks core
/// ([`crate::native::info::is_old_format`]) what it means. A string, not a bool, so a later
/// flag change (`-H`, 0.5.x) needs no wire change.
pub type WireSnapshot3 = (String, String, String, String);

/// A snapshot list on the bus, D-Bus type `(sssa(sss)asas)`: `(device, uuid, mode, snapshots,
/// warnings, leftovers)`. Empty strings mean "none"; mode is `btrfs`, `rsync` or empty.
pub type WireList = (
    String,
    String,
    String,
    Vec<WireSnapshot>,
    Vec<String>,
    Vec<String>,
);

/// A snapshot list on the bus as `Helper3` sends it, D-Bus type `(sssa(ssss)asas)`: as
/// [`WireList`], with each snapshot's format ([`WireSnapshot3`]).
pub type WireList3 = (
    String,
    String,
    String,
    Vec<WireSnapshot3>,
    Vec<String>,
    Vec<String>,
);

/// What `Helper3`'s `List` returns, D-Bus type `((sssa(ssss)asas)a{st})`: the list and the
/// usage.
pub type WireListWithUsage3 = (WireList3, WireUsage);

/// A [`SnapshotList`] as `Helper3` sends it, with each snapshot's format.
#[must_use]
pub fn to_wire3(list: &SnapshotList) -> WireList3 {
    let (device, uuid, mode, _, warnings, leftovers) = to_wire(list);
    let snapshots = list
        .snapshots
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.tags.iter().map(|t| t.letter()).collect(),
                s.comment.clone().unwrap_or_default(),
                s.rsync_flags.clone().unwrap_or_default(),
            )
        })
        .collect();
    (device, uuid, mode, snapshots, warnings, leftovers)
}

/// The [`SnapshotList`] `Helper3` sent, checked again; each snapshot keeps its format (`""`
/// is none).
///
/// # Errors
///
/// As [`from_wire`].
pub fn from_wire3(wire: WireList3) -> Result<SnapshotList> {
    let (device, uuid, mode, snapshots, warnings, leftovers) = wire;
    let (snapshots, flags): (Vec<WireSnapshot>, Vec<String>) = snapshots
        .into_iter()
        .map(|(name, tags, comment, flags)| ((name, tags, comment), flags))
        .unzip();
    let mut list = from_wire((device, uuid, mode, snapshots, warnings, leftovers))?;
    for (snapshot, flags) in list.snapshots.iter_mut().zip(flags) {
        snapshot.rsync_flags = non_empty(flags);
    }
    Ok(list)
}

/// A [`SnapshotList`] with its disk usage, as `Helper3` sends it.
#[must_use]
pub fn to_wire_with_usage3(list: &SnapshotList) -> WireListWithUsage3 {
    let (_, usage) = to_wire_with_usage(list);
    (to_wire3(list), usage)
}

/// The [`SnapshotList`] and disk usage `Helper3` sent (the usage as [`from_wire_with_usage`]
/// reads it).
///
/// # Errors
///
/// As [`from_wire`].
pub fn from_wire_with_usage3(wire: WireListWithUsage3) -> Result<SnapshotList> {
    let (list, usage) = wire;
    let mut list = from_wire3(list)?;
    list.usage = from_wire_with_usage((to_wire(&SnapshotList::default()), usage))?.usage;
    Ok(list)
}

/// A [`SnapshotList`] as the helper sends it (`Helper2`: without the snapshots' format, which
/// [`from_wire`] reads back as none).
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
        list.leftovers.clone(),
    )
}

/// The [`SnapshotList`] the helper sent, checked again.
///
/// # Errors
///
/// [`Error::Helper`] for an unknown mode or tag, or a snapshot or leftover name that isn't
/// `YYYY-MM-DD_HH-MM-SS`.
pub fn from_wire(wire: WireList) -> Result<SnapshotList> {
    let (device, uuid, mode, snapshots, warnings, leftovers) = wire;
    if let Some(bad) = leftovers.iter().find(|n| parse_snapshot_name(n).is_none()) {
        return Err(Error::Helper(format!("bad leftover name {bad:?}")));
    }
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
                rsync_flags: None,
            })
        })
        .collect::<Result<_>>()?;
    Ok(SnapshotList {
        device: non_empty(device),
        uuid: non_empty(uuid),
        mode,
        snapshots,
        warnings,
        leftovers,
        usage: None,
    })
}

/// Space on the backup device, D-Bus type `a{st}`: bytes by name. `total`, `used` and `free`
/// come together (`statvfs`), or none of them. A dict, so later keys don't change the
/// signature; unknown keys are ignored.
pub type WireUsage = HashMap<String, u64>;

/// What `List` returns, D-Bus type `((sssa(sss)asas)a{st})`: the list and the usage.
pub type WireListWithUsage = (WireList, WireUsage);

const USAGE_TOTAL: &str = "total";
const USAGE_USED: &str = "used";
const USAGE_FREE: &str = "free";

/// A [`SnapshotList`] with its disk usage, as the helper sends it.
#[must_use]
pub fn to_wire_with_usage(list: &SnapshotList) -> WireListWithUsage {
    let mut usage = WireUsage::new();
    if let Some(DiskUsage { total, used, free }) = list.usage {
        usage.insert(USAGE_TOTAL.to_owned(), total);
        usage.insert(USAGE_USED.to_owned(), used);
        usage.insert(USAGE_FREE.to_owned(), free);
    }
    (to_wire(list), usage)
}

/// The [`SnapshotList`] and disk usage the helper sent. A `total`/`used`/`free` set that is
/// incomplete or doesn't add up counts as unknown rather than failing the list.
///
/// # Errors
///
/// As [`from_wire`].
pub fn from_wire_with_usage(wire: WireListWithUsage) -> Result<SnapshotList> {
    let (list, usage) = wire;
    let mut list = from_wire(list)?;
    let get = |key: &str| usage.get(key).copied();
    list.usage = match (get(USAGE_TOTAL), get(USAGE_USED), get(USAGE_FREE)) {
        (Some(total), Some(used), Some(free))
            if total > 0 && used <= total && free <= total - used =>
        {
            Some(DiskUsage { total, used, free })
        }
        _ => None,
    };
    Ok(list)
}

/// A [`Config`] on the bus, D-Bus type `(sbbas)`: `(backup device UUID, include_root,
/// include_home, filters)`.
pub type WireConfig = (String, bool, bool, Vec<String>);

/// What `ReadConfig` returns, D-Bus type `(s(sbbas)sas)`: `(config.toml as read or empty, the
/// config in effect, lsblk JSON, notes)`. The notes are empty unless the config was converted
/// from the old format or imported from Timeshift's settings (not saved yet).
pub type WireConfigInfo = (String, WireConfig, String, Vec<String>);

#[must_use]
pub fn config_to_wire(config: &Config) -> WireConfig {
    (
        config.backup_device_uuid.clone(),
        config.include_root,
        config.include_home,
        config.filters.clone(),
    )
}

#[must_use]
pub fn config_from_wire(wire: WireConfig) -> Config {
    let (backup_device_uuid, include_root, include_home, filters) = wire;
    Config {
        backup_device_uuid,
        include_root,
        include_home,
        filters,
    }
}

/// The [`ConfigInfo`] the helper sent.
///
/// # Errors
///
/// [`Error::InvalidConfig`] for a `config.toml` text that doesn't parse, [`Error::Helper`] for
/// output that isn't lsblk's.
pub fn config_info_from_wire(wire: WireConfigInfo) -> Result<ConfigInfo> {
    let (text, config, lsblk, notes) = wire;
    if !text.is_empty() {
        Config::read(&text)?;
    }
    Ok(ConfigInfo {
        text,
        config: config_from_wire(config),
        devices: parse_lsblk(&lsblk)?,
        notes,
    })
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// An encoded [`Error::DeviceNotFound`]; the device follows.
const DEVICE_NOT_FOUND_HEADER: &str = "backup device not found: ";
/// An encoded [`Error::InvalidInput`]; the reason follows.
const INVALID_INPUT_HEADER: &str = "refused: ";
/// An encoded [`Error::DeviceRemoved`]: `<device>: <reason>` follows.
const DEVICE_REMOVED_HEADER: &str = "backup disk removed: ";
/// An encoded [`Error::Stopped`].
const STOPPED: &str = "stopped";
/// An encoded [`Error::RestoreRefused`]; the refusal's word follows.
const RESTORE_REFUSED_HEADER: &str = "restore refused: ";
/// An encoded [`Error::RestoreArmed`].
const RESTORE_ARMED: &str = "restore armed";
/// An encoded [`Error::DeleteManyStopped`]: `deleted=<a,b> failed=<c> left=<d,e> reason=`
/// and the encoded reason follow. Snapshot names have no spaces or commas, so the fields are
/// unambiguous; the reason comes last because it can hold anything.
const DELETE_MANY_HEADER: &str = "delete stopped: ";

/// An error as one message for the bus (a D-Bus error's text, or `Finished`'s `message`).
/// [`Error::DeviceNotFound`], [`Error::DeviceRemoved`], [`Error::InvalidInput`],
/// [`Error::RestoreRefused`], [`Error::RestoreArmed`], [`Error::Stopped`] and
/// [`Error::DeleteManyStopped`] keep their kind, so [`decode_error`] gives them back;
/// [`Error::Helper`] is its own words, without the `apsis-helper: ` its Display puts in front
/// for the journal; anything else is its text.
#[must_use]
pub fn encode_error(error: &Error) -> String {
    match error {
        Error::DeviceNotFound { device } => format!("{DEVICE_NOT_FOUND_HEADER}{device}"),
        Error::DeviceRemoved { device, reason } => {
            format!("{DEVICE_REMOVED_HEADER}{device}: {reason}")
        }
        Error::InvalidInput(reason) => format!("{INVALID_INPUT_HEADER}{reason}"),
        Error::RestoreRefused(word) => format!("{RESTORE_REFUSED_HEADER}{word}"),
        Error::RestoreArmed => RESTORE_ARMED.to_owned(),
        Error::Stopped => STOPPED.to_owned(),
        Error::DeleteManyStopped {
            deleted,
            failed,
            left,
            reason,
        } => format!(
            "{DELETE_MANY_HEADER}deleted={} failed={failed} left={} reason={}",
            deleted.join(","),
            left.join(","),
            encode_error(reason)
        ),
        Error::Helper(message) => message.clone(),
        other => other.to_string(),
    }
}

/// The error a message from [`encode_error`] stands for. A message it didn't encode becomes
/// [`Error::Helper`] with the text.
#[must_use]
pub fn decode_error(message: &str) -> Error {
    if let Some(rest) = message.strip_prefix(DELETE_MANY_HEADER)
        && let Some(stopped) = decode_delete_many(rest)
    {
        return stopped;
    }
    if let Some(device) = message.strip_prefix(DEVICE_NOT_FOUND_HEADER) {
        return Error::DeviceNotFound {
            device: device.to_owned(),
        };
    }
    if let Some(rest) = message.strip_prefix(DEVICE_REMOVED_HEADER) {
        let (device, reason) = rest.split_once(": ").unwrap_or((rest, ""));
        return Error::DeviceRemoved {
            device: device.to_owned(),
            reason: reason.to_owned(),
        };
    }
    if let Some(reason) = message.strip_prefix(INVALID_INPUT_HEADER) {
        return Error::InvalidInput(reason.to_owned());
    }
    if let Some(word) = message.strip_prefix(RESTORE_REFUSED_HEADER) {
        return Error::RestoreRefused(word.to_owned());
    }
    if message == RESTORE_ARMED {
        return Error::RestoreArmed;
    }
    if message == STOPPED {
        return Error::Stopped;
    }
    Error::Helper(message.to_owned())
}

/// The fields after [`DELETE_MANY_HEADER`]; `None` if they aren't as [`encode_error`] writes
/// them (then the whole message is plain text).
fn decode_delete_many(rest: &str) -> Option<Error> {
    let names = |list: &str| -> Vec<String> {
        list.split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let (deleted, rest) = rest.strip_prefix("deleted=")?.split_once(' ')?;
    let (failed, rest) = rest.strip_prefix("failed=")?.split_once(' ')?;
    let (left, reason) = rest.strip_prefix("left=")?.split_once(" reason=")?;
    if failed.is_empty() {
        return None;
    }
    Some(Error::DeleteManyStopped {
        deleted: names(deleted),
        failed: failed.to_owned(),
        left: names(left),
        reason: Box::new(decode_error(reason)),
    })
}

/// Checks `DeleteMany`'s names before anything runs: at least two, each a snapshot name
/// (`YYYY-MM-DD_HH-MM-SS`), none repeated. The helper runs this too; the window runs it first
/// so a mistake shows before the password dialog.
///
/// # Errors
///
/// [`Error::InvalidSnapshotName`] for a name that isn't one, [`Error::InvalidInput`] for a
/// repeat or fewer than two names.
pub fn check_delete_many(names: &[String]) -> Result<()> {
    if names.len() < 2 {
        return Err(Error::InvalidInput(
            "a delete of several needs at least two names".to_owned(),
        ));
    }
    for (i, name) in names.iter().enumerate() {
        if parse_snapshot_name(name).is_none() {
            return Err(Error::InvalidSnapshotName(name.clone()));
        }
        if names[..i].contains(name) {
            return Err(Error::InvalidInput(format!("{name} is named twice")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three snapshots on a configured device.
    fn sample() -> SnapshotList {
        let snapshot = |name: &str| Snapshot {
            name: name.to_owned(),
            created: parse_snapshot_name(name).unwrap(),
            tags: vec![Tag::OnDemand],
            comment: None,
            rsync_flags: None,
        };
        SnapshotList {
            device: Some("/dev/sdX1".to_owned()),
            uuid: Some("00000000-0000-0000-0000-000000000000".to_owned()),
            mode: Some(Mode::Rsync),
            snapshots: vec![
                snapshot("2026-09-19_09-29-57"),
                snapshot("2026-09-20_10-00-00"),
                snapshot("2026-09-25_11-28-53"),
            ],
            warnings: Vec::new(),
            leftovers: Vec::new(),
            usage: None,
        }
    }

    #[test]
    fn lists_survive_the_bus() {
        let mut list = sample();
        list.snapshots[0].tags = vec![Tag::OnDemand, Tag::Boot];
        list.snapshots[1].comment = Some("with \"quotes\" and ünïcode".to_owned());
        list.warnings = vec!["2026-09-02_09-00-00: incomplete: no info.json".to_owned()];
        assert_eq!(from_wire(to_wire(&list)).unwrap(), list);

        let empty = SnapshotList::default();
        assert_eq!(from_wire(to_wire(&empty)).unwrap(), empty);
    }

    /// `Helper3`'s list carries each snapshot's format as the raw flags string; `Helper2`'s
    /// drops it.
    #[test]
    fn the_format_survives_the_bus_as_the_raw_flags_string() {
        let mut list = sample();
        list.snapshots[0].rsync_flags = Some("-aAX --numeric-ids".to_owned());
        list.snapshots[1].rsync_flags = None;
        list.snapshots[2].rsync_flags = Some("-aHAX --numeric-ids".to_owned());
        let wire = to_wire3(&list);
        assert_eq!(wire.3[0].3, "-aAX --numeric-ids");
        assert_eq!(wire.3[1].3, "", "no key: an empty string on the bus");
        assert_eq!(
            wire.3[2].3, "-aHAX --numeric-ids",
            "a later format travels as is"
        );
        assert_eq!(from_wire3(wire).unwrap(), list);
        // The applet reads the format from the string with core's rule.
        let old = |flags: &str| crate::native::info::is_old_format(flags);
        assert!(!old("-aAX --numeric-ids"));
        assert!(old(""), "no key is the old format");
        assert!(!old("-aHAX --numeric-ids"));
        assert!(old("-a --numeric-ids"));
        // With the usage.
        list.usage = DiskUsage::from_statvfs(1000, 400, 350, 4096);
        assert_eq!(
            from_wire_with_usage3(to_wire_with_usage3(&list)).unwrap(),
            list
        );
        // Helper2's list has no room for it: none on arrival.
        let mut without = list.clone();
        without.usage = None;
        for s in &mut without.snapshots {
            s.rsync_flags = None;
        }
        assert_eq!(from_wire(to_wire(&list)).unwrap(), without);
        // Bad names and tags are refused as before.
        let mut bad = to_wire3(&list);
        bad.3[0].0 = "--help".to_owned();
        assert!(matches!(from_wire3(bad), Err(Error::Helper(_))));
    }

    #[test]
    fn usage_survives_the_bus() {
        let mut list = sample();
        list.usage = DiskUsage::from_statvfs(1000, 400, 350, 4096);
        assert_eq!(
            from_wire_with_usage(to_wire_with_usage(&list)).unwrap(),
            list
        );

        let unknown = SnapshotList::default();
        let wire = to_wire_with_usage(&unknown);
        assert!(wire.1.is_empty());
        assert_eq!(from_wire_with_usage(wire).unwrap(), unknown);
    }

    #[test]
    fn partial_or_odd_usage_is_unknown_and_unknown_keys_are_ignored() {
        let list = |pairs: &[(&str, u64)]| {
            let usage = pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
            from_wire_with_usage((to_wire(&SnapshotList::default()), usage)).unwrap()
        };
        assert_eq!(list(&[("total", 10), ("free", 5)]).usage, None);
        assert_eq!(
            list(&[("total", 10), ("used", 11), ("free", 0)]).usage,
            None
        );
        assert_eq!(list(&[("total", 10), ("used", 6), ("free", 5)]).usage, None);
        assert_eq!(list(&[("total", 0), ("used", 0), ("free", 0)]).usage, None);
        let later = list(&[("total", 10), ("used", 6), ("free", 3), ("inodes", 7)]);
        assert_eq!(
            later.usage,
            Some(DiskUsage {
                total: 10,
                used: 6,
                free: 3
            })
        );
        // An older helper's Timeshift key is just an unknown key now.
        assert_eq!(list(&[("reported-free", 42)]).usage, None);
    }

    #[test]
    fn plain_list_drops_the_usage() {
        let mut list = sample();
        list.usage = DiskUsage::from_statvfs(1000, 400, 350, 4096);
        let back = from_wire(to_wire(&list)).unwrap();
        assert_eq!(back.usage, None);
    }

    #[test]
    fn bad_wire_lists_are_refused() {
        let snapshot = |name: &str, tags: &str| (name.to_owned(), tags.to_owned(), String::new());
        let list = |mode: &str, snapshots| {
            let none = String::new;
            (
                none(),
                none(),
                mode.to_owned(),
                snapshots,
                Vec::new(),
                Vec::new(),
            )
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
    fn leftovers_survive_the_bus_and_must_be_snapshot_names() {
        let mut list = sample();
        list.leftovers = vec!["2026-09-29_14-02-11".to_owned()];
        assert_eq!(from_wire(to_wire(&list)).unwrap().leftovers, list.leftovers);
        list.leftovers = vec!["../x".to_owned()];
        assert!(matches!(from_wire(to_wire(&list)), Err(Error::Helper(_))));
    }

    #[test]
    fn configs_survive_the_bus() {
        let config = Config {
            backup_device_uuid: "uuid".to_owned(),
            include_root: true,
            include_home: false,
            filters: vec!["+ /root/.ssh".to_owned(), "- *.mp3".to_owned()],
        };
        assert_eq!(config_from_wire(config_to_wire(&config)), config);
    }

    #[test]
    fn config_info_is_parsed_on_arrival() {
        let lsblk = include_str!("../../tests/fixtures/lsblk.json");
        let config = Config {
            backup_device_uuid: "uuid".to_owned(),
            ..Config::default()
        };
        let wire = (
            config.to_text(),
            config_to_wire(&config),
            lsblk.to_owned(),
            Vec::new(),
        );
        let info = config_info_from_wire(wire).unwrap();
        assert_eq!(info.saved_device().as_deref(), Some("uuid"));
        assert_eq!(info.devices.len(), 10);
        assert!(!info.unsaved());
        // No file yet (an import): nothing saved.
        let imported = (
            String::new(),
            config_to_wire(&config),
            lsblk.to_owned(),
            vec!["imported".to_owned()],
        );
        let info = config_info_from_wire(imported).unwrap();
        assert_eq!(info.saved_device(), None);
        assert!(info.unsaved());
        assert_eq!(info.config, config);
        let bad = (
            "nope = ".to_owned(),
            config_to_wire(&config),
            lsblk.to_owned(),
            Vec::new(),
        );
        assert!(matches!(
            config_info_from_wire(bad),
            Err(Error::InvalidConfig(_))
        ));
    }

    #[test]
    fn a_missing_disk_survives_the_bus() {
        let error = Error::DeviceNotFound {
            device: "00000000-0000-0000-0000-000000000000".to_owned(),
        };
        assert!(matches!(
            decode_error(&encode_error(&error)),
            Error::DeviceNotFound { device } if device.starts_with("0000")
        ));
    }

    #[test]
    fn a_removed_disk_and_a_stop_survive_the_bus() {
        let removed = Error::DeviceRemoved {
            device: "00000000-0000-0000-0000-000000000000".to_owned(),
            reason: "rsync exited with code 23: Input/output error (os error 5)".to_owned(),
        };
        assert!(matches!(
            decode_error(&encode_error(&removed)),
            Error::DeviceRemoved { device, reason }
                if device.starts_with("0000") && reason.ends_with("(os error 5)")
        ));
        assert!(matches!(
            decode_error(&encode_error(&Error::Stopped)),
            Error::Stopped
        ));
    }

    #[test]
    fn refusals_survive_the_bus() {
        let refused = Error::InvalidInput("not deleting x: no info.json".to_owned());
        assert!(
            matches!(decode_error(&encode_error(&refused)), Error::InvalidInput(m) if m == "not deleting x: no info.json")
        );
    }

    /// A restore the helper refused travels as the refusal's word, so the applet
    /// shows the right dialog.
    #[test]
    fn a_refused_restore_survives_the_bus() {
        let refused = Error::RestoreRefused("boot-files:no-entry".to_owned());
        assert_eq!(
            encode_error(&refused),
            "restore refused: boot-files:no-entry"
        );
        assert!(
            matches!(decode_error(&encode_error(&refused)), Error::RestoreRefused(w) if w == "boot-files:no-entry")
        );
    }

    /// A delete refused while a restore is armed travels as itself, so the window says it
    /// in its own words, not in the helper's.
    #[test]
    fn a_delete_refused_while_a_restore_is_armed_survives_the_bus() {
        assert_eq!(encode_error(&Error::RestoreArmed), "restore armed");
        assert!(matches!(
            decode_error(&encode_error(&Error::RestoreArmed)),
            Error::RestoreArmed
        ));
    }

    /// The helper's own words cross the bus without `Error::Helper`'s `apsis-helper: `
    /// prefix (the journal's form), alone and as the reason of a stopped delete of several
    /// (step 8 saw `Restore failed: apsis-helper: No space left on device` in the window).
    #[test]
    fn a_helper_error_crosses_the_bus_without_the_prefix() {
        let said = "No space left on device (os error 28)";
        let error = Error::Helper(said.to_owned());
        assert_eq!(encode_error(&error), said);
        assert!(matches!(decode_error(&encode_error(&error)), Error::Helper(m) if m == said));
        let stopped = Error::DeleteManyStopped {
            deleted: Vec::new(),
            failed: "2026-09-25_11-28-53".to_owned(),
            left: Vec::new(),
            reason: Box::new(Error::Helper(said.to_owned())),
        };
        let Error::DeleteManyStopped { reason, .. } = decode_error(&encode_error(&stopped)) else {
            panic!("not decoded")
        };
        assert!(
            matches!(*reason, Error::Helper(ref m) if m == said),
            "{reason:?}"
        );
    }

    #[test]
    fn a_stopped_delete_of_several_survives_the_bus() {
        let name = |n: &str| n.to_owned();
        let stopped = Error::DeleteManyStopped {
            deleted: vec![name("2026-09-19_09-29-57"), name("2026-09-20_10-00-00")],
            failed: name("2026-09-25_11-28-53"),
            left: vec![name("2026-09-26_14-02-11")],
            reason: Box::new(Error::DeviceRemoved {
                device: "0000".to_owned(),
                reason: "rsync exited with code 23: Input/output error".to_owned(),
            }),
        };
        let message = encode_error(&stopped);
        assert_eq!(
            message,
            "delete stopped: deleted=2026-09-19_09-29-57,2026-09-20_10-00-00 \
             failed=2026-09-25_11-28-53 left=2026-09-26_14-02-11 reason=backup disk removed: \
             0000: rsync exited with code 23: Input/output error"
        );
        let Error::DeleteManyStopped {
            deleted,
            failed,
            left,
            reason,
        } = decode_error(&message)
        else {
            panic!("not decoded")
        };
        assert_eq!(deleted, ["2026-09-19_09-29-57", "2026-09-20_10-00-00"]);
        assert_eq!(failed, "2026-09-25_11-28-53");
        assert_eq!(left, ["2026-09-26_14-02-11"]);
        assert!(matches!(*reason, Error::DeviceRemoved { ref device, .. } if device == "0000"));

        // Failed on the first: nothing deleted, nothing left, a plain reason.
        let first = Error::DeleteManyStopped {
            deleted: Vec::new(),
            failed: name("2026-09-25_11-28-53"),
            left: Vec::new(),
            reason: Box::new(Error::Native("no space".to_owned())),
        };
        let Error::DeleteManyStopped {
            deleted,
            left,
            reason,
            ..
        } = decode_error(&encode_error(&first))
        else {
            panic!("not decoded")
        };
        assert!(deleted.is_empty() && left.is_empty());
        assert!(matches!(*reason, Error::Helper(ref m) if m == "no space"));

        // Not as the helper writes it: plain text.
        assert!(matches!(
            decode_error("delete stopped: at random"),
            Error::Helper(_)
        ));
        assert!(matches!(
            decode_error("delete stopped: deleted= failed= left= reason=x"),
            Error::Helper(_)
        ));
    }

    #[test]
    fn delete_many_needs_two_distinct_snapshot_names() {
        let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert!(check_delete_many(&names(&["2026-09-19_09-29-57", "2026-09-20_10-00-00"])).is_ok());
        assert!(matches!(
            check_delete_many(&names(&["2026-09-19_09-29-57"])),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            check_delete_many(&[]),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            check_delete_many(&names(&["2026-09-19_09-29-57", "../x"])),
            Err(Error::InvalidSnapshotName(n)) if n == "../x"
        ));
        assert!(matches!(
            check_delete_many(&names(&["2026-09-19_09-29-57", "2026-09-19_09-29-57"])),
            Err(Error::InvalidInput(m)) if m.contains("twice")
        ));
    }

    #[test]
    fn plain_messages_stay_plain() {
        for message in ["rsync exited with code 11: no space", "", "refused"] {
            assert!(matches!(decode_error(message), Error::Helper(m) if m == message));
        }
        let other = Error::Busy;
        assert_eq!(encode_error(&other), other.to_string());
    }
}
