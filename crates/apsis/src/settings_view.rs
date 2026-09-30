// SPDX-License-Identifier: GPL-3.0-only

//! The settings' model: Apsis's config as read (or converted, or imported), the edited copy,
//! Apsis's own per-user settings, and which row is selected. Drawing it is in `app.rs`.

// The tabs, the file chooser and the Create dialog use the rest (0.4.0, slice 3).
#![allow(
    dead_code,
    reason = "used by the settings tabs, which come in the next slice"
)]

use apsis_core::config::{self, Config, ConfigInfo, validate_signed_filter};
use apsis_core::settings::Device;

/// One line of the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// Space cycles through the devices that can hold snapshots.
    Device,
    /// `/root` in snapshots or not.
    IncludeRoot,
    /// `/home` in snapshots or not.
    IncludeHome,
    /// A filter (index into `Config::filters`). Space flips `+`/`-`.
    Filter(usize),
    /// Adds a typed filter.
    AddFilter,
    /// Apsis's own: remind (tooltip, panel icon) when the last snapshot is older than N days.
    Remind,
    /// Apsis's own: `12h · 62%` beside the panel icon.
    PanelLabel,
}

/// What `+`, `-` and `e` change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    Remind,
}

/// Longest reminder, in days.
pub const MAX_REMIND_DAYS: u32 = 365;
/// What space turns the reminder on with, and its default.
pub const DEFAULT_REMIND_DAYS: u32 = 7;

impl Row {
    /// The section (settings tab) it belongs to.
    pub fn section(self) -> Section {
        match self {
            Self::Device => Section::Location,
            Self::IncludeRoot | Self::IncludeHome => Section::Include,
            Self::Filter(_) | Self::AddFilter => Section::Filters,
            Self::Remind | Self::PanelLabel => Section::Misc,
        }
    }
}

/// The settings' tabs, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Location,
    Include,
    Filters,
    Misc,
}

impl Section {
    pub const ALL: [Self; 4] = [Self::Location, Self::Include, Self::Filters, Self::Misc];
}

/// Apsis's own per-user settings, as the settings show and change them. The app saves them
/// to cosmic-config whenever they change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApsisChoice {
    /// Remind when the last snapshot is older than N days; 0 = off.
    pub remind_days: u32,
    /// Show the age and disk use beside the panel icon.
    pub show_label: bool,
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

/// rsync's wildcard characters, escaped with `\` in a picked path.
const WILDCARDS: [char; 4] = ['\\', '*', '?', '['];

/// The filter for a path picked in the file chooser: `<path>/***` for a folder (the folder and
/// everything in it, as Timeshift's filter editor adds one), `<path>` for a file. rsync's
/// wildcard characters in the path are escaped. The sign is `-`, except `+` for a path inside
/// `/root` or `/home` while that isn't included (it was picked to be kept).
///
/// # Errors
///
/// Not an absolute path, or a document portal path (`/run/user/<uid>/doc/...`), which isn't
/// where the file really is.
pub fn picked_filter(path: &str, folder: bool, config: &Config) -> Result<String, String> {
    if !path.starts_with('/') {
        return Err(format!("{path:?} isn't a full path"));
    }
    if path.starts_with("/run/user/") && path.split('/').nth(4) == Some("doc") {
        return Err("that's a copy the file chooser made; pick the real file".to_owned());
    }
    let path = path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let escaped: String = path
        .chars()
        .flat_map(|c| {
            let escape = WILDCARDS.contains(&c).then_some('\\');
            escape.into_iter().chain(std::iter::once(c))
        })
        .collect();
    let body = if folder {
        format!("{}/***", escaped.trim_end_matches('/'))
    } else {
        escaped
    };
    let under = |top: &str| path == top || path.starts_with(&format!("{top}/"));
    let keep = (under("/root") && !config.include_root) || (under("/home") && !config.include_home);
    Ok(format!("{} {body}", if keep { '+' } else { '-' }))
}

/// A filter typed by hand: `+ x` and `- x` as they are, anything else is an exclude.
pub fn typed_filter(text: &str) -> String {
    let text = text.trim_start();
    if text.starts_with("+ ") || text.starts_with("- ") {
        text.to_owned()
    } else {
        format!("- {text}")
    }
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

    /// Whether the config was converted from the old format or imported from Timeshift's
    /// settings, and isn't saved yet.
    pub fn imported(&self) -> bool {
        self.info.unsaved()
    }

    /// Changed here, or converted or imported and not saved yet: Save writes it.
    pub fn dirty(&self) -> bool {
        self.imported() || self.info.text.is_empty() || self.edited != self.info.config
    }

    /// Drops the edits: back to the config as read (an import stays an import).
    pub fn discard(&mut self) {
        self.edited = self.info.config.clone();
        self.discard_armed = false;
    }

    /// Every row, top to bottom: the device, the two includes, the filters, then Apsis's own.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Device, Row::IncludeRoot, Row::IncludeHome];
        rows.extend((0..self.edited.filters.len()).map(Row::Filter));
        rows.extend([Row::AddFilter, Row::Remind, Row::PanelLabel]);
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

    /// Selects `row`, if it's there.
    pub fn select_row(&mut self, row: Row) {
        if let Some(index) = self.rows().iter().position(|r| *r == row) {
            self.select(index);
        }
    }

    /// The edited backup device, if it's connected.
    pub fn device(&self) -> Option<&Device> {
        let uuid = &self.edited.backup_device_uuid;
        self.info
            .devices
            .iter()
            .find(|d| !uuid.is_empty() && d.uuid == *uuid)
    }

    /// Devices that can hold snapshots (unencrypted Linux filesystems).
    pub fn candidates(&self) -> Vec<&Device> {
        self.info
            .devices
            .iter()
            .filter(|d| d.selectable())
            .collect()
    }

    /// Picks the device with filesystem UUID `uuid` (one of [`SettingsView::candidates`]).
    ///
    /// # Errors
    ///
    /// It can't hold snapshots, or isn't connected.
    pub fn pick_device(&mut self, uuid: &str) -> Result<(), String> {
        if !self.candidates().iter().any(|d| d.uuid == uuid) {
            return Err("that disk can't hold snapshots".to_owned());
        }
        self.discard_armed = false;
        self.edited.backup_device_uuid = uuid.to_owned();
        Ok(())
    }

    /// Turns `/root` or `/home` on or off.
    pub fn set_include(&mut self, row: Row, on: bool) {
        self.discard_armed = false;
        match row {
            Row::IncludeRoot => self.edited.include_root = on,
            Row::IncludeHome => self.edited.include_home = on,
            _ => {}
        }
    }

    /// Makes filter `index` an include (`+`) or an exclude (`-`).
    pub fn set_sign(&mut self, index: usize, include: bool) {
        let Some(filter) = self.edited.filters.get(index) else {
            return;
        };
        let body = filter
            .strip_prefix("+ ")
            .or_else(|| filter.strip_prefix("- "))
            .unwrap_or(filter);
        let changed = format!("{} {body}", if include { '+' } else { '-' });
        self.discard_armed = false;
        self.edited.filters[index] = changed;
    }

    /// Whether filter `index` is an include.
    pub fn is_include(&self, index: usize) -> bool {
        self.edited
            .filters
            .get(index)
            .is_some_and(|f| f.starts_with("+ "))
    }

    /// Space or Enter on the current row (not [`Row::AddFilter`], which opens a prompt).
    ///
    /// # Errors
    ///
    /// Why the row can't change.
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
            Row::IncludeRoot => {
                self.edited.include_root = !self.edited.include_root;
                Ok(())
            }
            Row::IncludeHome => {
                self.edited.include_home = !self.edited.include_home;
                Ok(())
            }
            Row::Filter(index) => {
                let include = !self.is_include(index);
                self.set_sign(index, include);
                Ok(())
            }
            Row::AddFilter => Ok(()),
            Row::Remind => {
                let days = &mut self.backend.remind_days;
                *days = if *days == 0 { DEFAULT_REMIND_DAYS } else { 0 };
                Ok(())
            }
            Row::PanelLabel => {
                self.backend.show_label = !self.backend.show_label;
                Ok(())
            }
        }
    }

    /// What `+`, `-` and `e` change on the current row, if anything.
    pub fn counted(&self) -> Option<Counted> {
        match self.current() {
            Row::Remind => Some(Counted::Remind),
            _ => None,
        }
    }

    /// The number `counted` stands for now.
    pub fn count_of(&self, counted: Counted) -> u32 {
        match counted {
            Counted::Remind => self.backend.remind_days,
        }
    }

    /// Where `counted` lives, and its range.
    fn count_mut(&mut self, counted: Counted) -> (&mut u32, u32, u32) {
        match counted {
            Counted::Remind => (&mut self.backend.remind_days, 0, MAX_REMIND_DAYS),
        }
    }

    /// `+` / `-` on the current row: a filter becomes an include or an exclude; the reminder
    /// gets a day more or less.
    ///
    /// # Errors
    ///
    /// Not a row `+` and `-` change.
    pub fn adjust_count(&mut self, up: bool) -> Result<(), String> {
        if let Row::Filter(index) = self.current() {
            self.set_sign(index, up);
            return Ok(());
        }
        let Some(counted) = self.counted() else {
            return Err("+ and - change a filter or a number".to_owned());
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
    /// Not a whole number in the range (`0` turns the reminder off).
    pub fn set_count(&mut self, counted: Counted, typed: &str) -> Result<(), String> {
        let (count, min, max) = self.count_mut(counted);
        match typed.trim().parse::<u32>() {
            Ok(value) if (min..=max).contains(&value) => {
                *count = value;
                Ok(())
            }
            _ => Err(match counted {
                Counted::Remind => format!("1 to {MAX_REMIND_DAYS} days, or 0 for off"),
            }),
        }
    }

    /// Adds `filter` (`+ x` or `- x`) at the top, where it wins over the older, broader ones,
    /// and selects it.
    ///
    /// # Errors
    ///
    /// It isn't a valid filter, or it's there already.
    pub fn add_filter(&mut self, filter: &str) -> Result<(), String> {
        validate_signed_filter(filter).map_err(|e| e.to_string())?;
        if self.edited.filters.iter().any(|p| p == filter) {
            return Err(format!("{filter:?} is already a filter"));
        }
        self.discard_armed = false;
        self.edited.filters.insert(0, filter.to_owned());
        self.select_row(Row::Filter(0));
        Ok(())
    }

    /// Removes the selected filter.
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
        if self.edited.filters.is_empty() {
            self.select_row(Row::AddFilter);
        } else {
            self.select_row(Row::Filter(index.min(self.edited.filters.len() - 1)));
        }
        Ok(())
    }

    /// Moves the selected filter one up (`up`) or down, and keeps it selected.
    ///
    /// # Errors
    ///
    /// Not a filter row, or it's already at that end.
    pub fn move_filter(&mut self, up: bool) -> Result<(), String> {
        let Row::Filter(index) = self.current() else {
            return Err("select a filter to move".to_owned());
        };
        let to = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|&i| i < self.edited.filters.len())
        };
        let Some(to) = to else {
            return Err("it's already at that end".to_owned());
        };
        self.discard_armed = false;
        self.edited.filters.swap(index, to);
        self.select_row(Row::Filter(to));
        Ok(())
    }

    /// What the helper will check too, for a message before the password dialog.
    ///
    /// # Errors
    ///
    /// Why it can't be saved.
    pub fn validate(&self) -> Result<(), String> {
        let saved = self.info.saved_device();
        config::validate(&self.edited, saved.as_deref(), &self.info.devices)
            .map_err(|e| e.to_string())
    }

    /// `Includes the system, /root and 3 filters.`: what a snapshot takes, for the Create
    /// dialog.
    pub fn includes_line(config: &Config) -> String {
        let mut parts = vec!["the system".to_owned()];
        if config.include_root {
            parts.push("/root".to_owned());
        }
        if config.include_home {
            parts.push("/home".to_owned());
        }
        match config.filters.len() {
            0 => {}
            1 => parts.push("1 filter".to_owned()),
            n => parts.push(format!("{n} filters")),
        }
        let text = match parts.as_slice() {
            [only] => format!("{only} only"),
            [first @ .., last] => format!("{} and {last}", first.join(", ")),
            [] => String::new(),
        };
        format!("Includes {text}.")
    }
}

#[cfg(test)]
mod tests {
    use apsis_core::config::{System, convert, import_timeshift};
    use apsis_core::settings::parse_lsblk;

    use super::*;

    const TIMESHIFT: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");

    /// The defaults: remind after 7 days, no panel label.
    const DEFAULT_BACKEND: ApsisChoice = ApsisChoice {
        remind_days: DEFAULT_REMIND_DAYS,
        show_label: false,
    };

    /// The real Timeshift settings, converted (with no users known: every pattern a filter).
    fn converted() -> Config {
        let import = import_timeshift(TIMESHIFT, &[]).unwrap();
        convert(&import.legacy, &System::default()).0
    }

    /// A saved config: the real file's device and filters, written by Apsis.
    fn view() -> SettingsView {
        let config = converted();
        SettingsView::new(
            ConfigInfo {
                text: config.to_text(),
                config,
                devices: parse_lsblk(LSBLK).unwrap(),
                notes: Vec::new(),
            },
            DEFAULT_BACKEND,
        )
    }

    /// Before the first save: imported from Timeshift.
    fn imported_view() -> SettingsView {
        SettingsView::new(
            ConfigInfo {
                text: String::new(),
                config: converted(),
                devices: parse_lsblk(LSBLK).unwrap(),
                notes: vec!["imported".to_owned()],
            },
            DEFAULT_BACKEND,
        )
    }

    #[test]
    fn rows_are_device_includes_filters_then_apsiss_own() {
        let view = view();
        let rows = view.rows();
        assert_eq!(rows[..3], [Row::Device, Row::IncludeRoot, Row::IncludeHome]);
        assert_eq!(
            rows.iter().filter(|r| matches!(r, Row::Filter(_))).count(),
            4
        );
        assert_eq!(
            rows[rows.len() - 3..],
            [Row::AddFilter, Row::Remind, Row::PanelLabel]
        );
        assert!(!view.dirty());
        assert_eq!(Row::IncludeHome.section(), Section::Include);
        assert_eq!(Row::Remind.section(), Section::Misc);
    }

    #[test]
    fn an_import_is_unsaved_until_written() {
        let mut view = imported_view();
        assert!(view.imported() && view.dirty());
        view.discard();
        assert!(view.dirty(), "an import stays one");
    }

    #[test]
    fn includes_and_signs_toggle() {
        let mut view = view();
        view.select_row(Row::IncludeHome);
        let before = view.edited.include_home;
        view.change().unwrap();
        assert_eq!(view.edited.include_home, !before);
        assert!(view.dirty());
        view.set_include(Row::IncludeHome, before);
        assert!(!view.dirty());

        view.select_row(Row::Filter(2));
        assert_eq!(view.edited.filters[2], "- /var/lib/libvirt/**");
        view.change().unwrap();
        assert_eq!(view.edited.filters[2], "+ /var/lib/libvirt/**");
        view.adjust_count(false).unwrap();
        assert_eq!(view.edited.filters[2], "- /var/lib/libvirt/**");
    }

    #[test]
    fn new_filters_go_on_top_and_move_up_and_down() {
        let mut view = view();
        view.add_filter("- *.iso").unwrap();
        assert_eq!(view.edited.filters[0], "- *.iso");
        assert_eq!(view.current(), Row::Filter(0));
        assert!(view.add_filter("- *.iso").unwrap_err().contains("already"));
        assert!(
            view.add_filter("*.mp3")
                .unwrap_err()
                .contains("starts with")
        );
        assert!(view.move_filter(true).is_err(), "already at the top");
        view.move_filter(false).unwrap();
        assert_eq!(view.edited.filters[1], "- *.iso");
        assert_eq!(view.current(), Row::Filter(1));
        let last = view.edited.filters.len() - 1;
        view.select_row(Row::Filter(last));
        assert!(view.move_filter(false).is_err(), "already at the bottom");
        view.remove_filter().unwrap();
        assert_eq!(view.current(), Row::Filter(last - 1));
        view.select_row(Row::Device);
        assert!(view.remove_filter().is_err());
        assert!(view.move_filter(true).is_err());
    }

    #[test]
    fn removing_the_last_filter_keeps_a_row_selected() {
        let mut view = view();
        view.edited.filters.truncate(1);
        view.select_row(Row::Filter(0));
        view.remove_filter().unwrap();
        assert_eq!(view.current(), Row::AddFilter);
    }

    #[test]
    fn picked_paths_become_filters() {
        let config = Config::default();
        assert_eq!(
            picked_filter("/var/lib/libvirt", true, &config).unwrap(),
            "- /var/lib/libvirt/***"
        );
        assert_eq!(
            picked_filter("/etc/big.iso", false, &config).unwrap(),
            "- /etc/big.iso"
        );
        // /home is out by default: a pick inside it is to be kept.
        assert_eq!(
            picked_filter("/home/user1/Projects/", true, &config).unwrap(),
            "+ /home/user1/Projects/***"
        );
        // /root is in by default: a pick inside it is to be left out.
        assert_eq!(
            picked_filter("/root/Downloads", true, &config).unwrap(),
            "- /root/Downloads/***"
        );
        // rsync's wildcards are escaped.
        assert_eq!(
            picked_filter("/srv/a[1]*b?", false, &config).unwrap(),
            "- /srv/a\\[1]\\*b\\?"
        );
        assert!(picked_filter("relative/x", false, &config).is_err());
        assert!(picked_filter("/run/user/1000/doc/abcd/file", false, &config).is_err());
        assert!(picked_filter("/run/user/1000/other", false, &config).is_ok());
    }

    #[test]
    fn typed_filters_default_to_exclude() {
        assert_eq!(typed_filter("*.iso"), "- *.iso");
        assert_eq!(typed_filter("  + /opt/keep"), "+ /opt/keep");
        assert_eq!(typed_filter("- /x"), "- /x");
    }

    #[test]
    fn the_includes_line_names_what_a_snapshot_takes() {
        let mut config = Config::default();
        assert_eq!(
            SettingsView::includes_line(&config),
            "Includes the system and /root."
        );
        config.include_root = false;
        assert_eq!(
            SettingsView::includes_line(&config),
            "Includes the system only."
        );
        config.include_root = true;
        config.include_home = true;
        config.filters = vec!["- a".to_owned(), "- b".to_owned(), "- c".to_owned()];
        assert_eq!(
            SettingsView::includes_line(&config),
            "Includes the system, /root, /home and 3 filters."
        );
        config.filters.truncate(1);
        config.include_root = false;
        assert_eq!(
            SettingsView::includes_line(&config),
            "Includes the system, /home and 1 filter."
        );
    }

    #[test]
    fn the_panel_label_is_a_toggle_off_by_default() {
        let mut view = view();
        assert!(!view.backend.show_label);
        view.select_row(Row::PanelLabel);
        assert_eq!(view.counted(), None);
        view.change().unwrap();
        assert!(view.backend.show_label);
        view.change().unwrap();
        assert!(!view.backend.show_label);
        // Apsis's own: the config isn't touched.
        assert!(!view.dirty());
    }

    #[test]
    fn remind_is_a_number_with_off() {
        let mut view = view();
        view.select_row(Row::Remind);
        assert_eq!(view.counted(), Some(Counted::Remind));
        view.change().unwrap();
        assert_eq!(view.backend.remind_days, 0);
        view.change().unwrap();
        assert_eq!(view.backend.remind_days, DEFAULT_REMIND_DAYS);
        view.adjust_count(false).unwrap();
        assert_eq!(view.backend.remind_days, DEFAULT_REMIND_DAYS - 1);
        assert!(view.set_count(Counted::Remind, "366").is_err());
        view.set_count(Counted::Remind, "0").unwrap();
        // Apsis's own: the config isn't touched.
        assert!(!view.dirty());
        view.select_row(Row::Device);
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
        view.pick_device("33333333-3333-3333-3333-333333333333")
            .unwrap_err();
        view.edited.backup_device_uuid = "33333333-3333-3333-3333-333333333333".to_owned();
        assert!(view.validate().unwrap_err().contains("unencrypted"));
    }
}
