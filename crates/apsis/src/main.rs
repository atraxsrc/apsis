// SPDX-License-Identifier: GPL-3.0-only

mod app;
mod config;
mod fmt;
mod i18n;
mod settings_view;
mod status;

use app::{Flags, Mode, StartView};
use cosmic::applet::PanelType;

fn main() -> cosmic::iced::Result {
    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    match mode(std::env::args_os().skip(1), in_panel()) {
        // Starts the applet's event loop with the mode as the application's flags.
        Mode::Applet => cosmic::applet::run::<app::AppModel>(Flags {
            mode: Mode::Applet,
            view: None,
        }),
        Mode::Window => app::run_window(StartView::from_args(std::env::args_os().skip(1))),
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

/// `--window` opens a window; so do `--settings` and `--about`, which open it on that view (the
/// panel's overview starts them). So does starting outside the panel (the app launcher, a terminal,
/// `just run`), where the panel button would be a tiny square on its own.
fn mode(mut args: impl Iterator<Item = std::ffi::OsString>, in_panel: bool) -> Mode {
    if !in_panel
        || args.any(|arg| matches!(arg.to_str(), Some("--window" | "--settings" | "--about")))
    {
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
    fn settings_and_about_open_the_window_too() {
        assert_eq!(mode_for(&["--settings"], true), Mode::Window);
        assert_eq!(mode_for(&["--about"], true), Mode::Window);
        assert_eq!(mode_for(&["--other"], true), Mode::Applet);
    }

    #[test]
    fn outside_the_panel_is_always_a_window() {
        assert_eq!(mode_for(&[], false), Mode::Window);
        assert_eq!(mode_for(&["--window"], false), Mode::Window);
    }
}
