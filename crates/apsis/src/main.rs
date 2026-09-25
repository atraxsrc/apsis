// SPDX-License-Identifier: GPL-3.0-only

mod app;
mod browser;
mod config;
mod fmt;
mod i18n;
mod settings_view;

use app::Mode;
use cosmic::applet::PanelType;

fn main() -> cosmic::iced::Result {
    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    match mode(std::env::args_os().skip(1), in_panel()) {
        // Starts the applet's event loop with the mode as the application's flags.
        Mode::Applet => cosmic::applet::run::<app::AppModel>(Mode::Applet),
        Mode::Window => app::run_window(),
    }
}

/// Whether cosmic-panel started us. It sets `COSMIC_PANEL_NAME` for its applets; libcosmic reads
/// it into the applet context, and treats an empty name as "not in a panel" too (see
/// `applet::Context::autosize_window`).
fn in_panel() -> bool {
    !matches!(
        cosmic::applet::Context::default().panel_type,
        PanelType::Other(name) if name.is_empty()
    )
}

/// `--window` opens a window. So does starting outside the panel (the app launcher, a terminal,
/// `just run`), where the panel button would be a tiny square on its own.
fn mode(mut args: impl Iterator<Item = std::ffi::OsString>, in_panel: bool) -> Mode {
    if !in_panel || args.any(|arg| arg == "--window") {
        Mode::Window
    } else {
        Mode::Applet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_for(args: &[&str], in_panel: bool) -> Mode {
        mode(args.iter().map(Into::into), in_panel)
    }

    #[test]
    fn panel_without_flag_is_the_applet() {
        assert_eq!(mode_for(&[], true), Mode::Applet);
    }

    #[test]
    fn window_flag_opens_a_window_even_in_the_panel() {
        assert_eq!(mode_for(&["--window"], true), Mode::Window);
    }

    #[test]
    fn outside_the_panel_is_always_a_window() {
        assert_eq!(mode_for(&[], false), Mode::Window);
        assert_eq!(mode_for(&["--window"], false), Mode::Window);
    }
}
