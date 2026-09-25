// SPDX-License-Identifier: GPL-3.0-only

//! Timeshift's settings: `/etc/timeshift/timeshift.json`, the devices it can back up to, and the
//! users whose home folders its filters cover.
//!
//! Everything here follows Timeshift's own code (linuxmint/timeshift e7e54ab: `Main.vala`
//! `save_app_config` / `load_app_config`, `UsersBox.vala`, `ExcludeBox.vala`, `Device.vala`), so
//! `timeshift-launcher` and Apsis can edit the same file:
//!
//! - Every value is a JSON string (`"true"`, `"5"`): Timeshift reads each one with
//!   `get_string_member`. [`Config::parse`] refuses a file where one of its fields isn't.
//! - Only the fields in [`Settings`] change. Every other field stays as it was, in its place.
//! - The file is written the way json-glib writes it (2-space indent, `"key" : value`, no final
//!   newline), so an unchanged file comes out byte for byte the same.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// Where Timeshift keeps its settings.
pub const CONFIG_PATH: &str = "/etc/timeshift/timeshift.json";
/// The one backup of the previous settings the helper keeps.
pub const BACKUP_PATH: &str = "/etc/timeshift/timeshift.json.bak";
/// Retention counts, as Timeshift's own spin buttons allow. A 0 would make Timeshift's cleanup
/// untag (and so delete) every uncommented snapshot of that level.
pub const MIN_COUNT: u32 = 1;
pub const MAX_COUNT: u32 = 999;
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

const BACKUP_DEVICE_UUID: &str = "backup_device_uuid";
const PARENT_DEVICE_UUID: &str = "parent_device_uuid";
const BTRFS_MODE: &str = "btrfs_mode";
/// Older Timeshift versions' name; when it's there Timeshift reads it instead of the new one.
const INCLUDE_BTRFS_HOME_OLD: &str = "include_btrfs_home";
const INCLUDE_BTRFS_HOME: &str = "include_btrfs_home_for_backup";
const EXCLUDE: &str = "exclude";
const EXCLUDE_APPS: &str = "exclude-apps";

/// Fields Timeshift reads as strings.
const STRING_FIELDS: [&str; 22] = [
    BACKUP_DEVICE_UUID,
    PARENT_DEVICE_UUID,
    "do_first_run",
    BTRFS_MODE,
    INCLUDE_BTRFS_HOME_OLD,
    INCLUDE_BTRFS_HOME,
    "include_btrfs_home_for_restore",
    "stop_cron_emails",
    "schedule_monthly",
    "schedule_weekly",
    "schedule_daily",
    "schedule_hourly",
    "schedule_boot",
    "count_monthly",
    "count_weekly",
    "count_daily",
    "count_hourly",
    "count_boot",
    "snapshot_size",
    "snapshot_count",
    "date_format",
    "pause_snapshots",
];

/// A schedule level: one `schedule_<level>` and one `count_<level>` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Monthly,
    Weekly,
    Daily,
    Hourly,
    Boot,
}

impl Level {
    /// In the order of Timeshift's Schedule tab.
    pub const ALL: [Self; 5] = [
        Self::Monthly,
        Self::Weekly,
        Self::Daily,
        Self::Hourly,
        Self::Boot,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Monthly => "monthly",
            Self::Weekly => "weekly",
            Self::Daily => "daily",
            Self::Hourly => "hourly",
            Self::Boot => "boot",
        }
    }

    /// Index into [`Settings::schedule`] and [`Settings::counts`].
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    fn schedule_field(self) -> String {
        format!("schedule_{}", self.name())
    }

    fn count_field(self) -> String {
        format!("count_{}", self.name())
    }

    /// Timeshift's defaults when the field is missing.
    fn default_count(self) -> u32 {
        match self {
            Self::Monthly => 2,
            Self::Weekly => 3,
            Self::Daily | Self::Boot => 5,
            Self::Hourly => 6,
        }
    }
}

/// What Apsis edits in Timeshift's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Filesystem UUID of the backup device; empty when none is selected.
    pub backup_device_uuid: String,
    /// btrfs snapshots instead of rsync.
    pub btrfs_mode: bool,
    /// btrfs mode: back up the `@home` subvolume too. (rsync mode covers home folders with
    /// filters, see [`User`].)
    pub include_btrfs_home: bool,
    /// Per [`Level`], in [`Level::ALL`] order.
    pub schedule: [bool; 5],
    /// How many snapshots to keep per [`Level`], in [`Level::ALL`] order.
    pub counts: [u32; 5],
    /// Timeshift's filter list, in order (rsync uses the first match). `+ ` in front includes,
    /// anything else excludes. The home folder patterns of [`User`] live here too.
    pub exclude: Vec<String>,
}

impl Settings {
    #[must_use]
    pub fn scheduled(&self, level: Level) -> bool {
        self.schedule[level.index()]
    }

    #[must_use]
    pub fn count(&self, level: Level) -> u32 {
        self.counts[level.index()]
    }
}

/// Timeshift's settings file, every field kept in its place.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    fields: Map<String, Value>,
}

impl Config {
    /// Reads a settings file, refusing one Timeshift itself would misread: not a JSON object,
    /// a string field that isn't a string, or a filter list that isn't a list of strings.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfig`], saying what's wrong.
    pub fn parse(text: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| Error::InvalidConfig(format!("not valid JSON: {e}")))?;
        let Value::Object(fields) = value else {
            return Err(Error::InvalidConfig("not a JSON object".to_owned()));
        };
        for field in STRING_FIELDS {
            if fields.get(field).is_some_and(|v| !v.is_string()) {
                return Err(Error::InvalidConfig(format!("{field} is not a string")));
            }
        }
        for field in [EXCLUDE, EXCLUDE_APPS] {
            let strings = |v: &Value| v.as_array().is_some_and(|a| a.iter().all(Value::is_string));
            if fields.get(field).is_some_and(|v| !strings(v)) {
                return Err(Error::InvalidConfig(format!(
                    "{field} is not a list of strings"
                )));
            }
        }
        Ok(Self { fields })
    }

    /// The settings as Timeshift reads them, with Timeshift's defaults for missing fields.
    #[must_use]
    pub fn settings(&self) -> Settings {
        let include_btrfs_home = if self.fields.contains_key(INCLUDE_BTRFS_HOME_OLD) {
            self.flag(INCLUDE_BTRFS_HOME_OLD)
        } else {
            self.flag(INCLUDE_BTRFS_HOME)
        };
        Settings {
            backup_device_uuid: self
                .string(BACKUP_DEVICE_UUID)
                .unwrap_or_default()
                .to_owned(),
            btrfs_mode: self.flag(BTRFS_MODE),
            include_btrfs_home,
            schedule: Level::ALL.map(|level| self.flag(&level.schedule_field())),
            counts: Level::ALL.map(|level| {
                self.string(&level.count_field())
                    .map_or(level.default_count(), vala_int)
            }),
            exclude: self
                .fields
                .get(EXCLUDE)
                .and_then(Value::as_array)
                .map(|patterns| {
                    patterns
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// UUID of the backup device's parent (LUKS or LVM container), empty for none.
    #[must_use]
    pub fn parent_device_uuid(&self) -> &str {
        self.string(PARENT_DEVICE_UUID).unwrap_or_default()
    }

    /// Writes `settings` into the fields Timeshift reads them from, all as strings. Fields
    /// already there keep their place; missing ones go at the end.
    fn apply(&mut self, settings: &Settings, parent_device_uuid: &str) {
        let flag = |on: bool| Value::from(if on { "true" } else { "false" });
        self.set(
            BACKUP_DEVICE_UUID,
            Value::from(settings.backup_device_uuid.as_str()),
        );
        self.set(PARENT_DEVICE_UUID, Value::from(parent_device_uuid));
        self.set(BTRFS_MODE, flag(settings.btrfs_mode));
        if self.fields.contains_key(INCLUDE_BTRFS_HOME_OLD) {
            self.set(INCLUDE_BTRFS_HOME_OLD, flag(settings.include_btrfs_home));
        }
        self.set(INCLUDE_BTRFS_HOME, flag(settings.include_btrfs_home));
        for level in Level::ALL {
            self.set(&level.schedule_field(), flag(settings.scheduled(level)));
            self.set(
                &level.count_field(),
                Value::from(settings.count(level).to_string()),
            );
        }
        let exclude = settings.exclude.iter().map(|p| Value::from(p.as_str()));
        self.set(EXCLUDE, Value::Array(exclude.collect()));
    }

    fn set(&mut self, field: &str, value: Value) {
        self.fields.insert(field.to_owned(), value);
    }

    fn string(&self, field: &str) -> Option<&str> {
        self.fields.get(field).and_then(Value::as_str)
    }

    /// Vala's `bool.parse`: only `"true"` is true.
    fn flag(&self, field: &str) -> bool {
        self.string(field) == Some("true")
    }

    /// The file as json-glib writes it (see the module docs).
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        write_object(&mut out, &self.fields, 0);
        out
    }
}

/// Vala's `int.parse` for a count: the number, or 0 for anything that isn't one (a negative
/// number too, which isn't a count).
fn vala_int(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

fn indent(out: &mut String, depth: usize) {
    out.extend(std::iter::repeat_n("  ", depth));
}

/// json-glib's pretty printing: `{}` / `[]` when empty, else one member per line.
fn write_object(out: &mut String, fields: &Map<String, Value>, depth: usize) {
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

/// The new settings file: `current` with `new` applied, after checking `new` against the
/// devices there are now.
///
/// The backup device's parent UUID comes from `devices` when the device changes, and stays as
/// it was when it doesn't (Timeshift keeps it too while the device isn't connected). The
/// result is read back and must give exactly `new`.
///
/// # Errors
///
/// [`Error::InvalidConfig`] if `current` can't be edited safely, [`Error::InvalidSettings`] if
/// `new` doesn't pass [`validate`].
pub fn edit(current: &str, new: &Settings, devices: &[Device]) -> Result<String> {
    let mut config = Config::parse(current)?;
    let old = config.settings();
    validate(new, &old, devices)?;
    let parent = if new.backup_device_uuid == old.backup_device_uuid {
        config.parent_device_uuid().to_owned()
    } else {
        devices
            .iter()
            .find(|d| d.uuid == new.backup_device_uuid)
            .map(|d| d.parent_uuid.clone())
            .unwrap_or_default()
    };
    config.apply(new, &parent);
    let text = config.to_text();
    let reread = Config::parse(&text)?;
    if reread.settings() != *new || reread.parent_device_uuid() != parent {
        return Err(Error::InvalidConfig(
            "the new file doesn't read back as written".to_owned(),
        ));
    }
    Ok(text)
}

/// Checks `new` before it's written. `old` is what the file says now, `devices` what's
/// connected now.
///
/// - A new backup device must be connected and selectable ([`Device::selectable`]); in btrfs
///   mode it must be btrfs. An unchanged one may be unplugged, unless btrfs mode is being
///   turned on (the device must then be there to be checked).
/// - btrfs mode can only be turned on when there's a btrfs filesystem ([`btrfs_available`]).
/// - Retention counts are [`MIN_COUNT`]..=[`MAX_COUNT`].
/// - Filters pass [`validate_filter`] and aren't repeated.
///
/// # Errors
///
/// [`Error::InvalidSettings`] with the first problem found.
pub fn validate(new: &Settings, old: &Settings, devices: &[Device]) -> Result<()> {
    let invalid = |reason: String| Err(Error::InvalidSettings(reason));
    if new.btrfs_mode && !old.btrfs_mode && !btrfs_available(devices) {
        return invalid("btrfs mode needs a btrfs filesystem, and there is none".to_owned());
    }
    let device = devices
        .iter()
        .find(|d| !d.uuid.is_empty() && d.uuid == new.backup_device_uuid);
    if new.backup_device_uuid != old.backup_device_uuid {
        match device {
            _ if new.backup_device_uuid.is_empty() => {
                return invalid("choose a backup device".to_owned());
            }
            None => return invalid("the chosen backup device isn't connected".to_owned()),
            Some(device) if !device.selectable() => {
                return invalid(format!("{} can't hold snapshots", device.path()));
            }
            Some(_) => {}
        }
    }
    if new.btrfs_mode && !old.btrfs_mode && device.is_none() {
        return invalid("btrfs mode needs a connected btrfs backup device".to_owned());
    }
    if let Some(device) = device.filter(|_| new.btrfs_mode)
        && device.fstype != "btrfs"
    {
        return invalid(format!(
            "btrfs mode needs a btrfs device; {} is {}",
            device.path(),
            device.fstype
        ));
    }
    for level in Level::ALL {
        let count = new.count(level);
        if !(MIN_COUNT..=MAX_COUNT).contains(&count) {
            return invalid(format!(
                "keep {MIN_COUNT} to {MAX_COUNT} {} snapshots, not {count}",
                level.name()
            ));
        }
    }
    for (i, pattern) in new.exclude.iter().enumerate() {
        validate_filter(pattern)?;
        if new.exclude[..i].contains(pattern) {
            return invalid(format!("filter {pattern:?} is there twice"));
        }
    }
    Ok(())
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
    /// LVM or ZFS container), with a UUID, and not inside an encrypted container (Apsis leaves
    /// encrypted backup devices to Timeshift for now).
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

/// Timeshift's check before offering btrfs mode: some filesystem is btrfs.
#[must_use]
pub fn btrfs_available(devices: &[Device]) -> bool {
    devices.iter().any(|d| d.fstype == "btrfs")
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

impl HomeState {
    /// The next state, for a key or click that cycles through them.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Excluded => Self::Hidden,
            Self::Hidden => Self::All,
            Self::All => Self::Excluded,
        }
    }
}

/// A user whose home folder Timeshift's Users tab lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub home: String,
    /// An ecryptfs home: Timeshift uses other patterns for it. Apsis only shows it.
    pub encrypted_home: bool,
}

impl User {
    /// Timeshift's patterns for this user: (exclude, include all, include hidden).
    fn patterns(&self) -> [String; 3] {
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

    /// Sets it like Timeshift does: the state's pattern is added at the end (if it isn't there
    /// yet) and the other two are removed.
    pub fn set_home_state(&self, exclude: &mut Vec<String>, state: HomeState) {
        let [excluded, all, hidden] = self.patterns();
        let (keep, drop) = match state {
            HomeState::Excluded => (excluded, [all, hidden]),
            HomeState::Hidden => (hidden, [excluded, all]),
            HomeState::All => (all, [excluded, hidden]),
        };
        exclude.retain(|p| !drop.contains(p));
        if !exclude.contains(&keep) {
            exclude.push(keep);
        }
    }

    /// Whether `pattern` is one of this user's home folder patterns.
    #[must_use]
    pub fn owns(&self, pattern: &str) -> bool {
        self.patterns().iter().any(|p| p == pattern)
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

/// Everything the settings view shows: the file as read, and the system it applies to.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsInfo {
    /// The file exactly as read. Sent back with a write, which the helper refuses if the file
    /// has changed since.
    pub text: String,
    pub config: Config,
    pub devices: Vec<Device>,
    pub users: Vec<User>,
    /// `timeshift-gtk` is open: it saves its own copy of the settings when it closes.
    pub timeshift_gui_open: bool,
}
