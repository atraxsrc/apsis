// SPDX-License-Identifier: GPL-3.0-only

//! Apsis's config, `/etc/apsis/config.toml`: the backup device and the rsync filters. Only
//! `apsis-helper` writes it.
//!
//! ```toml
//! # Written by apsis-helper; change it in Apsis's settings.
//! version = 1
//! backup_device_uuid = "8cecb045-975d-49d0-bd57-1ec9f6eb77b5"
//! filters = [
//!     "+ /home/user1/**",
//!     "/var/lib/libvirt/**",
//! ]
//! ```
//!
//! `filters` is one ordered list, in Timeshift's format: rsync takes the first pattern that
//! matches, and the home folder patterns (`<home>/**` nothing, `+ <home>/.**` hidden files only,
//! `+ <home>/**` everything; [`crate::settings::User`]) sit among the others. Keeping the order
//! keeps what a snapshot holds exactly as it was, so an imported config goes on hard-linking
//! against the snapshots taken before.
//!
//! **Import.** While there is no `config.toml`, a config is read from Timeshift's
//! `/etc/timeshift/timeshift.json` if there is one ([`import_timeshift`]): its backup device and
//! filter list, with notes on what was taken and what wasn't. It's only saved when the user
//! writes the settings; after that, `timeshift.json` isn't read again.

use serde_json::Value as Json;

use crate::error::{Error, Result};
use crate::settings::{Device, HomeState, User, validate_filter};

/// Where the config lives.
pub const CONFIG_PATH: &str = "/etc/apsis/config.toml";
/// The one backup of the previous config the helper keeps.
pub const BACKUP_PATH: &str = "/etc/apsis/config.toml.bak";
/// Timeshift's settings, read once for the import.
pub const TIMESHIFT_CONFIG: &str = "/etc/timeshift/timeshift.json";
/// The format this code writes and reads.
const VERSION: i64 = 1;

/// What a snapshot is taken to, and with which filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Filesystem UUID of the backup device; empty when none is chosen.
    pub backup_device_uuid: String,
    /// rsync filters, in order (see the module docs).
    pub filters: Vec<String>,
}

impl Config {
    /// Reads `config.toml`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfig`]: not TOML, another version, or a field of the wrong type.
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = |reason: String| Error::InvalidConfig(format!("{CONFIG_PATH}: {reason}"));
        let value: toml::Value = text
            .parse()
            .map_err(|e| invalid(format!("not valid TOML: {e}")))?;
        let table = value
            .as_table()
            .ok_or_else(|| invalid("not a table".to_owned()))?;
        match table.get("version").and_then(toml::Value::as_integer) {
            Some(VERSION) => {}
            other => {
                return Err(invalid(format!(
                    "version {} (this Apsis reads {VERSION})",
                    other.map_or_else(|| "missing".to_owned(), |v| v.to_string())
                )));
            }
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
        Ok(Self {
            backup_device_uuid,
            filters,
        })
    }

    /// The file's text: a comment, the version, the device, one filter per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let quote = |text: &str| toml::Value::String(text.to_owned()).to_string();
        let mut out = String::from(
            "# Written by apsis-helper; change it in Apsis's settings.\n\
             # filters: rsync patterns, first match wins; \"+ \" in front includes.\n",
        );
        out.push_str(&format!("version = {VERSION}\n"));
        out.push_str(&format!(
            "backup_device_uuid = {}\n",
            quote(&self.backup_device_uuid)
        ));
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

/// Checks a config before it's written (the helper checks again).
///
/// - A backup device that differs from the saved one (`old`; `None` when there's no config
///   file yet) must be connected, and a plain unencrypted Linux filesystem
///   ([`Device::selectable`]). An unchanged one may be unplugged.
/// - Filters pass [`validate_filter`] and aren't repeated.
///
/// # Errors
///
/// [`Error::InvalidSettings`] with the first problem found.
pub fn validate(new: &Config, old: Option<&Config>, devices: &[Device]) -> Result<()> {
    let invalid = |reason: String| Err(Error::InvalidSettings(reason));
    let changed = old.is_none_or(|old| old.backup_device_uuid != new.backup_device_uuid);
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
    for (i, pattern) in new.filters.iter().enumerate() {
        validate_filter(pattern)?;
        if new.filters[..i].contains(pattern) {
            return invalid(format!("filter {pattern:?} is there twice"));
        }
    }
    Ok(())
}

/// A config read from Timeshift's settings, and what the user should know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub config: Config,
    /// One line each: what was taken, what wasn't.
    pub notes: Vec<String>,
}

fn home_state_name(state: HomeState) -> &'static str {
    match state {
        HomeState::Excluded => "excluded",
        HomeState::Hidden => "hidden files only",
        HomeState::All => "everything",
    }
}

/// The config Timeshift's `timeshift.json` stands for: its `backup_device_uuid` and its
/// `exclude` list as they are (home patterns included, in order). Read only; Timeshift's file
/// isn't changed. `devices` and `users` are only for the notes.
///
/// # Errors
///
/// [`Error::InvalidConfig`]: not a JSON object.
pub fn import_timeshift(text: &str, devices: &[Device], users: &[User]) -> Result<Import> {
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
        "imported from {TIMESHIFT_CONFIG} (not saved yet; w saves)"
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
    let homes: Vec<String> = users
        .iter()
        .map(|user| {
            format!(
                "{}: {}",
                user.name,
                home_state_name(user.home_state(&filters))
            )
        })
        .collect();
    if !homes.is_empty() {
        notes.push(format!("  home     {}", homes.join(", ")));
    }
    let other = filters
        .iter()
        .filter(|f| !users.iter().any(|u| u.owns(f)))
        .count();
    notes.push(format!("  filters  {other}"));
    let mut not_used = vec!["schedule and counts (Apsis doesn't schedule)"];
    if string("btrfs_mode") == "true" {
        not_used.push("btrfs mode (Apsis takes rsync snapshots; btrfs ones aren't listed)");
    }
    if skipped > 0 {
        not_used.push("filters Apsis can't use (blank or with control characters)");
    }
    notes.push(format!("  not used {}", not_used.join("; ")));
    Ok(Import {
        config: Config {
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
    /// The config in effect: the file's, else the import, else empty.
    pub config: Config,
    pub devices: Vec<Device>,
    pub users: Vec<User>,
    /// The import's notes when `config` came from Timeshift's settings; empty otherwise.
    pub imported: Vec<String>,
}

impl ConfigInfo {
    /// The config as saved: `None` while there's no `config.toml`.
    #[must_use]
    pub fn saved(&self) -> Option<Config> {
        if self.text.is_empty() {
            None
        } else {
            Config::parse(&self.text).ok()
        }
    }
}

/// The config in effect from what's on disk: `config.toml` (`config_text`) if there is one,
/// else an import of `timeshift_json` if there is one, else empty. Returns the config and the
/// import notes (empty unless imported).
///
/// # Errors
///
/// A `config.toml` that can't be read ([`Config::parse`]). A `timeshift.json` that can't be
/// read is left alone (an empty config, with a note).
pub fn effective(
    config_text: Option<&str>,
    timeshift_json: Option<&str>,
    devices: &[Device],
    users: &[User],
) -> Result<(Config, Vec<String>)> {
    if let Some(text) = config_text {
        return Ok((Config::parse(text)?, Vec::new()));
    }
    match timeshift_json.map(|text| import_timeshift(text, devices, users)) {
        Some(Ok(import)) => Ok((import.config, import.notes)),
        Some(Err(error)) => Ok((
            Config::default(),
            vec![format!("not imported: {error}; pick a backup device")],
        )),
        None => Ok((Config::default(), Vec::new())),
    }
}
