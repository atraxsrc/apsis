// SPDX-License-Identifier: GPL-3.0-only

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};

/// Apsis's own settings (per user, in cosmic-config; Timeshift's are in its own file).
#[derive(Debug, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    demo: String,
    /// List and create with Apsis's native rsync backend instead of `timeshift`. Off unless
    /// the user turns it on.
    pub native_backend: bool,
    /// With the native backend, a create only shows what it would do. On until the user turns
    /// it off.
    pub native_dry_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            demo: String::new(),
            native_backend: false,
            native_dry_run: true,
        }
    }
}

impl Config {
    /// The backend choice, for the settings view.
    pub fn backend(&self) -> crate::settings_view::BackendChoice {
        crate::settings_view::BackendChoice {
            native: self.native_backend,
            dry_run: self.native_dry_run,
        }
    }
}
