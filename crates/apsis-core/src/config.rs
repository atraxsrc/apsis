// SPDX-License-Identifier: GPL-3.0-only

//! Apsis's config, `/etc/apsis/config.toml`: the backup device, what a snapshot includes
//! besides the system, and the filter list. Only `apsis-helper` writes it.
//!
//! ```toml
//! # Written by apsis-helper; change it in Apsis's settings.
//! # filters: first match wins, top to bottom; "- " leaves out, "+ " keeps.
//! # Built-in excludes (/proc, /dev, caches, ...) always come first.
//! version = 2
//! backup_device_uuid = "8cecb045-975d-49d0-bd57-1ec9f6eb77b5"
//! include_root = true
//! include_home = false
//! filters = [
//!     "+ /home/user1/Videos/keep/***",
//!     "- /home/*/Videos/***",
//! ]
//! ```
//!
//! **Older settings.** Apsis 0.2 and 0.3 wrote version 1: the same device, and one filter list
//! in Timeshift's format with each user's home folder patterns in it (`<home>/**` nothing,
//! `+ <home>/.**` hidden files only, `+ <home>/**` everything). Timeshift's own
//! `/etc/timeshift/timeshift.json`, read once while there's no `config.toml`, has that format
//! too. Both are converted by [`convert`], which never changes what a snapshot holds (one
//! documented exception, see [`convert`]); the result is used at once and written as version 2
//! when the user saves the settings.

use serde_json::Value as Json;

use crate::error::{Error, Result};
use crate::settings::{Device, HomeState, MAX_FILTER_BYTES, User, validate_filter};

/// Where the config lives.
pub const CONFIG_PATH: &str = "/etc/apsis/config.toml";
/// The one backup of the previous config the helper keeps.
pub const BACKUP_PATH: &str = "/etc/apsis/config.toml.bak";
/// Timeshift's settings, read once for the import.
pub const TIMESHIFT_CONFIG: &str = "/etc/timeshift/timeshift.json";
/// The format this code writes.
const VERSION: i64 = 2;
/// The format Apsis 0.2 and 0.3 wrote; read and converted.
const LEGACY_VERSION: i64 = 1;

/// What a snapshot is taken to, and what it holds besides the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Filesystem UUID of the backup device; empty when none is chosen.
    pub backup_device_uuid: String,
    /// `/root`, the root user's home folder.
    pub include_root: bool,
    /// `/home`, every user's home folder.
    pub include_home: bool,
    /// rsync filters, in order, each `+ pattern` or `- pattern`; the first that matches wins.
    pub filters: Vec<String>,
}

impl Default for Config {
    /// A fresh install: no device, `/root` in, `/home` out, no filters.
    fn default() -> Self {
        Self {
            backup_device_uuid: String::new(),
            include_root: true,
            include_home: false,
            filters: Vec::new(),
        }
    }
}

/// Settings in the old format (Apsis 0.2 and 0.3, or Timeshift's): a device and one filter
/// list in Timeshift's format, home folder patterns included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Legacy {
    pub backup_device_uuid: String,
    pub filters: Vec<String>,
}

/// A `config.toml` as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    Current(Config),
    /// Version 1: convert it with [`convert`].
    Legacy(Legacy),
}

impl Stored {
    /// The backup device the file names.
    #[must_use]
    pub fn device(&self) -> &str {
        match self {
            Self::Current(config) => &config.backup_device_uuid,
            Self::Legacy(legacy) => &legacy.backup_device_uuid,
        }
    }
}

impl Config {
    /// Reads `config.toml`: version 2 as it is, version 1 for [`convert`].
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfig`]: not TOML, another version, or a field of the wrong type.
    pub fn read(text: &str) -> Result<Stored> {
        let invalid = |reason: String| Error::InvalidConfig(format!("{CONFIG_PATH}: {reason}"));
        let value: toml::Value = text
            .parse()
            .map_err(|e| invalid(format!("not valid TOML: {e}")))?;
        let table = value
            .as_table()
            .ok_or_else(|| invalid("not a table".to_owned()))?;
        let version = table.get("version").and_then(toml::Value::as_integer);
        if version != Some(VERSION) && version != Some(LEGACY_VERSION) {
            return Err(invalid(format!(
                "version {} (this Apsis reads {LEGACY_VERSION} and {VERSION})",
                version.map_or_else(|| "missing".to_owned(), |v| v.to_string())
            )));
        }
        let backup_device_uuid = match table.get("backup_device_uuid") {
            None => String::new(),
            Some(toml::Value::String(uuid)) => uuid.clone(),
            Some(_) => return Err(invalid("backup_device_uuid is not a string".to_owned())),
        };
        let filters = match table.get("filters") {
            None => Vec::new(),
            Some(toml::Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| invalid("filters is not a list of strings".to_owned()))?,
            Some(_) => return Err(invalid("filters is not a list".to_owned())),
        };
        if version == Some(LEGACY_VERSION) {
            return Ok(Stored::Legacy(Legacy {
                backup_device_uuid,
                filters,
            }));
        }
        let flag = |key: &str, default: bool| match table.get(key) {
            None => Ok(default),
            Some(toml::Value::Boolean(value)) => Ok(*value),
            Some(_) => Err(invalid(format!("{key} is not true or false"))),
        };
        let defaults = Self::default();
        Ok(Stored::Current(Self {
            backup_device_uuid,
            include_root: flag("include_root", defaults.include_root)?,
            include_home: flag("include_home", defaults.include_home)?,
            filters,
        }))
    }

    /// The file's text: a comment, the version, the device, the includes, one filter per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let quote = |text: &str| toml::Value::String(text.to_owned()).to_string();
        let mut out = String::from(
            "# Written by apsis-helper; change it in Apsis's settings.\n\
             # filters: first match wins, top to bottom; \"- \" leaves out, \"+ \" keeps.\n\
             # Built-in excludes (/proc, /dev, caches, ...) always come first.\n",
        );
        out.push_str(&format!("version = {VERSION}\n"));
        out.push_str(&format!(
            "backup_device_uuid = {}\n",
            quote(&self.backup_device_uuid)
        ));
        out.push_str(&format!("include_root = {}\n", self.include_root));
        out.push_str(&format!("include_home = {}\n", self.include_home));
        if self.filters.is_empty() {
            out.push_str("filters = []\n");
        } else {
            out.push_str("filters = [\n");
            for filter in &self.filters {
                out.push_str(&format!("    {},\n", quote(filter)));
            }
            out.push_str("]\n");
        }
        out
    }
}

/// A filter as version 2 keeps it: `+ ` or `- `, then a pattern that isn't blank, with no
/// control characters or line breaks (they'd break the pattern file rsync reads), at most
/// [`MAX_FILTER_BYTES`].
///
/// # Errors
///
/// [`Error::InvalidSettings`] saying why.
pub fn validate_signed_filter(filter: &str) -> Result<()> {
    let invalid = |reason: &str| Err(Error::InvalidSettings(reason.to_owned()));
    let Some(body) = filter
        .strip_prefix("+ ")
        .or_else(|| filter.strip_prefix("- "))
    else {
        return invalid("a filter starts with + (include) or - (exclude)");
    };
    if body.trim().is_empty() {
        return invalid("a filter can't be blank");
    }
    if filter.chars().any(char::is_control) {
        return invalid("a filter can't contain control characters or line breaks");
    }
    if filter.len() > MAX_FILTER_BYTES {
        return invalid("a filter can't be that long");
    }
    Ok(())
}

/// Checks a config before it's written (the helper checks again).
///
/// - A backup device that differs from the saved one (`old_device`; `None` when there's no
///   config file yet) must be connected, and a plain unencrypted Linux filesystem
///   ([`Device::selectable`]). An unchanged one may be unplugged.
/// - Filters pass [`validate_signed_filter`] and aren't repeated.
///
/// # Errors
///
/// [`Error::InvalidSettings`] with the first problem found.
pub fn validate(new: &Config, old_device: Option<&str>, devices: &[Device]) -> Result<()> {
    let invalid = |reason: String| Err(Error::InvalidSettings(reason));
    let changed = old_device.is_none_or(|old| old != new.backup_device_uuid);
    if changed {
        let device = devices
            .iter()
            .find(|d| !d.uuid.is_empty() && d.uuid == new.backup_device_uuid);
        match device {
            _ if new.backup_device_uuid.is_empty() => {
                return invalid("choose a backup device".to_owned());
            }
            None => return invalid("the chosen backup device isn't connected".to_owned()),
            Some(device) if !device.selectable() => {
                return invalid(format!(
                    "{} can't hold snapshots: it must be an unencrypted Linux filesystem",
                    device.path()
                ));
            }
            Some(_) => {}
        }
    }
    for (i, filter) in new.filters.iter().enumerate() {
        validate_signed_filter(filter)?;
        if new.filters[..i].contains(filter) {
            return invalid(format!("filter {filter:?} is there twice"));
        }
    }
    Ok(())
}

/// What the converter needs to know about this system: the users (`/etc/passwd`, as
/// [`crate::settings::parse_passwd`] reads it, ecryptfs homes marked) and the names in `/home`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct System {
    pub users: Vec<User>,
    pub home_entries: Vec<String>,
}

/// Where a user's home folder is, for the conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Root,
    /// Under `/home` (an ecryptfs home too).
    Home,
    Elsewhere,
}

fn place(user: &User) -> Place {
    let home = user.home.trim_end_matches('/');
    if home == "/root" {
        Place::Root
    } else if home.starts_with("/home/") {
        Place::Home
    } else {
        Place::Elsewhere
    }
}

/// Which of a user's patterns `pattern` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Exclude,
    All,
    Hidden,
}

fn kind_of(user: &User, pattern: &str) -> Option<Kind> {
    let [exclude, all, hidden] = user.patterns();
    if pattern == exclude {
        Some(Kind::Exclude)
    } else if pattern == all {
        Some(Kind::All)
    } else if pattern == hidden {
        Some(Kind::Hidden)
    } else {
        None
    }
}

/// `pattern` with a sign: `+ x` stays, `- x` stays, a bare `x` (an exclude in Timeshift's
/// format) becomes `- x`.
fn signed(pattern: &str) -> String {
    if pattern.starts_with("+ ") || pattern.starts_with("- ") {
        pattern.to_owned()
    } else {
        format!("- {pattern}")
    }
}

/// Whether `pattern` (a filter's body) could match something inside `folder`: it isn't
/// anchored (no leading `/`, so rsync tries it on every name), or it's anchored at or under
/// `folder`.
fn could_match_inside(pattern: &str, folder: &str) -> bool {
    let body = pattern
        .strip_prefix("+ ")
        .or_else(|| pattern.strip_prefix("- "))
        .unwrap_or(pattern);
    !body.starts_with('/') || body == folder || body.starts_with(&format!("{folder}/"))
}

/// Converts old settings (see [`Legacy`]) without changing what a snapshot holds. Returns the
/// config and notes on what was done, one line each.
///
/// 1. Every bare pattern gets `- `; `+ ` patterns stay; the order is kept.
/// 2. `/root`: root's home state `everything` turns `include_root` on; `excluded` leaves it
///    off; `hidden files only` leaves it off and keeps `+ /root/.**` as a row. Root's other
///    patterns go (the built-in `/root/**` covers them).
/// 3. `/home`: `include_home` is on only if every user with a home under `/home/` is
///    `everything`, every name in `/home` (but `lost+found` and `.ecryptfs`) is one of their
///    homes, and no filter after a dropped `+ <home>/**` could match inside that home. Else
///    it's off and each include stays as a row where it was (`hidden files only` becomes a
///    `+ <home>/.**` row). A plain `<home>/**` exclude goes (the built-in `/home/*/**`).
/// 4. A user whose home is elsewhere and was `excluded` gets `- <home>/**`: version 2 has no
///    per-user exclude step, so it would otherwise count as system files.
///
/// The exception: version 2's rsync list adds each kept folder's parents in front of a `+`
/// filter ([`crate::native::exclude::for_backup`]), so a `+` filter that a later `-` used to
/// hide (and that therefore did nothing) starts to work.
#[must_use]
pub fn convert(legacy: &Legacy, system: &System) -> (Config, Vec<String>) {
    let filters = &legacy.filters;
    let owner = |pattern: &str| {
        system
            .users
            .iter()
            .find_map(|user| kind_of(user, pattern).map(|kind| (user, kind)))
    };
    let root = system.users.iter().find(|u| place(u) == Place::Root);
    let root_state = root.map(|u| u.home_state(filters));
    let include_root = root_state == Some(HomeState::All);

    let home_users: Vec<&User> = system
        .users
        .iter()
        .filter(|u| place(u) == Place::Home)
        .collect();
    let all_everything = !home_users.is_empty()
        && home_users
            .iter()
            .all(|u| u.home_state(filters) == HomeState::All);
    let unknown_entries: Vec<&String> = system
        .home_entries
        .iter()
        .filter(|name| *name != "lost+found" && *name != ".ecryptfs")
        .filter(|name| {
            !home_users
                .iter()
                .any(|u| u.home.trim_end_matches('/') == format!("/home/{name}"))
        })
        .collect();
    // A filter after a home's `+ <home>/**` that could match inside it had no effect there;
    // with the home in `/home` (after all filters) it would.
    let shadowed: Vec<&String> = filters
        .iter()
        .enumerate()
        .filter_map(|(i, pattern)| match owner(pattern) {
            Some((user, Kind::All)) if place(user) == Place::Home => Some((i, user)),
            _ => None,
        })
        .flat_map(|(i, user)| {
            let home = user.home.trim_end_matches('/').to_owned();
            let ecryptfs = format!("/home/.ecryptfs/{}", user.name);
            filters[i + 1..].iter().filter(move |later| {
                owner(later).is_none()
                    && (could_match_inside(later, &home) || could_match_inside(later, &ecryptfs))
            })
        })
        .collect();
    let include_home = all_everything && unknown_entries.is_empty() && shadowed.is_empty();

    let mut rows: Vec<String> = Vec::new();
    let mut kept_home_rows: Vec<String> = Vec::new();
    for pattern in filters {
        let row = match owner(pattern) {
            Some((user, kind)) => match (place(user), kind) {
                (Place::Root | Place::Home, Kind::Exclude) => None,
                (Place::Root, Kind::All) => None,
                (Place::Home, Kind::All) if include_home => None,
                (Place::Root | Place::Home, _) => {
                    kept_home_rows.push(format!("{} ({})", signed(pattern), user.name));
                    Some(signed(pattern))
                }
                (Place::Elsewhere, _) => Some(signed(pattern)),
            },
            None => Some(signed(pattern)),
        };
        if let Some(row) = row
            && !rows.contains(&row)
        {
            rows.push(row);
        }
    }
    for user in &system.users {
        if place(user) == Place::Elsewhere && user.home_state(filters) == HomeState::Excluded {
            let row = format!("- {}/**", user.home.trim_end_matches('/'));
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }

    let mut notes = Vec::new();
    notes.push(format!(
        "  /root    {}",
        match root_state {
            Some(HomeState::All) => "included",
            Some(HomeState::Hidden) => "not included; its hidden files kept as a filter",
            _ => "not included",
        }
    ));
    notes.push(format!(
        "  /home    {}",
        if include_home {
            "included (every home was \"everything\")".to_owned()
        } else if !all_everything {
            "not included".to_owned()
        } else if !unknown_entries.is_empty() {
            format!(
                "not included: /home also holds {}, which wasn't in snapshots",
                unknown_entries
                    .iter()
                    .map(|n| format!("/home/{n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!(
                "not included: {} would then apply inside home folders too",
                shadowed
                    .iter()
                    .map(|f| signed(f))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    ));
    for row in &kept_home_rows {
        notes.push(format!("  kept as a filter  {row}"));
    }
    let other = rows.len() - kept_home_rows.len();
    notes.push(format!("  filters  {other}"));
    (
        Config {
            backup_device_uuid: legacy.backup_device_uuid.clone(),
            include_root,
            include_home,
            filters: rows,
        },
        notes,
    )
}

/// Settings read from Timeshift's `timeshift.json`, in the old format, and notes on what was
/// taken and what wasn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub legacy: Legacy,
    /// One line each.
    pub notes: Vec<String>,
}

/// What Timeshift's `timeshift.json` holds for Apsis: its `backup_device_uuid` and its
/// `exclude` list as they are (home patterns included, in order). Read only; Timeshift's file
/// isn't changed. `devices` are only for the notes.
///
/// # Errors
///
/// [`Error::InvalidConfig`]: not a JSON object.
pub fn import_timeshift(text: &str, devices: &[Device]) -> Result<Import> {
    let invalid = |reason: &str| Error::InvalidConfig(format!("{TIMESHIFT_CONFIG}: {reason}"));
    let json: Json = serde_json::from_str(text).map_err(|_| invalid("not valid JSON"))?;
    let fields = json
        .as_object()
        .ok_or_else(|| invalid("not a JSON object"))?;
    let string = |key: &str| fields.get(key).and_then(Json::as_str).unwrap_or_default();
    let backup_device_uuid = string("backup_device_uuid").to_owned();
    let mut skipped = 0;
    let filters: Vec<String> = fields
        .get("exclude")
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pattern = item.as_str().filter(|p| validate_filter(p).is_ok());
                    if pattern.is_none() {
                        skipped += 1;
                    }
                    pattern.map(str::to_owned)
                })
                .fold(Vec::new(), |mut kept: Vec<String>, pattern| {
                    if !kept.contains(&pattern) {
                        kept.push(pattern);
                    }
                    kept
                })
        })
        .unwrap_or_default();

    let mut notes = vec![format!(
        "imported from {TIMESHIFT_CONFIG} (not saved yet; Save keeps it)"
    )];
    let device = devices
        .iter()
        .find(|d| !backup_device_uuid.is_empty() && d.uuid == backup_device_uuid);
    let short: String = backup_device_uuid.chars().take(8).collect();
    notes.push(match device {
        _ if backup_device_uuid.is_empty() => "  device   none set in Timeshift".to_owned(),
        Some(device) if !device.selectable() => format!(
            "  device   {short}… ({}, {}): can't be used, pick another",
            device.name,
            if device.fstype.is_empty() {
                "no filesystem"
            } else {
                &device.fstype
            }
        ),
        Some(device) => format!("  device   {short}… ({}, {})", device.name, device.fstype),
        None => format!("  device   {short}… (not connected)"),
    });
    let mut not_used = vec!["schedule and counts (Apsis doesn't schedule)"];
    if string("btrfs_mode") == "true" {
        not_used.push("btrfs mode (Apsis takes rsync snapshots; btrfs ones aren't listed)");
    }
    if skipped > 0 {
        not_used.push("filters Apsis can't use (blank or with control characters)");
    }
    notes.push(format!("  not used {}", not_used.join("; ")));
    Ok(Import {
        legacy: Legacy {
            backup_device_uuid,
            filters,
        },
        notes,
    })
}

/// Everything the settings view shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInfo {
    /// `config.toml` exactly as read; empty when there's none yet. Sent back with a write,
    /// which the helper refuses if the file changed since.
    pub text: String,
    /// The config in effect: the file's (converted if it's version 1), else the import, else
    /// the defaults.
    pub config: Config,
    pub devices: Vec<Device>,
    /// What a conversion or import did, while it isn't saved yet; empty otherwise.
    pub notes: Vec<String>,
}

impl ConfigInfo {
    /// The backup device the saved file names: `None` while there's no `config.toml`.
    #[must_use]
    pub fn saved_device(&self) -> Option<String> {
        if self.text.is_empty() {
            None
        } else {
            Config::read(&self.text).ok().map(|s| s.device().to_owned())
        }
    }

    /// Whether the config in effect isn't what the file says yet (a conversion or an import).
    #[must_use]
    pub fn unsaved(&self) -> bool {
        !self.notes.is_empty()
    }
}

/// The config in effect from what's on disk: `config.toml` (`config_text`) if there is one
/// (converted if it's version 1), else an import of `timeshift_json` if there is one
/// (converted), else the defaults. Returns the config and notes (empty unless something was
/// converted or imported).
///
/// # Errors
///
/// A `config.toml` that can't be read ([`Config::read`]). A `timeshift.json` that can't be
/// read is left alone (the defaults, with a note).
pub fn effective(
    config_text: Option<&str>,
    timeshift_json: Option<&str>,
    devices: &[Device],
    system: &System,
) -> Result<(Config, Vec<String>)> {
    if let Some(text) = config_text {
        return Ok(match Config::read(text)? {
            Stored::Current(config) => (config, Vec::new()),
            Stored::Legacy(legacy) => {
                let (config, mut notes) = convert(&legacy, system);
                notes.insert(
                    0,
                    "settings from Apsis 0.3 (not saved yet; Save keeps them)".to_owned(),
                );
                (config, notes)
            }
        });
    }
    match timeshift_json.map(|text| import_timeshift(text, devices)) {
        Some(Ok(import)) => {
            let (config, notes) = convert(&import.legacy, system);
            let mut all = import.notes;
            all.extend(notes);
            Ok((config, all))
        }
        Some(Err(error)) => Ok((
            Config::default(),
            vec![format!("not imported: {error}; pick a backup device")],
        )),
        None => Ok((Config::default(), Vec::new())),
    }
}
