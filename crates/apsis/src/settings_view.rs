// SPDX-License-Identifier: GPL-3.0-only

//! The settings view's model: Apsis's config as read (or imported), the edited copy, Apsis's
//! own per-user settings, and which row is selected. Drawing it is in `app.rs`.

use apsis_core::config::{self, Config, ConfigInfo};
use apsis_core::settings::{self, Device, HomeState, User};

/// One line of the settings view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// Space cycles through the devices that can hold snapshots.
    Device,
    /// A user's home folder (index into `ConfigInfo::users`).
    Home(usize),
    /// A filter (index into `Config::filters`). `x` removes it.
    Filter(usize),
    /// `+ add filter`: opens the prompt.
    AddFilter,
    /// Apsis's own: keep the last N snapshots (`p` prunes the rest, after `y`).
    KeepManual,
    /// Apsis's own: remind (tooltip, panel icon) when the last snapshot is older than N days.
    Remind,
}

/// What `+`, `-` and `e` change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    KeepManual,
    Remind,
}

/// Most snapshots "keep" can be set to.
pub const MAX_KEEP_MANUAL: u32 = 999;
/// What space turns "keep" on with.
pub const DEFAULT_KEEP_MANUAL: u32 = 10;
/// Longest reminder, in days.
pub const MAX_REMIND_DAYS: u32 = 365;
/// What space turns the reminder on with, and its default.
pub const DEFAULT_REMIND_DAYS: u32 = 7;

impl Row {
    /// The section it belongs to, shown on the first row of each.
    pub fn section(self) -> Section {
        match self {
            Self::Device => Section::Device,
            Self::Home(_) => Section::Home,
            Self::Filter(_) | Self::AddFilter => Section::Filters,
            Self::KeepManual | Self::Remind => Section::Apsis,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Device,
    Home,
    Filters,
    Apsis,
}

/// Apsis's own per-user settings, as the settings view shows and changes them. The app saves
/// them to cosmic-config whenever they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApsisChoice {
    /// Keep the last N snapshots; 0 = off.
    pub keep_manual: u32,
    /// Remind when the last snapshot is older than N days; 0 = off.
    pub remind_days: u32,
}

/// The config as read, and as edited.
#[derive(Debug, Clone)]
pub struct SettingsView {
    pub info: ConfigInfo,
    pub edited: Config,
    /// Apsis's own settings. Not part of [`SettingsView::dirty`]: they're saved as they change.
    pub backend: ApsisChoice,
    /// Index into [`SettingsView::rows`].
    pub cursor: usize,
    /// Esc was pressed once with unsaved changes; the next Esc discards them.
    pub discard_armed: bool,
}

impl SettingsView {
    pub fn new(info: ConfigInfo, backend: ApsisChoice) -> Self {
        let edited = info.config.clone();
        Self {
            info,
            edited,
            backend,
            cursor: 0,
            discard_armed: false,
        }
    }

    /// Whether the config came from Timeshift's settings and isn't saved yet.
    pub fn imported(&self) -> bool {
        !self.info.imported.is_empty()
    }

    /// Changed here, or imported and not saved yet: `w` writes it.
    pub fn dirty(&self) -> bool {
        match self.info.saved() {
            Some(saved) => self.edited != saved,
            None => self.edited != Config::default(),
        }
    }

    /// Drops the edits: back to the config as read (an import stays an import).
    pub fn discard(&mut self) {
        self.edited = self.info.config.clone();
        self.discard_armed = false;
    }

    /// Every row, top to bottom: the device, one per user's home, the filters, then Apsis's
    /// own.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Device];
        rows.extend((0..self.info.users.len()).map(Row::Home));
        rows.extend((0..self.edited.filters.len()).map(Row::Filter));
        rows.extend([Row::AddFilter, Row::KeepManual, Row::Remind]);
        rows
    }

    pub fn current(&self) -> Row {
        let rows = self.rows();
        rows[self.cursor.min(rows.len() - 1)]
    }

    /// Selects row `index`, or the last one.
    pub fn select(&mut self, index: usize) {
        self.cursor = index.min(self.rows().len() - 1);
        self.discard_armed = false;
    }

    /// The edited backup device, if it's connected.
    pub fn device(&self) -> Option<&Device> {
        let uuid = &self.edited.backup_device_uuid;
        self.info
            .devices
            .iter()
            .find(|d| !uuid.is_empty() && d.uuid == *uuid)
    }

    /// Devices space cycles through: those that can hold snapshots (unencrypted Linux
    /// filesystems).
    pub fn candidates(&self) -> Vec<&Device> {
        self.info
            .devices
            .iter()
            .filter(|d| d.selectable())
            .collect()
    }

    pub fn user(&self, index: usize) -> &User {
        &self.info.users[index]
    }

    /// Whether `filter` is one of the users' home folder patterns (shown on the home rows, not
    /// as a filter).
    pub fn is_home_pattern(&self, filter: &str) -> bool {
        self.info.users.iter().any(|u| u.owns(filter))
    }

    pub fn home_state(&self, index: usize) -> HomeState {
        self.user(index).home_state(&self.edited.filters)
    }

    /// Space or Enter on the current row (not [`Row::AddFilter`], which opens a prompt).
    ///
    /// # Errors
    ///
    /// Why the row can't change, for the activity pane.
    pub fn change(&mut self) -> Result<(), String> {
        self.discard_armed = false;
        match self.current() {
            Row::Device => {
                let candidates = self.candidates();
                let current = candidates
                    .iter()
                    .position(|d| d.uuid == self.edited.backup_device_uuid);
                let next = match current {
                    Some(i) => candidates.get((i + 1) % candidates.len()),
                    None => candidates.first(),
                };
                match next {
                    Some(device)
                        if Some(device.uuid.as_str()) != self.device().map(|d| d.uuid.as_str()) =>
                    {
                        self.edited.backup_device_uuid = device.uuid.clone();
                        Ok(())
                    }
                    _ => Err("no other device can hold snapshots".to_owned()),
                }
            }
            Row::Home(index) => {
                let user = self.user(index).clone();
                if user.encrypted_home {
                    return Err(format!(
                        "{}'s home is encrypted (ecryptfs): Apsis leaves its setting as it is",
                        user.name
                    ));
                }
                let next = user.home_state(&self.edited.filters).next();
                user.set_home_state(&mut self.edited.filters, next);
                Ok(())
            }
            Row::Filter(_) => Err("x removes a filter, a adds one".to_owned()),
            Row::AddFilter => Ok(()),
            Row::KeepManual => {
                let keep = &mut self.backend.keep_manual;
                *keep = if *keep == 0 { DEFAULT_KEEP_MANUAL } else { 0 };
                Ok(())
            }
            Row::Remind => {
                let days = &mut self.backend.remind_days;
                *days = if *days == 0 { DEFAULT_REMIND_DAYS } else { 0 };
                Ok(())
            }
        }
    }

    /// What `+`, `-` and `e` change on the current row, if anything.
    pub fn counted(&self) -> Option<Counted> {
        match self.current() {
            Row::KeepManual => Some(Counted::KeepManual),
            Row::Remind => Some(Counted::Remind),
            _ => None,
        }
    }

    /// The number `counted` stands for now.
    pub fn count_of(&self, counted: Counted) -> u32 {
        match counted {
            Counted::KeepManual => self.backend.keep_manual,
            Counted::Remind => self.backend.remind_days,
        }
    }

    /// Where `counted` lives, and its range.
    fn count_mut(&mut self, counted: Counted) -> (&mut u32, u32, u32) {
        match counted {
            Counted::KeepManual => (&mut self.backend.keep_manual, 0, MAX_KEEP_MANUAL),
            Counted::Remind => (&mut self.backend.remind_days, 0, MAX_REMIND_DAYS),
        }
    }

    /// `+` / `-` on keep or remind.
    ///
    /// # Errors
    ///
    /// Not a row with a number.
    pub fn adjust_count(&mut self, up: bool) -> Result<(), String> {
        let Some(counted) = self.counted() else {
            return Err("+ and - change a number".to_owned());
        };
        self.discard_armed = false;
        let (count, min, max) = self.count_mut(counted);
        *count = if up {
            count.saturating_add(1)
        } else {
            count.saturating_sub(1)
        }
        .clamp(min, max);
        Ok(())
    }

    /// A number typed at the prompt.
    ///
    /// # Errors
    ///
    /// Not a whole number in the range (`0` turns keep and remind off).
    pub fn set_count(&mut self, counted: Counted, typed: &str) -> Result<(), String> {
        let (count, min, max) = self.count_mut(counted);
        match typed.trim().parse::<u32>() {
            Ok(value) if (min..=max).contains(&value) => {
                *count = value;
                Ok(())
            }
            _ => Err(match counted {
                Counted::KeepManual => format!("keep 1 to {MAX_KEEP_MANUAL}, or 0 for off"),
                Counted::Remind => format!("1 to {MAX_REMIND_DAYS} days, or 0 for off"),
            }),
        }
    }

    /// Adds a filter at the end (as Timeshift does) and selects it.
    ///
    /// # Errors
    ///
    /// It fails `validate_filter`, or is there already.
    pub fn add_filter(&mut self, pattern: &str) -> Result<(), String> {
        settings::validate_filter(pattern).map_err(|e| e.to_string())?;
        if self.edited.filters.iter().any(|p| p == pattern) {
            return Err(format!("{pattern:?} is already a filter"));
        }
        self.edited.filters.push(pattern.to_owned());
        let index = self.edited.filters.len() - 1;
        if let Some(row) = self.rows().iter().position(|r| *r == Row::Filter(index)) {
            self.cursor = row;
        }
        Ok(())
    }

    /// `x` on a filter row.
    ///
    /// # Errors
    ///
    /// Not a filter row.
    pub fn remove_filter(&mut self) -> Result<(), String> {
        let Row::Filter(index) = self.current() else {
            return Err("select a filter to remove".to_owned());
        };
        self.discard_armed = false;
        self.edited.filters.remove(index);
        self.select(self.cursor);
        Ok(())
    }

    /// What the helper will check too, for a message before the password dialog.
    ///
    /// # Errors
    ///
    /// Why it can't be saved.
    pub fn validate(&self) -> Result<(), String> {
        config::validate(&self.edited, self.info.saved().as_ref(), &self.info.devices)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use apsis_core::config::import_timeshift;
    use apsis_core::settings::parse_lsblk;

    use super::*;

    const TIMESHIFT: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");

    fn user(name: &str, home: &str) -> User {
        User {
            name: name.to_owned(),
            home: home.to_owned(),
            encrypted_home: false,
        }
    }

    /// The defaults: keep off, remind after 7 days.
    const DEFAULT_BACKEND: ApsisChoice = ApsisChoice {
        keep_manual: 0,
        remind_days: DEFAULT_REMIND_DAYS,
    };

    fn users() -> Vec<User> {
        vec![user("root", "/root"), user("user1", "/home/user1")]
    }

    /// A saved config: the real Timeshift file's device and filters, written by Apsis.
    fn view() -> SettingsView {
        let config = import_timeshift(TIMESHIFT, &[], &[]).unwrap().config;
        SettingsView::new(
            ConfigInfo {
                text: config.to_text(),
                config,
                devices: parse_lsblk(LSBLK).unwrap(),
                users: users(),
                imported: Vec::new(),
            },
            DEFAULT_BACKEND,
        )
    }

    /// Before the first save: imported from Timeshift.
    fn imported_view() -> SettingsView {
        let import = import_timeshift(TIMESHIFT, &[], &users()).unwrap();
        SettingsView::new(
            ConfigInfo {
                text: String::new(),
                config: import.config,
                devices: parse_lsblk(LSBLK).unwrap(),
                users: users(),
                imported: import.notes,
            },
            DEFAULT_BACKEND,
        )
    }

    fn select(view: &mut SettingsView, row: Row) {
        let index = view.rows().iter().position(|r| *r == row).unwrap();
        view.select(index);
    }

    #[test]
    fn rows_are_device_homes_filters_then_apsiss_own() {
        let view = view();
        let rows = view.rows();
        assert_eq!(rows[..3], [Row::Device, Row::Home(0), Row::Home(1)]);
        assert_eq!(
            rows.iter().filter(|r| matches!(r, Row::Filter(_))).count(),
            4
        );
        assert_eq!(
            rows[rows.len() - 3..],
            [Row::AddFilter, Row::KeepManual, Row::Remind]
        );
        assert!(!view.dirty());
    }

    #[test]
    fn an_import_is_unsaved_until_written() {
        let mut view = imported_view();
        assert!(view.imported() && view.dirty());
        // Dropping edits keeps the import (still to be saved).
        select(&mut view, Row::Home(1));
        view.change().unwrap();
        view.discard();
        assert_eq!(view.edited, view.info.config);
        assert!(view.dirty());
        // Validated as a first save: the device must be connected and plain.
        view.validate().unwrap();
    }

    #[test]
    fn a_first_run_without_timeshift_starts_empty() {
        let mut view = SettingsView::new(
            ConfigInfo {
                text: String::new(),
                config: Config::default(),
                devices: parse_lsblk(LSBLK).unwrap(),
                users: users(),
                imported: Vec::new(),
            },
            DEFAULT_BACKEND,
        );
        assert!(!view.dirty());
        assert!(view.device().is_none());
        assert!(view.validate().unwrap_err().contains("choose"));
        view.change().unwrap();
        assert!(view.device().is_some() && view.dirty());
        view.validate().unwrap();
    }

    #[test]
    fn device_cycles_through_selectable_devices_only() {
        let mut view = view();
        let mut seen = Vec::new();
        for _ in 0..4 {
            view.change().unwrap();
            seen.push(view.device().unwrap().name.clone());
        }
        // In lsblk's order from the backup disk, wrapping round: never vfat or LUKS.
        assert_eq!(seen, ["sdd1", "nvme0n1p2", "sdb1", "sdd1"]);
    }

    #[test]
    fn home_rows_cycle_timeshifts_three_states() {
        let mut view = view();
        select(&mut view, Row::Home(1));
        assert_eq!(view.home_state(1), HomeState::All);
        view.change().unwrap();
        assert_eq!(view.home_state(1), HomeState::Excluded);
        view.change().unwrap();
        assert_eq!(view.home_state(1), HomeState::Hidden);
        view.change().unwrap();
        assert_eq!(view.home_state(1), HomeState::All);
        assert!(view.is_home_pattern("+ /home/user1/**"));
        assert!(!view.is_home_pattern("/var/lib/libvirt/**"));

        view.info.users[1].encrypted_home = true;
        assert!(view.change().is_err());
    }

    #[test]
    fn filters_are_added_at_the_end_and_removed() {
        let mut view = view();
        view.add_filter("*.mp3").unwrap();
        assert_eq!(view.current(), Row::Filter(4));
        assert!(view.add_filter("*.mp3").is_err());
        assert!(view.add_filter("+ ").is_err());
        view.remove_filter().unwrap();
        assert_eq!(view.edited.filters.len(), 4);
        assert!(!view.dirty());
        select(&mut view, Row::Device);
        assert!(view.remove_filter().is_err());
    }

    #[test]
    fn removing_the_last_filter_keeps_a_row_selected() {
        let mut view = view();
        view.edited.filters.truncate(1);
        select(&mut view, Row::Filter(0));
        view.remove_filter().unwrap();
        assert_eq!(view.current(), Row::AddFilter);
    }

    #[test]
    fn keep_and_remind_are_numbers_with_off() {
        let mut view = view();
        select(&mut view, Row::KeepManual);
        view.change().unwrap();
        assert_eq!(view.backend.keep_manual, DEFAULT_KEEP_MANUAL);
        view.adjust_count(false).unwrap();
        assert_eq!(view.backend.keep_manual, DEFAULT_KEEP_MANUAL - 1);
        view.set_count(Counted::KeepManual, "1").unwrap();
        view.adjust_count(false).unwrap();
        view.adjust_count(false).unwrap();
        assert_eq!(view.backend.keep_manual, 0);
        assert!(view.set_count(Counted::KeepManual, "1000").is_err());
        view.set_count(Counted::KeepManual, "999").unwrap();
        view.change().unwrap();
        assert_eq!(view.backend.keep_manual, 0);

        select(&mut view, Row::Remind);
        assert_eq!(view.counted(), Some(Counted::Remind));
        view.change().unwrap();
        assert_eq!(view.backend.remind_days, 0);
        view.change().unwrap();
        assert_eq!(view.backend.remind_days, DEFAULT_REMIND_DAYS);
        assert!(view.set_count(Counted::Remind, "366").is_err());
        view.set_count(Counted::Remind, "0").unwrap();
        // Apsis's own: the config isn't touched.
        assert!(!view.dirty());
        select(&mut view, Row::Device);
        assert!(view.adjust_count(true).is_err());
    }

    #[test]
    fn validate_matches_the_helper() {
        let mut view = view();
        view.validate().unwrap();
        view.edited.filters.push(view.edited.filters[0].clone());
        assert!(view.validate().unwrap_err().contains("twice"));
        // An encrypted device is refused before the password dialog.
        let mut view = self::view();
        view.edited.backup_device_uuid = "33333333-3333-3333-3333-333333333333".to_owned();
        assert!(view.validate().unwrap_err().contains("unencrypted"));
    }
}
