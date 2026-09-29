// SPDX-License-Identifier: GPL-3.0-only

//! Talking to `apsis-helper`, the root D-Bus service that does the snapshot work for the applet.
//!
//! - [`names`]: the shared bus, interface, error and polkit names.
//! - [`WireList`] and [`WireListWithUsage`]: a snapshot list and the backup disk's usage.
//! - [`WireConfigInfo`] and [`WireConfig`]: what `ReadConfig` returns and `WriteConfig` takes.
//! - [`WireListing`]: what `Browse` returns.
//! - [`encode_error`] / [`decode_error`]: how errors keep their kind across the bus.
//! - [`HelperClient`]: the applet's side.

mod client;
pub mod names;

pub use client::HelperClient;

use std::collections::HashMap;

use crate::config::{Config, ConfigInfo};
use crate::error::{Error, Result};
use crate::model::{Mode, Snapshot, SnapshotList, Tag, parse_snapshot_name};
use crate::restore::{Entry, Kind, Listing, Live};
use crate::settings::{User, parse_lsblk};
use crate::usage::DiskUsage;

/// One snapshot on the bus: `(name, tags, comment)`. Tags are the letters Timeshift's list
/// showed (`OB`); an
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

/// The [`SnapshotList`] the helper sent, checked again.
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
        usage: None,
    })
}

/// Space on the backup device, D-Bus type `a{st}`: bytes by name. `total`, `used` and `free`
/// come together (`statvfs`), or none of them. A dict, so later keys don't change the
/// signature; unknown keys are ignored.
pub type WireUsage = HashMap<String, u64>;

/// What `NativeListWithUsage` returns, D-Bus type `((sssa(sss)as)a{st})`: the list and the
/// usage.
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

/// A user on the bus: `(name, home, encrypted_home)`.
pub type WireUser = (String, String, bool);

/// A [`Config`] on the bus, D-Bus type `(sas)`: `(backup device UUID, filters)`.
pub type WireConfig = (String, Vec<String>);

/// What `ReadConfig` returns, D-Bus type `(s(sas)sa(ssb)as)`: `(config.toml as read or empty,
/// the config in effect, lsblk JSON, users, import notes)`. The notes are empty unless the
/// config was imported from Timeshift's settings.
pub type WireConfigInfo = (String, WireConfig, String, Vec<WireUser>, Vec<String>);

#[must_use]
pub fn config_to_wire(config: &Config) -> WireConfig {
    (config.backup_device_uuid.clone(), config.filters.clone())
}

#[must_use]
pub fn config_from_wire(wire: WireConfig) -> Config {
    let (backup_device_uuid, filters) = wire;
    Config {
        backup_device_uuid,
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
    let (text, config, lsblk, users, imported) = wire;
    if !text.is_empty() {
        Config::parse(&text)?;
    }
    Ok(ConfigInfo {
        text,
        config: config_from_wire(config),
        devices: parse_lsblk(&lsblk)?,
        users: users
            .into_iter()
            .map(|(name, home, encrypted_home)| User {
                name,
                home,
                encrypted_home,
            })
            .collect(),
        imported,
    })
}

/// One browse entry on the bus: `(name, kind, size, mtime, mode, uid, gid, owner, link target,
/// live, live size, live mtime)`. `kind` and `live` are [`Kind::word`] and [`Live::word`].
pub type WireEntry = (
    String,
    String,
    u64,
    i64,
    u32,
    u32,
    u32,
    String,
    String,
    String,
    u64,
    i64,
);

/// What `Browse` returns, D-Bus type `(a(sstxuuussstx)b)`: the entries and `truncated`.
pub type WireListing = (Vec<WireEntry>, bool);

#[must_use]
pub fn listing_to_wire(listing: &Listing) -> WireListing {
    let entries = listing
        .entries
        .iter()
        .map(|e| {
            (
                e.name.clone(),
                e.kind.word().to_owned(),
                e.size,
                e.mtime,
                e.mode,
                e.uid,
                e.gid,
                e.owner.clone(),
                e.target.clone(),
                e.live.word().to_owned(),
                e.live_size,
                e.live_mtime,
            )
        })
        .collect();
    (entries, listing.truncated)
}

/// The [`Listing`] the helper sent.
///
/// # Errors
///
/// [`Error::Helper`] for an unknown kind or live state.
pub fn listing_from_wire(wire: WireListing) -> Result<Listing> {
    let (entries, truncated) = wire;
    let entries = entries
        .into_iter()
        .map(|w| {
            let (
                name,
                kind,
                size,
                mtime,
                mode,
                uid,
                gid,
                owner,
                target,
                live,
                live_size,
                live_mtime,
            ) = w;
            Ok(Entry {
                kind: Kind::from_word(&kind)
                    .ok_or_else(|| Error::Helper(format!("unknown entry kind {kind:?}")))?,
                live: Live::from_word(&live)
                    .ok_or_else(|| Error::Helper(format!("unknown live state {live:?}")))?,
                name,
                size,
                mtime,
                mode,
                uid,
                gid,
                owner,
                target,
                live_size,
                live_mtime,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Listing { entries, truncated })
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// An encoded [`Error::DeviceNotFound`]; the device follows.
const DEVICE_NOT_FOUND_HEADER: &str = "backup device not found: ";
/// An encoded [`Error::InvalidInput`]; the reason follows.
const INVALID_INPUT_HEADER: &str = "refused: ";
/// An encoded [`Error::Restore`]; the reason follows.
const RESTORE_HEADER: &str = "restore failed: ";

/// An error as one message for the bus (a D-Bus error's text, or `Finished`'s `message`).
/// [`Error::DeviceNotFound`], [`Error::InvalidInput`] and [`Error::Restore`] keep their kind, so
/// [`decode_error`] gives them back; anything else is its text.
#[must_use]
pub fn encode_error(error: &Error) -> String {
    match error {
        Error::DeviceNotFound { device } => format!("{DEVICE_NOT_FOUND_HEADER}{device}"),
        Error::InvalidInput(reason) => format!("{INVALID_INPUT_HEADER}{reason}"),
        Error::Restore(reason) => format!("{RESTORE_HEADER}{reason}"),
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
    if let Some(reason) = message.strip_prefix(INVALID_INPUT_HEADER) {
        return Error::InvalidInput(reason.to_owned());
    }
    if let Some(reason) = message.strip_prefix(RESTORE_HEADER) {
        return Error::Restore(reason.to_owned());
    }
    Error::Helper(message.to_owned())
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
    fn configs_survive_the_bus() {
        let config = Config {
            backup_device_uuid: "uuid".to_owned(),
            filters: vec!["+ /root/**".to_owned(), "*.mp3".to_owned()],
        };
        assert_eq!(config_from_wire(config_to_wire(&config)), config);
    }

    #[test]
    fn config_info_is_parsed_on_arrival() {
        let lsblk = include_str!("../../tests/fixtures/lsblk.json");
        let users = vec![("user1".to_owned(), "/home/user1".to_owned(), false)];
        let config = Config {
            backup_device_uuid: "uuid".to_owned(),
            filters: Vec::new(),
        };
        let wire = (
            config.to_text(),
            config_to_wire(&config),
            lsblk.to_owned(),
            users.clone(),
            Vec::new(),
        );
        let info = config_info_from_wire(wire).unwrap();
        assert_eq!(info.saved(), Some(config.clone()));
        assert_eq!(info.devices.len(), 10);
        assert_eq!(info.users[0].home, "/home/user1");
        // No file yet (an import): nothing saved.
        let imported = (
            String::new(),
            config_to_wire(&config),
            lsblk.to_owned(),
            users.clone(),
            vec!["imported".to_owned()],
        );
        let info = config_info_from_wire(imported).unwrap();
        assert_eq!(info.saved(), None);
        assert_eq!(info.config, config);
        let bad = (
            "nope = ".to_owned(),
            config_to_wire(&config),
            lsblk.to_owned(),
            users,
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
    fn restore_errors_survive_the_bus() {
        let refused = Error::InvalidInput("/proc/x: never".to_owned());
        assert!(
            matches!(decode_error(&encode_error(&refused)), Error::InvalidInput(m) if m == "/proc/x: never")
        );
        let failed = Error::Restore("rsync failed with exit code 11".to_owned());
        assert!(
            matches!(decode_error(&encode_error(&failed)), Error::Restore(m) if m.ends_with("11"))
        );
    }

    #[test]
    fn listings_survive_the_bus() {
        let listing = Listing {
            entries: vec![Entry {
                name: "hosts".to_owned(),
                kind: Kind::Link,
                size: 9,
                mtime: -5,
                mode: 0o120_777,
                uid: 0,
                gid: 0,
                owner: "root:root".to_owned(),
                target: "/x".to_owned(),
                live: Live::Changed,
                live_size: 12,
                live_mtime: 1_700_000_000,
            }],
            truncated: true,
        };
        assert_eq!(
            listing_from_wire(listing_to_wire(&listing)).unwrap(),
            listing
        );
        let mut bad = listing_to_wire(&listing);
        bad.0[0].1 = "pipe".to_owned();
        assert!(matches!(listing_from_wire(bad), Err(Error::Helper(_))));
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
