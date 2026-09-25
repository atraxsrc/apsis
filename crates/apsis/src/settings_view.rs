// SPDX-License-Identifier: GPL-3.0-only

//! The settings view's model: Timeshift's settings as read, the edited copy, and which row is
//! selected. Drawing it is in `app.rs`.

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
}

impl Row {
    /// The section it belongs to, shown on the first row of each.
    pub fn section(self) -> Section {
        match self {
            Self::Device => Section::Device,
            Self::Mode | Self::BtrfsHome => Section::Mode,
            Self::Schedule(_) => Section::Schedule,
            Self::Home(_) => Section::Home,
            Self::Filter(_) | Self::AddFilter => Section::Filters,
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
}

/// The settings as read, and as edited.
#[derive(Debug, Clone)]
pub struct SettingsView {
    pub info: SettingsInfo,
    pub edited: Settings,
    /// Index into [`SettingsView::rows`].
    pub cursor: usize,
    /// Esc was pressed once with unsaved changes; the next Esc discards them.
    pub discard_armed: bool,
}

impl SettingsView {
    pub fn new(info: SettingsInfo) -> Self {
        let edited = info.config.settings();
        Self {
            info,
            edited,
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
    /// (as in Timeshift's Users tab).
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
        }
    }

    /// `+` / `-` on a schedule row.
    ///
    /// # Errors
    ///
    /// Not a schedule row.
    pub fn adjust_count(&mut self, up: bool) -> Result<(), String> {
        let Row::Schedule(level) = self.current() else {
            return Err("+ and - change a schedule's count".to_owned());
        };
        self.discard_armed = false;
        let count = &mut self.edited.counts[level.index()];
        *count = if up {
            count.saturating_add(1)
        } else {
            count.saturating_sub(1)
        }
        .clamp(MIN_COUNT, MAX_COUNT);
        Ok(())
    }

    /// A count typed at the prompt.
    ///
    /// # Errors
    ///
    /// Not a whole number from [`MIN_COUNT`] to [`MAX_COUNT`].
    pub fn set_count(&mut self, level: Level, typed: &str) -> Result<(), String> {
        match typed.trim().parse::<u32>() {
            Ok(count) if (MIN_COUNT..=MAX_COUNT).contains(&count) => {
                self.edited.counts[level.index()] = count;
                Ok(())
            }
            _ => Err(format!("keep {MIN_COUNT} to {MAX_COUNT} snapshots")),
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

    fn view() -> SettingsView {
        SettingsView::new(SettingsInfo {
            text: CONFIG.to_owned(),
            config: Config::parse(CONFIG).unwrap(),
            devices: parse_lsblk(LSBLK).unwrap(),
            users: vec![user("root", "/root"), user("user1", "/home/user1")],
            timeshift_gui_open: false,
        })
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
        assert_eq!(rows.last(), Some(&Row::AddFilter));

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
        assert!(view.set_count(Level::Daily, "0").is_err());
        assert!(view.set_count(Level::Daily, "1000").is_err());
        assert!(view.set_count(Level::Daily, "-3").is_err());
        view.set_count(Level::Daily, " 12 ").unwrap();
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
    fn validate_matches_the_helper() {
        let mut view = view();
        view.validate().unwrap();
        view.edited.counts[0] = 0;
        assert!(view.validate().unwrap_err().contains("1 to 999"));
    }
}
