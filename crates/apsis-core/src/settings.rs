// SPDX-License-Identifier: GPL-3.0-only

//! What the settings view and the config work with: the devices snapshots can go to (`lsblk`),
//! the users whose home folders the filters cover (`/etc/passwd`), the home folder patterns and
//! filter checks, and the json-glib style JSON writer `info.json` is written with.
//!
//! The home patterns, filter rules and device checks are Timeshift's (linuxmint/timeshift:
//! `UsersBox.vala`, `ExcludeBox.vala`, `Device.vala`), so a config imported from Timeshift keeps
//! backing up exactly what it did.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// Longest filter pattern accepted, in bytes (Linux's `PATH_MAX`).
pub const MAX_FILTER_BYTES: usize = 4096;

/// `lsblk` as the helper runs it: every block device, flat, sizes in bytes. The same devices
/// Timeshift's `--list-devices` shows (it runs lsblk too), plus their UUIDs.
pub const LSBLK_ARGS: [&str; 6] = [
    "lsblk",
    "--json",
    "--list",
    "--bytes",
    "--output",
    "NAME,KNAME,PKNAME,TYPE,FSTYPE,UUID,SIZE,LABEL",
];

fn indent(out: &mut String, depth: usize) {
    out.extend(std::iter::repeat_n("  ", depth));
}

/// json-glib's pretty printing: `{}` / `[]` when empty, else one member per line.
pub(crate) fn write_object(out: &mut String, fields: &Map<String, Value>, depth: usize) {
    out.push('{');
    if !fields.is_empty() {
        for (i, (key, value)) in fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('\n');
            indent(out, depth + 1);
            out.push_str(&Value::from(key.as_str()).to_string());
            out.push_str(" : ");
            write_value(out, value, depth + 1);
        }
        out.push('\n');
        indent(out, depth);
    }
    out.push('}');
}

fn write_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Object(fields) => write_object(out, fields, depth),
        Value::Array(items) => {
            out.push('[');
            if !items.is_empty() {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    indent(out, depth + 1);
                    write_value(out, item, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A filter as Timeshift's filter editor takes it: any rsync pattern that isn't blank (after an
/// optional `+ `, which makes it an include). No control characters or newlines, which would
/// break the pattern file Timeshift hands rsync.
///
/// # Errors
///
/// [`Error::InvalidSettings`] saying why.
pub fn validate_filter(pattern: &str) -> Result<()> {
    let invalid = |reason: &str| Err(Error::InvalidSettings(reason.to_owned()));
    let body = pattern.strip_prefix("+ ").unwrap_or(pattern);
    if body.trim().is_empty() {
        return invalid("a filter can't be blank");
    }
    if pattern.chars().any(char::is_control) {
        return invalid("a filter can't contain control characters or line breaks");
    }
    if pattern.len() > MAX_FILTER_BYTES {
        return invalid("a filter can't be that long");
    }
    Ok(())
}

/// A block device, as `lsblk` ([`LSBLK_ARGS`]) reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// `sdb1`, `luks-…`, `vg-root`: `/dev/<name>` or `/dev/mapper/<name>`.
    pub name: String,
    /// The kernel's name (`sdb1`, `dm-0`).
    pub kname: String,
    /// `disk`, `part`, `crypt`, `lvm`, …
    pub kind: String,
    /// Empty when there's no filesystem.
    pub fstype: String,
    /// Filesystem UUID; empty when there's none.
    pub uuid: String,
    pub label: String,
    pub size: u64,
    /// UUID of the device holding this one (Timeshift's `parent_device_uuid`); empty for a
    /// partition on a plain disk, which has no UUID.
    pub parent_uuid: String,
}

impl Device {
    /// Where to find it.
    #[must_use]
    pub fn path(&self) -> String {
        if matches!(self.kind.as_str(), "crypt" | "lvm" | "dm") {
            format!("/dev/mapper/{}", self.name)
        } else {
            format!("/dev/{}", self.name)
        }
    }

    /// Timeshift's `has_linux_filesystem`: the devices `--list-devices` shows.
    #[must_use]
    pub fn is_linux(&self) -> bool {
        matches!(
            self.fstype.to_lowercase().as_str(),
            "ext2"
                | "ext3"
                | "ext4"
                | "f2fs"
                | "reiserfs"
                | "reiser4"
                | "xfs"
                | "jfs"
                | "zfs"
                | "zfs_member"
                | "btrfs"
                | "lvm"
                | "lvm2"
                | "lvm2_member"
                | "luks"
                | "crypt"
                | "crypto_luks"
        )
    }

    /// Whether Apsis offers it as a backup device: a Linux filesystem of its own (not a LUKS,
    /// LVM or ZFS container), with a UUID, and not inside an encrypted container (encrypted
    /// backup devices aren't supported yet).
    #[must_use]
    pub fn selectable(&self) -> bool {
        let container = matches!(
            self.fstype.to_lowercase().as_str(),
            "zfs_member" | "lvm" | "lvm2" | "lvm2_member" | "luks" | "crypt" | "crypto_luks"
        );
        self.is_linux() && !container && !self.uuid.is_empty() && self.kind != "crypt"
    }
}

/// Every block device `lsblk` ([`LSBLK_ARGS`]) listed, once each, with its parent's UUID.
///
/// # Errors
///
/// [`Error::Helper`] if the output isn't lsblk's JSON.
pub fn parse_lsblk(json: &str) -> Result<Vec<Device>> {
    let bad = |what: &str| Error::Helper(format!("unexpected lsblk output: {what}"));
    let value: Value = serde_json::from_str(json).map_err(|e| bad(&e.to_string()))?;
    let rows = value
        .get("blockdevices")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no blockdevices"))?;
    let text = |row: &Value, key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut devices: Vec<(Device, String)> = Vec::new();
    for row in rows {
        let kname = text(row, "kname");
        // An LVM volume on several disks is listed once per disk: keep the first.
        if kname.is_empty() || devices.iter().any(|(d, _)| d.kname == kname) {
            continue;
        }
        // Older lsblk prints sizes as strings.
        let size = match row.get("size") {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        let device = Device {
            name: text(row, "name"),
            kname,
            kind: text(row, "type"),
            fstype: text(row, "fstype"),
            uuid: text(row, "uuid"),
            label: text(row, "label"),
            size,
            parent_uuid: String::new(),
        };
        devices.push((device, text(row, "pkname")));
    }
    let uuids: Vec<(String, String)> = devices
        .iter()
        .map(|(d, _)| (d.kname.clone(), d.uuid.clone()))
        .collect();
    Ok(devices
        .into_iter()
        .map(|(mut device, pkname)| {
            device.parent_uuid = uuids
                .iter()
                .find(|(kname, _)| !pkname.is_empty() && *kname == pkname)
                .map(|(_, uuid)| uuid.clone())
                .unwrap_or_default();
            device
        })
        .collect())
}

/// Which files of a user's home folder rsync mode backs up (Timeshift's Users tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeState {
    /// Nothing (Timeshift's default).
    Excluded,
    /// Hidden files and folders only (settings).
    Hidden,
    /// Everything.
    All,
}

/// A user whose home folder Timeshift's Users tab lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub home: String,
    /// An ecryptfs home: Timeshift's other patterns for it. Apsis shows it and doesn't change it.
    pub encrypted_home: bool,
}

impl User {
    /// Timeshift's patterns for this user: (exclude, include all, include hidden).
    pub(crate) fn patterns(&self) -> [String; 3] {
        let hidden = format!("+ {}/.**", self.home);
        if self.encrypted_home {
            let path = format!("/home/.ecryptfs/{}/***", self.name);
            [path.clone(), format!("+ {path}"), hidden]
        } else {
            [
                format!("{}/**", self.home),
                format!("+ {}/**", self.home),
                hidden,
            ]
        }
    }

    /// Read like Timeshift reads it: an include pattern decides, else it's excluded.
    #[must_use]
    pub fn home_state(&self, exclude: &[String]) -> HomeState {
        let [_, all, hidden] = self.patterns();
        if exclude.contains(&hidden) {
            HomeState::Hidden
        } else if exclude.contains(&all) {
            HomeState::All
        } else {
            HomeState::Excluded
        }
    }
}

/// The users Timeshift lists from `/etc/passwd`: root and everyone who isn't a system account
/// (uid below 1000, or 65534 `nobody`), by name. `encrypted_home` is left false; the helper
/// checks it.
#[must_use]
pub fn parse_passwd(text: &str) -> Vec<User> {
    let mut users: Vec<User> = text
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, uid, _, _, home, ..] = fields.as_slice() else {
                return None;
            };
            let uid: u32 = uid.trim().parse().ok()?;
            let system = (uid != 0 && uid < 1000) || uid == 65534;
            let (name, home) = (name.trim(), home.trim());
            (!system && !name.is_empty() && home.starts_with('/')).then(|| User {
                name: name.to_owned(),
                home: home.to_owned(),
                encrypted_home: false,
            })
        })
        .collect();
    users.sort_by(|a, b| a.name.cmp(&b.name));
    users
}
