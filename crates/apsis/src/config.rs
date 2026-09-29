// SPDX-License-Identifier: GPL-3.0-only

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};

/// Apsis's own per-user settings, in cosmic-config. The backup device and filters are in
/// `/etc/apsis/config.toml`, which only the helper writes. (Older versions also stored
/// `native_backend` and `native_dry_run` here; they're no longer read.)
#[derive(Debug, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    demo: String,
    /// Keep the last N snapshots without a comment; `p` shows what goes and deletes after `y`.
    /// 0 = off, the default.
    pub keep_manual: u32,
    /// Tooltip and panel icon remind when the last snapshot is older than this many days.
    /// 0 = off.
    pub remind_days: u32,
    /// Show `12h · 62%` beside the panel icon (horizontal panels only). Off by default.
    pub show_label: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            demo: String::new(),
            keep_manual: 0,
            remind_days: crate::settings_view::DEFAULT_REMIND_DAYS,
            show_label: false,
        }
    }
}

impl Config {
    /// Apsis's own settings, for the settings view.
    pub fn backend(&self) -> crate::settings_view::ApsisChoice {
        crate::settings_view::ApsisChoice {
            keep_manual: self.keep_manual,
            remind_days: self.remind_days,
            show_label: self.show_label,
        }
    }
}
