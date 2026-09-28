// SPDX-License-Identifier: GPL-3.0-only

//! The settings view's model: Timeshift's settings as read, the edited copy, Apsis's own
//! backend choice, and which row is selected. Drawing it is in `app.rs`.

use apsis_core::settings::{
    self, Device, HomeState, Level, MAX_COUNT, MIN_COUNT, Settings, SettingsInfo, User,
};

/// One line of the settings view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// Space cycles through the devices that can hold snapshots.
    Device,
    /// rsync or btrfs.
    Mode,
    /// btrfs mode: back up `@home` too.
    BtrfsHome,
    /// Space turns the level on or off, `+`/`-`/`e` change how many to keep.
    Schedule(Level),
    /// rsync mode: a user's home folder (index into `SettingsInfo::users`).
    Home(usize),
    /// A filter (index into `Settings::exclude`). `x` removes it.
    Filter(usize),
    /// `+ add filter`: opens the prompt.
    AddFilter,
    /// Apsis's own: Timeshift or the native rsync backend. Saved at once, not by `w`.
    Backend,
    /// Apsis's own, with the native backend: a create only shows what it would do.
    DryRun,
    /// Apsis's own: keep the last N manual snapshots (`p` prunes the rest, after `y`).
    KeepManual,
    /// Apsis's own: remind (tooltip, panel icon) when the last snapshot is older than N days.
    Remind,
}

/// What `+`, `-` and `e` change: a schedule level's count, or one of Apsis's own numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    Level(Level),
    KeepManual,
    Remind,
}

/// Most manual snapshots "keep" can be set to, as for Timeshift's counts.
pub const MAX_KEEP_MANUAL: u32 = MAX_COUNT;
/// What space turns "keep manual" on with.
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
            Self::Mode | Self::BtrfsHome => Section::Mode,
            Self::Schedule(_) => Section::Schedule,
            Self::Home(_) => Section::Home,
            Self::Filter(_) | Self::AddFilter => Section::Filters,
            Self::Backend | Self::DryRun | Self::KeepManual | Self::Remind => Section::Apsis,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Device,
    Mode,
    Schedule,
    Home,
    Filters,
    Apsis,
}

/// Apsis's own settings, as the settings view shows and changes them. The app saves them to
/// cosmic-config whenever they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApsisChoice {
    /// The native rsync backend instead of `timeshift`.
    pub native: bool,
    /// With the native backend: a create only shows what it would do.
    pub dry_run: bool,
    /// Keep the last N manual snapshots; 0 = off.
    pub keep_manual: u32,
    /// Remind when the last snapshot is older than N days; 0 = off.
    pub remind_days: u32,
}

/// The settings as read, and as edited.
#[derive(Debug, Clone)]
pub struct SettingsView {
    pub info: SettingsInfo,
    pub edited: Settings,
    /// Apsis's own setting. Not part of [`SettingsView::dirty`]: it's saved as it changes.
    pub backend: ApsisChoice,
    /// Index into [`SettingsView::rows`].
    pub cursor: usize,
    /// Esc was pressed once with unsaved changes; the next Esc discards them.
    pub discard_armed: bool,
}

impl SettingsView {
    pub fn new(info: SettingsInfo, backend: ApsisChoice) -> Self {
        let edited = info.config.settings();
        Self {
            info,
            edited,
            backend,
            cursor: 0,
            discard_armed: false,
        }
    }

    /// The settings as the file has them.
    pub fn saved(&self) -> Settings {
        self.info.config.settings()
    }

    pub fn dirty(&self) -> bool {
        self.edited != self.saved()
    }

    /// Every row, top to bottom. `@home` only in btrfs mode, home folders only in rsync mode
    /// (as in Timeshift's Users tab). Apsis's own rows last; dry run only with the native
    /// backend.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Device, Row::Mode];
        if self.edited.btrfs_mode {
            rows.push(Row::BtrfsHome);
        }
        rows.extend(Level::ALL.map(Row::Schedule));
        if !self.edited.btrfs_mode {
            rows.extend((0..self.info.users.len()).map(Row::Home));
        }
        rows.extend((0..self.edited.exclude.len()).map(Row::Filter));
        rows.push(Row::AddFilter);
        rows.push(Row::Backend);
        if self.backend.native {
            rows.push(Row::DryRun);
        }
        rows.extend([Row::KeepManual, Row::Remind]);
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

    pub fn btrfs_available(&self) -> bool {
        settings::btrfs_available(&self.info.devices)
    }

    /// The edited backup device, if it's connected.
    pub fn device(&self) -> Option<&Device> {
        let uuid = &self.edited.backup_device_uuid;
        self.info
            .devices
            .iter()
            .find(|d| !uuid.is_empty() && d.uuid == *uuid)
    }

    /// Devices space cycles through: those that can hold snapshots, btrfs ones in btrfs mode.
    pub fn candidates(&self) -> Vec<&Device> {
        self.info
            .devices
            .iter()
            .filter(|d| d.selectable() && (!self.edited.btrfs_mode || d.fstype == "btrfs"))
            .collect()
    }

    pub fn user(&self, index: usize) -> &User {
        &self.info.users[index]
    }

    pub fn home_state(&self, index: usize) -> HomeState {
        self.user(index).home_state(&self.edited.exclude)
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
            Row::Mode => {
                if !self.edited.btrfs_mode && !self.btrfs_available() {
                    return Err("btrfs mode needs a btrfs filesystem, and there is none".to_owned());
                }
                self.edited.btrfs_mode = !self.edited.btrfs_mode;
                Ok(())
            }
            Row::BtrfsHome => {
                self.edited.include_btrfs_home = !self.edited.include_btrfs_home;
                Ok(())
            }
            Row::Schedule(level) => {
                let on = &mut self.edited.schedule[level.index()];
                *on = !*on;
                Ok(())
            }
            Row::Home(index) => {
                let user = self.user(index).clone();
                if user.encrypted_home {
                    return Err(format!(
                        "{}'s home is encrypted: change it in Timeshift",
                        user.name
                    ));
                }
                let next = user.home_state(&self.edited.exclude).next();
                user.set_home_state(&mut self.edited.exclude, next);
                Ok(())
            }
            Row::Filter(_) => Err("x removes a filter, a adds one".to_owned()),
            Row::AddFilter => Ok(()),
            Row::Backend => {
                self.backend.native = !self.backend.native;
                Ok(())
            }
            Row::DryRun => {
                self.backend.dry_run = !self.backend.dry_run;
                Ok(())
            }
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
            Row::Schedule(level) => Some(Counted::Level(level)),
            Row::KeepManual => Some(Counted::KeepManual),
            Row::Remind => Some(Counted::Remind),
            _ => None,
        }
    }

    /// The number `counted` stands for now.
    pub fn count_of(&self, counted: Counted) -> u32 {
        match counted {
            Counted::Level(level) => self.edited.count(level),
            Counted::KeepManual => self.backend.keep_manual,
            Counted::Remind => self.backend.remind_days,
        }
    }

    /// Where `counted` lives, and its range.
    fn count_mut(&mut self, counted: Counted) -> (&mut u32, u32, u32) {
        match counted {
            Counted::Level(level) => (&mut self.edited.counts[level.index()], MIN_COUNT, MAX_COUNT),
            Counted::KeepManual => (&mut self.backend.keep_manual, 0, MAX_KEEP_MANUAL),
            Counted::Remind => (&mut self.backend.remind_days, 0, MAX_REMIND_DAYS),
        }
    }

    /// `+` / `-` on a schedule row, or on keep manual / remind.
    ///
    /// # Errors
    ///
    /// Not a row with a number.
    pub fn adjust_count(&mut self, up: bool) -> Result<(), String> {
        let Some(counted) = self.counted() else {
            return Err("+ and - change a count".to_owned());
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
    /// Not a whole number in the range (`0` turns keep manual and remind off).
    pub fn set_count(&mut self, counted: Counted, typed: &str) -> Result<(), String> {
        let (count, min, max) = self.count_mut(counted);
        match typed.trim().parse::<u32>() {
            Ok(value) if (min..=max).contains(&value) => {
                *count = value;
                Ok(())
            }
            _ => Err(match counted {
                Counted::Level(_) => format!("keep {MIN_COUNT} to {MAX_COUNT} snapshots"),
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
        if self.edited.exclude.iter().any(|p| p == pattern) {
            return Err(format!("{pattern:?} is already a filter"));
        }
        self.edited.exclude.push(pattern.to_owned());
        let index = self.edited.exclude.len() - 1;
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
        self.edited.exclude.remove(index);
        self.select(self.cursor);
        Ok(())
    }

    /// What the helper will check too, for a message before the password dialog.
    ///
    /// # Errors
    ///
    /// Why it can't be saved.
    pub fn validate(&self) -> Result<(), String> {
        settings::validate(&self.edited, &self.saved(), &self.info.devices)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use apsis_core::settings::{Config, parse_lsblk};

    use super::*;

    const CONFIG: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");
    const BTRFS_UUID: &str = "44444444-4444-4444-4444-444444444444";

    fn user(name: &str, home: &str) -> User {
        User {
            name: name.to_owned(),
            home: home.to_owned(),
            encrypted_home: false,
        }
    }

    /// The defaults: Timeshift's backend, dry run on, keep manual off, remind after 7 days.
    const DEFAULT_BACKEND: ApsisChoice = ApsisChoice {
        native: false,
        dry_run: true,
        keep_manual: 0,
        remind_days: DEFAULT_REMIND_DAYS,
    };

    fn view() -> SettingsView {
        SettingsView::new(
            SettingsInfo {
                text: CONFIG.to_owned(),
                config: Config::parse(CONFIG).unwrap(),
                devices: parse_lsblk(LSBLK).unwrap(),
                users: vec![user("root", "/root"), user("user1", "/home/user1")],
                timeshift_gui_open: false,
            },
            DEFAULT_BACKEND,
        )
    }

    fn select(view: &mut SettingsView, row: Row) {
        let index = view.rows().iter().position(|r| *r == row).unwrap();
        view.select(index);
    }

    #[test]
    fn rows_follow_the_mode() {
        let mut view = view();
        let rows = view.rows();
        assert_eq!(rows[..2], [Row::Device, Row::Mode]);
        assert!(rows.contains(&Row::Home(1)) && !rows.contains(&Row::BtrfsHome));
        assert_eq!(
            rows.iter().filter(|r| matches!(r, Row::Filter(_))).count(),
            4
        );
        assert_eq!(
            rows[rows.len() - 4..],
            [Row::AddFilter, Row::Backend, Row::KeepManual, Row::Remind]
        );

        select(&mut view, Row::Mode);
        view.change().unwrap();
        let rows = view.rows();
        assert!(rows.contains(&Row::BtrfsHome) && !rows.contains(&Row::Home(0)));
        assert!(view.dirty());
        view.change().unwrap();
        assert!(!view.dirty());
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
        // In btrfs mode only btrfs devices count: one, so nothing to switch to.
        view.edited.backup_device_uuid = BTRFS_UUID.to_owned();
        view.edited.btrfs_mode = true;
        select(&mut view, Row::Device);
        assert!(view.change().is_err());
    }

    #[test]
    fn btrfs_mode_needs_btrfs_somewhere() {
        let mut view = view();
        view.info.devices.retain(|d| d.fstype != "btrfs");
        select(&mut view, Row::Mode);
        assert!(view.change().is_err());
        assert!(!view.edited.btrfs_mode);
    }

    #[test]
    fn schedule_toggles_and_counts_stay_in_range() {
        let mut view = view();
        select(&mut view, Row::Schedule(Level::Daily));
        view.change().unwrap();
        assert!(view.edited.scheduled(Level::Daily));
        view.adjust_count(true).unwrap();
        assert_eq!(view.edited.count(Level::Daily), 6);
        view.edited.counts[Level::Daily.index()] = 1;
        view.adjust_count(false).unwrap();
        assert_eq!(view.edited.count(Level::Daily), 1);
        assert!(view.set_count(Counted::Level(Level::Daily), "0").is_err());
        assert!(
            view.set_count(Counted::Level(Level::Daily), "1000")
                .is_err()
        );
        assert!(view.set_count(Counted::Level(Level::Daily), "-3").is_err());
        view.set_count(Counted::Level(Level::Daily), " 12 ")
            .unwrap();
        assert_eq!(view.edited.count(Level::Daily), 12);
        select(&mut view, Row::Device);
        assert!(view.adjust_count(true).is_err());
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
        assert_eq!(view.edited.exclude.len(), 4);
        assert!(!view.dirty());
        select(&mut view, Row::Mode);
        assert!(view.remove_filter().is_err());
    }

    #[test]
    fn removing_the_last_filter_keeps_a_row_selected() {
        let mut view = view();
        view.edited.exclude.truncate(1);
        select(&mut view, Row::Filter(0));
        view.remove_filter().unwrap();
        assert_eq!(view.current(), Row::AddFilter);
    }

    #[test]
    fn keep_manual_and_remind_are_numbers_with_off() {
        let mut view = view();
        select(&mut view, Row::KeepManual);
        view.change().unwrap();
        assert_eq!(view.backend.keep_manual, DEFAULT_KEEP_MANUAL);
        view.adjust_count(false).unwrap();
        assert_eq!(view.backend.keep_manual, DEFAULT_KEEP_MANUAL - 1);
        view.set_count(Counted::KeepManual, "1").unwrap();
        // `-` from 1 is off; not below.
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
        assert_eq!(view.backend.remind_days, 0);
        // Apsis's own: Timeshift's file isn't touched.
        assert!(!view.dirty());
        // Schedule counts still refuse 0.
        assert!(view.set_count(Counted::Level(Level::Daily), "0").is_err());
    }

    #[test]
    fn backend_rows_are_apsis_own_and_not_unsaved_timeshift_changes() {
        let mut view = view();
        assert!(!view.rows().contains(&Row::DryRun));
        select(&mut view, Row::Backend);
        view.change().unwrap();
        assert_eq!(
            view.backend,
            ApsisChoice {
                native: true,
                ..DEFAULT_BACKEND
            }
        );
        // Dry run shows up once the native backend is on, before keep manual and remind.
        let rows = view.rows();
        assert_eq!(
            rows[rows.len() - 3..],
            [Row::DryRun, Row::KeepManual, Row::Remind]
        );
        select(&mut view, Row::DryRun);
        view.change().unwrap();
        assert!(!view.backend.dry_run);
        // Timeshift's file isn't touched by these.
        assert!(!view.dirty());
    }

    #[test]
    fn validate_matches_the_helper() {
        let mut view = view();
        view.validate().unwrap();
        view.edited.counts[0] = 0;
        assert!(view.validate().unwrap_err().contains("1 to 999"));
    }
}
