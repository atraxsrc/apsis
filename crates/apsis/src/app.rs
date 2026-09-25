// SPDX-License-Identifier: GPL-3.0-only

use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use apsis_core::helper::HelperClient;
use apsis_core::restore::{
    Destination, Entry, Kind, Listing as FolderListing, Live, Request, SnapPath,
};
use apsis_core::settings::{HomeState, Level, Settings, SettingsInfo};
use apsis_core::{
    Backend, MAX_COMMENT_CHARS, PkexecRunner, Snapshot, SnapshotList, TimeshiftCli,
    validate_comment,
};
use cosmic::applet::{menu_button, padded_control};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::advanced::text::EllipsizeHeightLimit;
use cosmic::iced::border::Radius;
use cosmic::iced::keyboard::{self, Key, key::Named};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::widget::scrollable::{Direction, RelativeOffset, Scrollbar, snap_to};
use cosmic::iced::widget::svg as iced_svg;
use cosmic::iced::widget::text::{self as iced_text, Ellipsize, Wrapping};
use cosmic::iced::{Alignment, Background, Border, Color, Length, Limits, Padding, Size};
use cosmic::iced::{Subscription, event, mouse, time, window, window::Id};
use cosmic::prelude::*;
use cosmic::widget::text::{body, monotext};
use cosmic::widget::{self, container, icon};
use cosmic::{Theme, theme};

use crate::browser::{self, Browser, Load};
use crate::config::Config;
use crate::fl;
use crate::fmt;
use crate::settings_view::{BackendChoice, Row, Section, SettingsView};

/// Popup width in logical pixels (UI.md: ~720, room for the list and details side by side).
const POPUP_WIDTH: f32 = 720.0;
/// Window mode: the size it opens at (the popup's width). The panes fill whatever size it's
/// dragged to.
const WINDOW_SIZE: Size = Size::new(POPUP_WIDTH, 520.0);
/// Window mode: the smallest it can be dragged to, with the footer hints (the settings view's
/// are the widest) and a few rows still in view.
const WINDOW_MIN_SIZE: Size = Size::new(640.0, 440.0);
// The window opens no smaller than it may be dragged to.
const _: () = assert!(
    WINDOW_MIN_SIZE.width <= WINDOW_SIZE.width && WINDOW_MIN_SIZE.height <= WINDOW_SIZE.height
);
/// Width of the edge that resizes the window when dragged, in logical pixels.
const WINDOW_RESIZE_BORDER: f64 = 8.0;
/// If the window never reports focus, it becomes resizable this long after starting anyway.
const WINDOW_SHOWN_FALLBACK: Duration = Duration::from_millis(1500);
/// Width of the right-click menu.
const MENU_WIDTH: f32 = 240.0;
/// Height of one snapshot row: monotext line height (20) plus vertical padding.
const ROW_HEIGHT: f32 = 24.0;
/// Rows shown before the list scrolls.
const VISIBLE_ROWS: u16 = 8;
/// The snapshots pane is at least this many rows high, so short lists don't look cramped.
const MIN_ROWS: u16 = 5;
/// Half a monotext line: the pane border runs through the middle of the title.
const TITLE_HALF: f32 = 10.0;
/// Width of a pane's border line.
const LINE: f32 = 1.0;
/// How far the title sits from the pane's left edge, clear of the rounded corner.
const TITLE_INSET: f32 = 10.0;
/// Key column in the details pane (`comment` is the longest key).
const DETAILS_KEY_WIDTH: f32 = 64.0;
/// Key column in the help and About views.
const HELP_KEY_WIDTH: f32 = 80.0;
/// Section column in the settings view (`schedule` is the longest).
const SETTINGS_KEY_WIDTH: f32 = 72.0;
/// Longest filter typed at the `>` line.
const FILTER_CHARS: usize = 512;
/// Longest comment shown in a popup row; the row also ellipsizes to the pane width, and the
/// details pane shows all of it. A window can be wider, so there only the pane width cuts it.
const ROW_COMMENT_CHARS: usize = 28;
/// Longest breadcrumb in the browser's pane title: the popup's pane, and a window's.
const BREADCRUMB_CHARS: usize = 40;
const WINDOW_BREADCRUMB_CHARS: usize = 80;
/// Longest answer kept at the `[y/N]` prompt; only `y` means yes.
const CONFIRM_CHARS: usize = 3;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Themed name of the panel icon; installed by `just install`.
const SYMBOLIC_ICON: &str = "io.github.atraxsrc.Apsis-symbolic";
/// The same SVG, for when it isn't installed yet (`just run`).
const SYMBOLIC_ICON_SVG: &[u8] = include_bytes!(
    "../../../resources/icons/hicolor/symbolic/apps/io.github.atraxsrc.Apsis-symbolic.svg"
);
/// Size of the icon in the popup header, matching the monotext line height.
const HEADER_ICON_SIZE: u16 = 16;

/// For the About view, from `Cargo.toml`.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

static LIST_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("snapshot-list"));
static SETTINGS_LIST_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("settings-list"));
static BROWSE_LIST_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("browse-list"));
/// The `>` input line. Keeping it focused gives the popup a focused widget for key input.
static INPUT_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("prompt-input"));
/// `APSIS_DEBUG_KEYS=1` logs key and popup focus events to stderr, to see where keys get lost.
static DEBUG_KEYS: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("APSIS_DEBUG_KEYS").is_some_and(|v| v == "1"));

type Cli = TimeshiftCli<PkexecRunner>;

/// How Apsis was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Started by cosmic-panel: a panel button that opens the popup.
    Applet,
    /// `apsis --window`, or started outside the panel: the popup's UI in a normal window.
    Window,
}

/// Runs the popup's UI in a normal window, resizable by its edges and corners. Esc closes it.
///
/// It starts floating, even with tiling on: COSMIC (cosmic-comp `Shell::map_window`) decides
/// floating or tiled once, when a window first appears, and floats one whose minimum and
/// maximum size are equal. So it opens with both at [`WINDOW_SIZE`], and once it's on screen
/// ([`Message::WindowShown`]) the maximum goes and the minimum drops to [`WINDOW_MIN_SIZE`].
pub fn run_window() -> cosmic::iced::Result {
    let settings = cosmic::app::Settings::default()
        .size(WINDOW_SIZE)
        .size_limits(
            Limits::NONE
                .min_width(WINDOW_SIZE.width)
                .max_width(WINDOW_SIZE.width)
                .min_height(WINDOW_SIZE.height)
                .max_height(WINDOW_SIZE.height),
        )
        .resizable(Some(WINDOW_RESIZE_BORDER));
    cosmic::app::run::<AppModel>(settings, Mode::Window)
}

/// The applet: a panel button and a terminal-style popup listing Timeshift snapshots. In
/// [`Mode::Window`] the main window takes the popup's place.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    mode: Mode,
    /// The popup id. In window mode, the main window's id while it's open.
    popup: Option<Id>,
    /// The right-click menu's popup id. At most one of `popup` and `menu` is open.
    menu: Option<Id>,
    /// Configuration data that persists between application runs.
    config: Config,
    /// The fallback when `apsis-helper` isn't installed: `pkexec timeshift ...`, shared with
    /// the background tasks.
    pkexec: Arc<Cli>,
    listing: Listing,
    /// A list is running in the background.
    loading: bool,
    /// UUID of the backup disk from the last good list, to name it when it goes missing.
    known_uuid: Option<String>,
    /// A create or delete is running in the background (as root, via pkexec).
    running: Option<Operation>,
    /// What the `>` line is asking for.
    prompt: Prompt,
    /// How the last create or delete went, or why a comment was refused.
    status: Option<Status>,
    spinner: usize,
    /// Index into the displayed (newest first) snapshots.
    selected: usize,
    overlay: Overlay,
    /// Timeshift's settings, for the settings view.
    settings: SettingsLoad,
    /// A settings write is running in the helper.
    saving_settings: bool,
    /// The last native dry run's plan, shown by [`Overlay::DryRun`].
    dry_run_plan: Option<String>,
    /// The snapshot browser, while it's open ([`Overlay::Browse`]).
    browser: Option<Browser>,
    /// A restore whose dry run was shown: Enter runs it for real.
    pending_restore: Option<Request>,
    /// The last restore's plan or result, shown by [`Overlay::RestorePlan`].
    restore_text: Option<String>,
    /// Window mode: the window is on screen and its size limits are relaxed (see
    /// [`run_window`]).
    window_resizable: bool,
    /// The symbolic Apsis icon, from the icon theme or embedded.
    icon: icon::Handle,
}

/// Where the settings view's data is.
#[derive(Debug, Clone)]
enum SettingsLoad {
    NotLoaded,
    /// Keeps the selected row across a reload.
    Loading {
        cursor: usize,
    },
    Failed(String),
    Ready(Box<SettingsView>),
}

/// What the last list produced.
#[derive(Debug, Clone)]
enum Listing {
    /// Nothing listed yet. The first list runs when the popup first opens, not at login,
    /// because each list asks for a password until the Phase 4 helper exists.
    NotLoaded,
    /// Snapshots newest first.
    Loaded(SnapshotList),
    Failed(CliError),
}

/// A `Clone`able summary of [`apsis_core::Error`] for messages and the view.
///
/// Used for list, create and delete.
#[derive(Debug, Clone)]
pub enum CliError {
    NotInstalled,
    /// polkit refused `apsis-helper`, or the password dialog was dismissed. Timeshift didn't
    /// run. (pkexec reports the same as exit code 126 or 127, see [`pkexec_refused`].)
    NotAuthorized,
    /// Timeshift failed: its exit code and the last lines it printed about it.
    Failed {
        code: Option<i32>,
        output: Vec<String>,
    },
    /// The backup disk isn't there (unplugged, or dropped off the USB bus). `device` is what
    /// Timeshift named.
    DeviceNotFound {
        device: String,
    },
    Other(String),
}

impl From<apsis_core::Error> for CliError {
    fn from(error: apsis_core::Error) -> Self {
        match error {
            apsis_core::Error::NotInstalled => Self::NotInstalled,
            apsis_core::Error::NotAuthorized => Self::NotAuthorized,
            apsis_core::Error::Failed { code, output } => Self::Failed {
                code,
                output: fmt::tail(&output, apsis_core::MAX_OUTPUT_LINES),
            },
            apsis_core::Error::DeviceNotFound { device } => Self::DeviceNotFound { device },
            other => Self::Other(other.to_string()),
        }
    }
}

/// What the `>` line is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Prompt {
    /// Each typed character is a command key; the line stays empty.
    Command,
    /// `> comment: _` for a new snapshot. Enter creates it, Esc cancels.
    Comment(String),
    /// `> delete <name>? [y/N] _`. Enter with `y` deletes, anything else cancels.
    ConfirmDelete { name: String, typed: String },
    /// Settings: `> add filter: _`. Enter adds it.
    Filter(String),
    /// Settings: `> keep daily: 5_`. Enter sets the count.
    Count { level: Level, typed: String },
    /// Browser: `> restore 3 items to [f]older (~/Apsis-restored) or [o]riginal? _`. Enter
    /// with nothing or `f` is folder mode, `o` original; anything else cancels.
    RestoreWhere { count: usize, typed: String },
    /// Original mode, after its plan: `> put 3 items back over the running system? [y/N] _`.
    ConfirmRestore { count: usize, typed: String },
}

/// A create or delete, run as root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// With the comment as typed; the backend trims it.
    Create(String),
    Delete(String),
    /// Native backend with dry run on: what a create would do. Nothing is written.
    DryRun(String),
    /// A file-level restore, or its dry run.
    Restore(Request),
}

/// How the last create or delete went, shown in the activity pane.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Info(String),
    Error(String),
}

/// A piece of a pane's border (see [`pane`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// Left of the title: the top line and the rounded top-left corner.
    TopLeft,
    /// Right of the title: the top line and the rounded top-right corner.
    TopRight,
    /// Under the title row: both sides and the bottom, with the rounded bottom corners.
    Bottom,
}

impl Edge {
    /// Where the line shows: the fill inside is inset by [`LINE`] on these sides.
    fn line_padding(self) -> Padding {
        let mut padding = Padding::ZERO;
        match self {
            Edge::TopLeft => (padding.top, padding.left) = (LINE, LINE),
            Edge::TopRight => (padding.top, padding.right) = (LINE, LINE),
            Edge::Bottom => (padding.left, padding.right, padding.bottom) = (LINE, LINE, LINE),
        }
        padding
    }

    /// `radius` on this piece's outer corners, square elsewhere so the pieces join up.
    fn radius(self, radius: f32) -> Radius {
        let mut corners = Radius::from(0.0);
        match self {
            Edge::TopLeft => corners.top_left = radius,
            Edge::TopRight => corners.top_right = radius,
            Edge::Bottom => (corners.bottom_left, corners.bottom_right) = (radius, radius),
        }
        corners
    }
}

/// How a line in the activity pane looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    Dim,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Details,
    Help,
    About,
    Settings,
    /// The last native dry run's plan, in the left pane.
    DryRun,
    /// A snapshot's files, in the left pane; the selected entry's details on the right.
    Browse,
    /// A restore's plan (Enter runs it) or result, in the left pane.
    RestorePlan,
}

/// Keys the popup reacts to (see UI.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Up,
    Down,
    First,
    Last,
    Details,
    Refresh,
    Help,
    Create,
    Delete,
    Escape,
    /// `s`: the settings view.
    Settings,
    /// Settings: space changes the selected row.
    Toggle,
    /// Settings: `+` / `-` change a schedule's count, `e` types it.
    More,
    Less,
    Edit,
    /// Settings: `a` adds a filter, `x` removes the selected one.
    Add,
    Remove,
    /// Settings: `w` writes them to Timeshift.
    Write,
    /// Browser: `h` or Backspace goes up a folder, `l` into one.
    Back,
    Into,
    /// Browser: `J` marks or unmarks, then moves down (space marks in place).
    MarkDown,
    /// Browser: `R` restores the marked entries.
    Restore,
    /// Tab: the details pane of the selected snapshot.
    FocusDetails,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    /// Right-click on the panel button.
    ToggleMenu,
    /// Menu: open the popup and refresh, like `r`.
    MenuRefresh,
    /// Menu: open the popup on the About view.
    MenuAbout,
    /// Menu: open the popup on the settings view.
    MenuSettings,
    /// Menu: `cosmic-settings panel`.
    MenuPanelSettings,
    /// The repository link in the About view.
    OpenRepository,
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
    UpdateConfig(Config),
    Key(Id, KeyAction),
    /// Text typed into the `>` line while it has focus.
    Input(String),
    /// Enter in the `>` line.
    Submit,
    /// The `>` line lost focus (Esc or Tab in it).
    InputUnfocused,
    Refresh,
    /// `[c]reate` / `[d]elete` hints.
    StartCreate,
    StartDelete,
    Listed(Result<SnapshotList, CliError>),
    /// A create or delete finished.
    Finished(Operation, Result<(), CliError>),
    /// A native dry run finished: the plan's text.
    DryRunDone(Result<String, CliError>),
    Tick,
    Select(usize),
    /// Double-click on a snapshot: its files.
    OpenBrowser(usize),
    /// A browse finished: the snapshot and folder it was for.
    Browsed(String, SnapPath, Result<FolderListing, CliError>),
    /// A click on a browser row, and a double-click (into a folder, or marks a file).
    BrowseSelect(usize),
    BrowseActivate(usize),
    /// A browser hint button.
    BrowseKey(KeyAction),
    /// A restore (or its dry run) finished: the plan's or result's text.
    RestoreDone(Request, Result<String, CliError>),
    ToggleHelp,
    Escape,
    /// `[s]ettings` hint.
    OpenSettings,
    SettingsRead(Result<SettingsInfo, CliError>),
    /// A settings write finished: a note from the helper (empty when all went well).
    SettingsWritten(Result<String, CliError>),
    /// A click on a settings row, and a double-click (changes it, like space).
    SettingsSelect(usize),
    SettingsActivate(usize),
    /// A settings hint button.
    SettingsKey(KeyAction),
    /// Window mode: the window is on screen (first focus, or the fallback timer).
    WindowShown,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = Mode;
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "io.github.atraxsrc.Apsis";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, mode: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) | Err((_, config)) => config,
            })
            .unwrap_or_default();
        let mut app = AppModel {
            core,
            mode,
            popup: None,
            menu: None,
            config,
            pkexec: Arc::new(TimeshiftCli::new(PkexecRunner)),
            listing: Listing::NotLoaded,
            known_uuid: None,
            loading: false,
            running: None,
            prompt: Prompt::Command,
            status: None,
            spinner: 0,
            selected: 0,
            overlay: Overlay::None,
            settings: SettingsLoad::NotLoaded,
            saving_settings: false,
            dry_run_plan: None,
            browser: None,
            pending_restore: None,
            restore_text: None,
            window_resizable: false,
            icon: symbolic_icon(),
        };
        let task = match mode {
            Mode::Applet => Task::none(),
            Mode::Window => app.open_window(),
        };
        (app, task)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// The panel button, with a tooltip saying how old the newest snapshot is. Left click opens
    /// the popup, right click the menu. The button itself only reacts to the left button.
    ///
    /// In window mode, the popup's UI instead.
    fn view(&self) -> Element<'_, Self::Message> {
        if self.mode == Mode::Window {
            return self.surface();
        }
        let button = self
            .core
            .applet
            .icon_button_from_handle(self.icon.clone())
            .on_press(Message::TogglePopup);
        let button = widget::mouse_area(button).on_right_release(Message::ToggleMenu);
        self.core
            .applet
            .applet_tooltip(
                button,
                self.tooltip(),
                self.popup.is_some() || self.menu.is_some(),
                Message::Surface,
                None,
            )
            .into()
    }

    /// The terminal-style popup, or the right-click menu.
    fn view_window(&self, id: Id) -> Element<'_, Self::Message> {
        if self.menu == Some(id) {
            return self.menu_view();
        }
        self.core
            .applet
            .popup_container(self.surface())
            .limits(
                Limits::NONE
                    .min_width(POPUP_WIDTH)
                    .max_width(POPUP_WIDTH)
                    .min_height(1.0)
                    .max_height(1000.0),
            )
            .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];
        if self.popup.is_some() || self.menu.is_some() {
            subscriptions.push(event::listen_with(key_action));
        }
        if self.mode == Mode::Window && !self.window_resizable {
            subscriptions.push(event::listen_with(window_focused));
            subscriptions.push(time::every(WINDOW_SHOWN_FALLBACK).map(|_| Message::WindowShown));
        }
        let settings_busy = self.saving_settings
            || matches!(self.settings, SettingsLoad::Loading { .. })
            || self.browser.as_ref().is_some_and(Browser::is_loading);
        if self.popup.is_some() && (self.loading || self.running.is_some() || settings_busy) {
            subscriptions.push(time::every(Duration::from_millis(80)).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    #[allow(clippy::too_many_lines, reason = "one arm per message")]
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::UpdateConfig(config) => self.config = config,
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Surface(action));
            }
            Message::TogglePopup => return self.toggle_popup(),
            Message::ToggleMenu => return self.toggle_menu(),
            Message::MenuRefresh => {
                let open = self.open_popup(Overlay::None);
                return Task::batch([open, self.start_list()]);
            }
            Message::MenuAbout => return self.open_popup(Overlay::About),
            Message::MenuSettings => return self.open_settings_popup(),
            Message::OpenSettings => return self.on_key(KeyAction::Settings),
            Message::SettingsRead(result) => return self.on_settings_read(result),
            Message::SettingsWritten(result) => return self.on_settings_written(result),
            Message::SettingsSelect(index) => return self.select_setting(index, false),
            Message::SettingsActivate(index) => return self.select_setting(index, true),
            Message::SettingsKey(action) => return self.on_key(action),
            Message::WindowShown => return self.make_window_resizable(),
            Message::MenuPanelSettings => {
                let close = self.close_menu();
                let mut settings = std::process::Command::new("cosmic-settings");
                settings.arg("panel");
                return Task::batch([close, spawn(settings)]);
            }
            Message::OpenRepository => {
                let mut open = std::process::Command::new("xdg-open");
                open.arg(REPOSITORY);
                return spawn(open);
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.reset_popup_state();
                }
                if self.menu == Some(id) {
                    self.menu = None;
                }
            }
            Message::Key(id, action) if self.popup == Some(id) => return self.on_key(action),
            Message::Key(id, KeyAction::Escape) if self.menu == Some(id) => {
                return self.close_menu();
            }
            Message::Key(id, action) => {
                if *DEBUG_KEYS {
                    eprintln!(
                        "apsis: dropped {action:?} for window {id:?}, popup {:?}",
                        self.popup
                    );
                }
            }
            Message::Input(text) => match &mut self.prompt {
                Prompt::Command => {
                    // Each typed character is a command key; the line stays empty.
                    let tasks: Vec<_> = text
                        .chars()
                        .filter_map(char_action)
                        .map(|action| self.on_key(action))
                        .collect();
                    return Task::batch(tasks);
                }
                Prompt::Comment(comment) => {
                    *comment = text.chars().take(MAX_COMMENT_CHARS).collect();
                }
                Prompt::ConfirmDelete { typed, .. } => {
                    *typed = text.chars().take(CONFIRM_CHARS).collect();
                }
                Prompt::Filter(pattern) => {
                    *pattern = text.chars().take(FILTER_CHARS).collect();
                }
                Prompt::Count { typed, .. } => {
                    *typed = text.chars().filter(char::is_ascii_digit).take(3).collect();
                }
                Prompt::RestoreWhere { typed, .. } | Prompt::ConfirmRestore { typed, .. } => {
                    *typed = text.chars().take(CONFIRM_CHARS).collect();
                }
            },
            Message::Submit => return self.submit(),
            Message::InputUnfocused if self.popup.is_some() => return focus_input(),
            Message::InputUnfocused => {}
            Message::Refresh => return self.start_list(),
            Message::StartCreate => return self.on_key(KeyAction::Create),
            Message::StartDelete => return self.on_key(KeyAction::Delete),
            Message::Finished(operation, result) => return self.on_finished(&operation, result),
            Message::DryRunDone(result) => return self.on_dry_run(result),
            Message::Listed(result) => {
                self.on_listed(result);
                // The polkit dialog took keyboard focus; hand it back to the `>` line.
                if self.popup.is_some() {
                    return focus_input();
                }
            }
            Message::Tick => self.spinner = (self.spinner + 1) % SPINNER.len(),
            Message::Select(index) => {
                self.selected = index;
                self.overlay = Overlay::None;
            }
            Message::OpenBrowser(index) => {
                self.selected = index;
                return self.open_browser();
            }
            Message::Browsed(snapshot, path, result) => {
                return self.on_browsed(&snapshot, &path, result);
            }
            Message::BrowseSelect(index) => {
                if let Some(browser) = &mut self.browser {
                    browser.select(index);
                }
            }
            Message::BrowseActivate(index) => {
                let Some(browser) = &mut self.browser else {
                    return Task::none();
                };
                browser.select(index);
                let is_dir = browser.current().is_some_and(|e| e.kind == Kind::Dir);
                return self.on_key(if is_dir {
                    KeyAction::Into
                } else {
                    KeyAction::Toggle
                });
            }
            Message::BrowseKey(action) => return self.on_key(action),
            Message::RestoreDone(request, result) => return self.on_restore_done(request, result),
            Message::ToggleHelp => self.toggle_overlay(Overlay::Help),
            Message::Escape => return self.escape(),
        }
        Task::none()
    }

    /// The applet's transparent surfaces; a window keeps the default opaque background.
    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        match self.mode {
            Mode::Applet => Some(cosmic::applet::style()),
            Mode::Window => None,
        }
    }
}

// Update helpers.
impl AppModel {
    /// Window mode: the main window is the popup. Lists at once, and focuses the `>` line so
    /// typing works straight away (command keys work without it too, see [`key_action`]).
    fn open_window(&mut self) -> Task<cosmic::Action<Message>> {
        self.core.window.show_maximize = false;
        self.core.window.show_minimize = false;
        self.core.set_header_title(fl!("app-title"));
        self.popup = self.core.main_window_id();
        Task::batch([focus_input(), self.start_list()])
    }

    /// Window mode, once the window is on screen: drops the maximum size and lowers the minimum,
    /// so it can be resized. COSMIC has already placed it floating (see [`run_window`]).
    fn make_window_resizable(&mut self) -> Task<cosmic::Action<Message>> {
        if self.mode != Mode::Window || self.window_resizable {
            return Task::none();
        }
        let Some(id) = self.core.main_window_id() else {
            return Task::none();
        };
        self.window_resizable = true;
        Task::batch([
            window::set_max_size(id, None),
            window::set_min_size(id, Some(WINDOW_MIN_SIZE)),
        ])
    }

    /// Closes the popup, or in window mode the window (which quits Apsis).
    fn close_surface(&self, id: Id) -> Task<cosmic::Action<Message>> {
        match self.mode {
            Mode::Applet => destroy_popup(id),
            Mode::Window => window::close(id),
        }
    }

    fn toggle_popup(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(popup) = self.popup.take() {
            self.reset_popup_state();
            return destroy_popup(popup);
        }
        self.open_popup(Overlay::None)
    }

    /// Opens the right-click menu, closing the popup first; or closes the menu.
    fn toggle_menu(&mut self) -> Task<cosmic::Action<Message>> {
        if self.menu.is_some() {
            return self.close_menu();
        }
        let Some(parent) = self.core.main_window_id() else {
            return Task::none();
        };
        let close = match self.popup.take() {
            Some(popup) => {
                self.reset_popup_state();
                destroy_popup(popup)
            }
            None => Task::none(),
        };
        let id = Id::unique();
        self.menu = Some(id);
        let mut settings = self
            .core
            .applet
            .get_popup_settings(parent, id, None, None, None);
        settings.positioner.size_limits = Limits::NONE
            .min_width(MENU_WIDTH)
            .max_width(MENU_WIDTH)
            .min_height(1.0)
            .max_height(1080.0);
        close.chain(get_popup(settings))
    }

    fn close_menu(&mut self) -> Task<cosmic::Action<Message>> {
        match self.menu.take() {
            Some(menu) => destroy_popup(menu),
            None => Task::none(),
        }
    }

    /// Opens the popup on `overlay`, closing the menu first. If the popup is already open, only
    /// switches the overlay. The first opening also starts the first list.
    fn open_popup(&mut self, overlay: Overlay) -> Task<cosmic::Action<Message>> {
        let close = self.close_menu();
        self.overlay = overlay;
        if self.popup.is_some() {
            return close;
        }
        let Some(parent) = self.core.main_window_id() else {
            return close;
        };
        let id = Id::unique();
        self.popup = Some(id);
        let mut settings = self
            .core
            .applet
            .get_popup_settings(parent, id, None, None, None);
        settings.positioner.size_limits = Limits::NONE
            .min_width(POPUP_WIDTH)
            .max_width(POPUP_WIDTH)
            .min_height(1.0)
            .max_height(1080.0);
        let open = close.chain(Task::batch([get_popup(settings), focus_input()]));
        if matches!(self.listing, Listing::NotLoaded) {
            Task::batch([open, self.start_list()])
        } else {
            open
        }
    }

    /// The popup closed: drop overlays, the browser and any half-typed prompt. A running
    /// operation and its status line stay.
    fn reset_popup_state(&mut self) {
        self.overlay = Overlay::None;
        self.prompt = Prompt::Command;
        self.browser = None;
        self.pending_restore = None;
    }

    /// Lists in the background, through `apsis-helper` or pkexec. Does nothing while a list,
    /// create or delete is running (Timeshift runs one at a time).
    fn start_list(&mut self) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        self.loading = true;
        self.spinner = 0;
        let pkexec = Arc::clone(&self.pkexec);
        let native = self.config.native_backend;
        cosmic::task::future(async move { Message::Listed(list_snapshots(pkexec, native).await) })
    }

    fn on_listed(&mut self, result: Result<SnapshotList, CliError>) {
        self.loading = false;
        match result {
            Ok(mut list) => {
                // Keep the same snapshot selected across refreshes when it still exists.
                // If it was deleted, stay at the same position.
                let selected_name = self.snapshots().get(self.selected).map(|s| s.name.clone());
                list.snapshots.sort_by_key(|s| std::cmp::Reverse(s.created));
                self.known_uuid.clone_from(&list.uuid);
                let near = self.selected.min(list.snapshots.len().saturating_sub(1));
                self.selected = selected_name
                    .and_then(|name| list.snapshots.iter().position(|s| s.name == name))
                    .unwrap_or(near);
                self.listing = Listing::Loaded(list);
            }
            Err(error) => {
                self.listing = Listing::Failed(error);
                self.selected = 0;
            }
        }
        if self.overlay == Overlay::Details && self.snapshots().is_empty() {
            self.overlay = Overlay::None;
        }
    }

    /// Whether `c` can start a create: a list showed a snapshot device and nothing is running.
    fn can_create(&self) -> bool {
        let has_device =
            matches!(&self.listing, Listing::Loaded(list) if list.snapshot_device().is_some());
        has_device && !self.loading && self.running.is_none()
    }

    /// Whether `d` can start a delete: as for create, plus a snapshot is selected.
    fn can_delete(&self) -> bool {
        self.can_create() && self.selected < self.snapshots().len()
    }

    fn on_key(&mut self, action: KeyAction) -> Task<cosmic::Action<Message>> {
        // UI.md: while a create or delete runs, only Esc works (and doesn't cancel it).
        if self.running.is_some() && action != KeyAction::Escape {
            return Task::none();
        }
        if self.prompt != Prompt::Command {
            return match action {
                KeyAction::Escape => self.escape(),
                // Enter the `>` line didn't capture (it had lost focus).
                KeyAction::Details => self.submit(),
                _ => Task::none(),
            };
        }
        match self.overlay {
            Overlay::Settings => return self.settings_key(action),
            Overlay::Browse => return self.browse_key(action),
            Overlay::RestorePlan => return self.plan_key(action),
            _ => {}
        }
        let count = self.snapshots().len();
        let target = match action {
            KeyAction::Up => self.selected.checked_sub(1),
            KeyAction::Down => Some(self.selected + 1).filter(|&i| i < count),
            KeyAction::First => (count > 0).then_some(0),
            KeyAction::Last => count.checked_sub(1),
            KeyAction::Details => return self.open_browser(),
            KeyAction::FocusDetails => {
                if count > 0 {
                    self.toggle_overlay(Overlay::Details);
                }
                None
            }
            KeyAction::Refresh => return self.start_list(),
            KeyAction::Create => {
                if self.can_create() {
                    self.overlay = Overlay::None;
                    self.status = None;
                    self.prompt = Prompt::Comment(String::new());
                }
                return focus_input();
            }
            KeyAction::Delete => {
                if self.can_delete() {
                    self.overlay = Overlay::None;
                    self.status = None;
                    self.prompt = Prompt::ConfirmDelete {
                        name: self.snapshots()[self.selected].name.clone(),
                        typed: String::new(),
                    };
                }
                return focus_input();
            }
            KeyAction::Help => {
                self.toggle_overlay(Overlay::Help);
                None
            }
            KeyAction::Escape => return self.escape(),
            KeyAction::Settings => {
                self.overlay = Overlay::Settings;
                return self.load_settings_if_needed();
            }
            // Settings keys.
            KeyAction::Toggle
            | KeyAction::More
            | KeyAction::Less
            | KeyAction::Edit
            | KeyAction::Add
            | KeyAction::Remove
            | KeyAction::Write
            // Browser keys.
            | KeyAction::Back
            | KeyAction::Into
            | KeyAction::MarkDown
            | KeyAction::Restore => None,
        };
        let Some(index) = target else {
            return Task::none();
        };
        self.selected = index;
        scroll_to(&LIST_ID, index, count)
    }

    /// Keys in the settings view. Changes stay in the view until `w` writes them.
    fn settings_key(&mut self, action: KeyAction) -> Task<cosmic::Action<Message>> {
        match action {
            KeyAction::Escape | KeyAction::Settings => return self.escape(),
            KeyAction::Help => {
                self.toggle_overlay(Overlay::Help);
                return Task::none();
            }
            // Reads the file again, dropping unsaved changes.
            KeyAction::Refresh => return self.load_settings(),
            KeyAction::Write => return self.save_settings(),
            _ => {}
        }
        let SettingsLoad::Ready(view) = &mut self.settings else {
            return Task::none();
        };
        if self.saving_settings {
            return Task::none();
        }
        let rows = view.rows().len();
        let result = match action {
            KeyAction::Up | KeyAction::Down | KeyAction::First | KeyAction::Last => {
                let index = match action {
                    KeyAction::Up => view.cursor.saturating_sub(1),
                    KeyAction::Down => view.cursor + 1,
                    KeyAction::First => 0,
                    _ => rows - 1,
                };
                view.select(index);
                return scroll_to(&SETTINGS_LIST_ID, view.cursor, rows);
            }
            KeyAction::Details | KeyAction::Toggle if view.current() == Row::AddFilter => {
                self.prompt = Prompt::Filter(String::new());
                self.status = None;
                return focus_input();
            }
            KeyAction::Add => {
                self.prompt = Prompt::Filter(String::new());
                self.status = None;
                return focus_input();
            }
            KeyAction::Edit => match view.current() {
                Row::Schedule(level) => {
                    let typed = view.edited.count(level).to_string();
                    self.prompt = Prompt::Count { level, typed };
                    self.status = None;
                    return focus_input();
                }
                _ => Err(fl!("settings-edit-hint")),
            },
            KeyAction::Details | KeyAction::Toggle => view.change(),
            KeyAction::More => view.adjust_count(true),
            KeyAction::Less => view.adjust_count(false),
            KeyAction::Remove => view.remove_filter(),
            // Snapshot keys do nothing here.
            _ => Ok(()),
        };
        let backend = view.backend;
        self.status = result.err().map(Status::Error);
        if backend != self.config.backend() {
            return self.set_backend(backend);
        }
        Task::none()
    }

    /// Menu: the popup on the settings view.
    fn open_settings_popup(&mut self) -> Task<cosmic::Action<Message>> {
        let open = self.open_popup(Overlay::Settings);
        Task::batch([open, self.load_settings_if_needed()])
    }

    /// A click on a settings row selects it; a double-click (`change`) changes it, like space.
    fn select_setting(&mut self, index: usize, change: bool) -> Task<cosmic::Action<Message>> {
        if let SettingsLoad::Ready(view) = &mut self.settings {
            view.select(index);
        }
        if change {
            return self.on_key(KeyAction::Toggle);
        }
        Task::none()
    }

    /// Reads the settings unless they're loading, or edited and not saved yet.
    fn load_settings_if_needed(&mut self) -> Task<cosmic::Action<Message>> {
        match &self.settings {
            SettingsLoad::Loading { .. } => Task::none(),
            SettingsLoad::Ready(view) if view.dirty() => Task::none(),
            _ => self.load_settings(),
        }
    }

    /// Reads Timeshift's settings through the helper, dropping unsaved changes.
    fn load_settings(&mut self) -> Task<cosmic::Action<Message>> {
        if self.saving_settings || matches!(self.settings, SettingsLoad::Loading { .. }) {
            return Task::none();
        }
        let cursor = match &self.settings {
            SettingsLoad::Ready(view) => view.cursor,
            _ => 0,
        };
        self.settings = SettingsLoad::Loading { cursor };
        self.spinner = 0;
        cosmic::task::future(async { Message::SettingsRead(read_settings().await) })
    }

    fn on_settings_read(
        &mut self,
        result: Result<SettingsInfo, CliError>,
    ) -> Task<cosmic::Action<Message>> {
        let cursor = match self.settings {
            SettingsLoad::Loading { cursor } => cursor,
            _ => 0,
        };
        self.settings = match result {
            Ok(info) => {
                let mut view = SettingsView::new(info, self.config.backend());
                view.select(cursor);
                SettingsLoad::Ready(Box::new(view))
            }
            Err(error) => SettingsLoad::Failed(error_summary(&error, self.known_uuid.as_deref())),
        };
        if self.popup.is_some() {
            return focus_input();
        }
        Task::none()
    }

    /// `w`: checks the edits here first (so a mistake shows before the password dialog), then
    /// has the helper write them.
    fn save_settings(&mut self) -> Task<cosmic::Action<Message>> {
        if self.saving_settings || self.loading || self.running.is_some() {
            return Task::none();
        }
        let SettingsLoad::Ready(view) = &self.settings else {
            return Task::none();
        };
        if !view.dirty() {
            self.status = Some(Status::Info(fl!("settings-unchanged")));
            return Task::none();
        }
        if let Err(reason) = view.validate() {
            self.status = Some(Status::Error(reason));
            return Task::none();
        }
        let (expected, settings) = (view.info.text.clone(), view.edited.clone());
        self.saving_settings = true;
        self.status = None;
        self.spinner = 0;
        cosmic::task::future(async move {
            Message::SettingsWritten(write_settings(expected, settings).await)
        })
    }

    /// Shows how the write went. Once written, reads the settings back and lists again (the
    /// backup device may have changed).
    fn on_settings_written(
        &mut self,
        result: Result<String, CliError>,
    ) -> Task<cosmic::Action<Message>> {
        self.saving_settings = false;
        let written = result.is_ok();
        self.status = Some(match result {
            Ok(note) if note.is_empty() => Status::Info(fl!("settings-saved")),
            Ok(note) => Status::Error(fl!("settings-saved-note", note = note)),
            Err(error) => Status::Error(fl!(
                "settings-failed",
                reason = error_summary(&error, self.known_uuid.as_deref())
            )),
        });
        let mut tasks = Vec::new();
        if written {
            tasks.push(self.load_settings());
            tasks.push(self.start_list());
        }
        // The polkit dialog took keyboard focus; hand it back to the `>` line.
        if self.popup.is_some() {
            tasks.push(focus_input());
        }
        Task::batch(tasks)
    }

    /// Enter: runs what the prompt asked for, or shows details.
    fn submit(&mut self) -> Task<cosmic::Action<Message>> {
        match std::mem::replace(&mut self.prompt, Prompt::Command) {
            Prompt::Command if self.overlay == Overlay::Settings => {
                self.settings_key(KeyAction::Toggle)
            }
            Prompt::Command if self.overlay == Overlay::Browse => {
                self.browse_key(KeyAction::Details)
            }
            Prompt::Command if self.overlay == Overlay::RestorePlan => {
                self.plan_key(KeyAction::Details)
            }
            Prompt::Filter(pattern) => {
                if let SettingsLoad::Ready(view) = &mut self.settings {
                    match view.add_filter(&pattern) {
                        Ok(()) => self.status = None,
                        Err(reason) => {
                            self.status = Some(Status::Error(reason));
                            self.prompt = Prompt::Filter(pattern);
                        }
                    }
                }
                Task::none()
            }
            Prompt::Count { level, typed } => {
                if let SettingsLoad::Ready(view) = &mut self.settings {
                    match view.set_count(level, &typed) {
                        Ok(()) => self.status = None,
                        Err(reason) => {
                            self.status = Some(Status::Error(reason));
                            self.prompt = Prompt::Count { level, typed };
                        }
                    }
                }
                Task::none()
            }
            Prompt::Command => self.open_browser(),
            Prompt::RestoreWhere { typed, .. } => {
                let destination = match typed.trim().to_ascii_lowercase().as_str() {
                    "" | "f" => Destination::Folder,
                    "o" => Destination::Original,
                    _ => {
                        self.status = Some(Status::Info(fl!("restore-cancelled")));
                        return Task::none();
                    }
                };
                let Some(browser) = &self.browser else {
                    return Task::none();
                };
                let request = Request {
                    snapshot: browser.snapshot.clone(),
                    paths: browser.targets(),
                    destination,
                    dry_run: true,
                };
                self.pending_restore = None;
                self.run(Operation::Restore(request))
            }
            Prompt::ConfirmRestore { typed, .. } => {
                if typed.trim().eq_ignore_ascii_case("y")
                    && let Some(request) = self.pending_restore.take()
                {
                    self.run(Operation::Restore(request))
                } else {
                    self.status = Some(Status::Info(fl!("restore-cancelled")));
                    Task::none()
                }
            }
            Prompt::Comment(comment) => {
                // Checked here too, so a bad comment can be fixed before the password prompt.
                if let Err(error) = validate_comment(&comment) {
                    self.status = Some(Status::Error(error.to_string()));
                    self.prompt = Prompt::Comment(comment);
                    return Task::none();
                }
                if self.config.native_backend && self.config.native_dry_run {
                    self.run(Operation::DryRun(comment))
                } else {
                    self.run(Operation::Create(comment))
                }
            }
            Prompt::ConfirmDelete { name, typed } => {
                if typed.trim().eq_ignore_ascii_case("y") {
                    self.run(Operation::Delete(name))
                } else {
                    self.status = Some(Status::Info(fl!("delete-cancelled")));
                    Task::none()
                }
            }
        }
    }

    /// Creates or deletes in the background, through `apsis-helper` or pkexec.
    fn run(&mut self, operation: Operation) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        self.running = Some(operation.clone());
        self.status = None;
        self.spinner = 0;
        if let Operation::DryRun(comment) = operation {
            return cosmic::task::future(async move {
                Message::DryRunDone(native_dry_run(&comment).await)
            });
        }
        if let Operation::Restore(request) = operation {
            return cosmic::task::future(async move {
                let result = restore(&request).await;
                Message::RestoreDone(request, result)
            });
        }
        let pkexec = Arc::clone(&self.pkexec);
        let native = self.config.native_backend;
        cosmic::task::future(async move {
            let result = operate(pkexec, operation.clone(), native).await;
            Message::Finished(operation, result)
        })
    }

    /// Shows how it went and refreshes the list if Timeshift ran (not when polkit refused, or
    /// the input or device check stopped it first: nothing changed, and without the helper a
    /// refresh would be another password prompt).
    fn on_finished(
        &mut self,
        operation: &Operation,
        result: Result<(), CliError>,
    ) -> Task<cosmic::Action<Message>> {
        self.running = None;
        let ran = match &result {
            Ok(()) => true,
            Err(CliError::Failed { code, .. }) => !pkexec_refused(*code),
            // Nothing ran, or (disk missing) a list would only fail again.
            Err(
                CliError::NotInstalled
                | CliError::NotAuthorized
                | CliError::DeviceNotFound { .. }
                | CliError::Other(_),
            ) => false,
        };
        self.status = Some(match (operation, result) {
            (Operation::Create(_), Ok(())) => Status::Info(fl!("created")),
            (Operation::Delete(name), Ok(())) => Status::Info(fl!("deleted", name = name.clone())),
            (Operation::Create(_), Err(error)) => {
                let reason = error_summary(&error, self.known_uuid.as_deref());
                Status::Error(fl!("create-failed", reason = reason))
            }
            (Operation::Delete(_), Err(error)) => {
                let reason = error_summary(&error, self.known_uuid.as_deref());
                Status::Error(fl!("delete-failed", reason = reason))
            }
            // Dry runs end in `on_dry_run`, restores in `on_restore_done`.
            (Operation::DryRun(_), _) => Status::Info(fl!("dry-run-done")),
            (Operation::Restore(_), _) => Status::Info(fl!("restore-done")),
        });
        let mut tasks = Vec::new();
        if ran {
            tasks.push(self.start_list());
        }
        // The polkit dialog took keyboard focus; hand it back to the `>` line.
        if self.popup.is_some() {
            tasks.push(focus_input());
        }
        Task::batch(tasks)
    }

    /// A native dry run ended: its plan goes in the left pane. Nothing was written, so there's
    /// nothing to refresh.
    fn on_dry_run(&mut self, result: Result<String, CliError>) -> Task<cosmic::Action<Message>> {
        self.running = None;
        match result {
            Ok(plan) => {
                self.dry_run_plan = Some(plan);
                self.overlay = Overlay::DryRun;
                self.status = Some(Status::Info(fl!("dry-run-done")));
            }
            Err(error) => {
                let reason = error_summary(&error, self.known_uuid.as_deref());
                self.status = Some(Status::Error(fl!("dry-run-failed", reason = reason)));
            }
        }
        if self.popup.is_some() {
            return focus_input();
        }
        Task::none()
    }

    /// Enter on a snapshot: its files, from `/`, through the helper (password once, cached).
    fn open_browser(&mut self) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        let Some(snapshot) = self.snapshots().get(self.selected) else {
            return Task::none();
        };
        self.browser = Some(Browser::new(snapshot.name.clone()));
        self.pending_restore = None;
        self.overlay = Overlay::Browse;
        self.status = None;
        self.fetch_browse(SnapPath::root())
    }

    /// Reads folder `path` of the browser's snapshot in the background.
    fn fetch_browse(&mut self, path: SnapPath) -> Task<cosmic::Action<Message>> {
        let Some(browser) = &self.browser else {
            return Task::none();
        };
        let snapshot = browser.snapshot.clone();
        self.spinner = 0;
        cosmic::task::future(async move {
            let result = browse(&snapshot, &path.to_string()).await;
            Message::Browsed(snapshot, path, result)
        })
    }

    fn on_browsed(
        &mut self,
        snapshot: &str,
        path: &SnapPath,
        result: Result<FolderListing, CliError>,
    ) -> Task<cosmic::Action<Message>> {
        let result = result.map_err(|e| error_summary(&e, self.known_uuid.as_deref()));
        if let Some(browser) = &mut self.browser
            && browser.snapshot == snapshot
        {
            browser.loaded(path, result);
        }
        // The polkit dialog took keyboard focus; hand it back to the `>` line.
        if self.popup.is_some() {
            return focus_input();
        }
        Task::none()
    }

    /// Keys in the browser.
    fn browse_key(&mut self, action: KeyAction) -> Task<cosmic::Action<Message>> {
        if action == KeyAction::Escape {
            return self.escape();
        }
        let Some(browser) = &mut self.browser else {
            return Task::none();
        };
        let count = browser.entries().len();
        match action {
            KeyAction::Up | KeyAction::Down | KeyAction::First | KeyAction::Last => {
                let Some(last) = count.checked_sub(1) else {
                    return Task::none();
                };
                let index = match action {
                    KeyAction::Up => browser.cursor.saturating_sub(1),
                    KeyAction::Down => (browser.cursor + 1).min(last),
                    KeyAction::First => 0,
                    _ => last,
                };
                browser.select(index);
                return scroll_to(&BROWSE_LIST_ID, index, count);
            }
            KeyAction::Details | KeyAction::Into => {
                if let Some(path) = browser.enter() {
                    return self.fetch_browse(path);
                }
            }
            KeyAction::Back => {
                if let Some(path) = browser.up() {
                    return self.fetch_browse(path);
                }
            }
            KeyAction::Refresh => {
                let path = browser.reload();
                return self.fetch_browse(path);
            }
            KeyAction::Toggle => browser.toggle_mark(),
            KeyAction::MarkDown => {
                browser.toggle_mark_and_move();
                return scroll_to(&BROWSE_LIST_ID, browser.cursor, browser.entries().len());
            }
            KeyAction::Restore if !browser.is_loading() => {
                let count = browser.targets().len();
                if count > 0 {
                    self.status = None;
                    self.prompt = Prompt::RestoreWhere {
                        count,
                        typed: String::new(),
                    };
                    return focus_input();
                }
            }
            // Snapshot and settings keys do nothing here.
            _ => {}
        }
        Task::none()
    }

    /// Keys on a restore's plan: Enter runs it, Esc goes back to the browser.
    fn plan_key(&mut self, action: KeyAction) -> Task<cosmic::Action<Message>> {
        match action {
            KeyAction::Escape => self.escape(),
            KeyAction::Details => self.run_pending_restore(),
            _ => Task::none(),
        }
    }

    /// Enter on a plan: folder mode runs at once; original mode asks for `y` first.
    fn run_pending_restore(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(request) = self.pending_restore.clone() else {
            return Task::none();
        };
        match request.destination {
            Destination::Folder => {
                self.pending_restore = None;
                self.run(Operation::Restore(request))
            }
            Destination::Original => {
                self.status = None;
                self.prompt = Prompt::ConfirmRestore {
                    count: request.paths.len(),
                    typed: String::new(),
                };
                focus_input()
            }
        }
    }

    /// A restore or its dry run ended. A dry run's plan goes in the left pane, ready to run; a
    /// real run's result too, and the folder is read again (the running system changed).
    fn on_restore_done(
        &mut self,
        request: Request,
        result: Result<String, CliError>,
    ) -> Task<cosmic::Action<Message>> {
        self.running = None;
        let mut tasks = Vec::new();
        match (request.dry_run, result) {
            (true, Ok(plan)) => {
                self.pending_restore = Some(Request {
                    dry_run: false,
                    ..request
                });
                self.restore_text = Some(plan);
                self.overlay = Overlay::RestorePlan;
                self.status = Some(Status::Info(fl!("restore-plan-ready")));
            }
            (true, Err(error)) => {
                let reason = error_summary(&error, self.known_uuid.as_deref());
                self.status = Some(Status::Error(fl!(
                    "restore-dry-run-failed",
                    reason = reason
                )));
            }
            (false, result) => {
                match result {
                    Ok(text) => {
                        self.restore_text = Some(text);
                        self.overlay = Overlay::RestorePlan;
                        self.status = Some(Status::Info(fl!("restore-done")));
                        if let Some(browser) = &mut self.browser {
                            browser.marked.clear();
                        }
                    }
                    Err(error) => {
                        let reason = error_summary(&error, self.known_uuid.as_deref());
                        self.status = Some(Status::Error(fl!("restore-failed", reason = reason)));
                        self.overlay = Overlay::Browse;
                    }
                }
                if let Some(browser) = &mut self.browser {
                    let path = browser.reload();
                    tasks.push(self.fetch_browse(path));
                }
            }
        }
        // The polkit dialog took keyboard focus; hand it back to the `>` line.
        if self.popup.is_some() {
            tasks.push(focus_input());
        }
        Task::batch(tasks)
    }

    /// Saves Apsis's backend choice (changed in the settings view) to cosmic-config, and lists
    /// again when the backend itself changed.
    fn set_backend(&mut self, choice: BackendChoice) -> Task<cosmic::Action<Message>> {
        let switched = choice.native != self.config.native_backend;
        let saved =
            cosmic_config::Config::new(<Self as cosmic::Application>::APP_ID, Config::VERSION)
                .and_then(|context| {
                    self.config.set_native_backend(&context, choice.native)?;
                    self.config.set_native_dry_run(&context, choice.dry_run)
                });
        self.status = Some(match saved {
            Ok(_) => Status::Info(backend_name(choice)),
            Err(error) => Status::Error(fl!("backend-save-failed", reason = error.to_string())),
        });
        if switched {
            self.dry_run_plan = None;
            return self.start_list();
        }
        Task::none()
    }

    /// Esc cancels a prompt, then closes the overlay (a restore plan goes back to the
    /// browser), then the popup (or the window).
    fn escape(&mut self) -> Task<cosmic::Action<Message>> {
        if self.prompt != Prompt::Command {
            self.prompt = Prompt::Command;
            self.status = None;
            // Esc also unfocused the `>` line.
            return focus_input();
        }
        match self.overlay {
            Overlay::RestorePlan => {
                self.overlay = Overlay::Browse;
                self.pending_restore = None;
                return focus_input();
            }
            Overlay::Browse => {
                self.overlay = Overlay::None;
                self.browser = None;
                self.pending_restore = None;
                return focus_input();
            }
            _ => {}
        }
        if self.overlay == Overlay::Settings
            && let SettingsLoad::Ready(view) = &mut self.settings
            && view.dirty()
        {
            // Unsaved changes: the first Esc warns, the second drops them.
            if !view.discard_armed {
                view.discard_armed = true;
                self.status = Some(Status::Info(fl!("settings-unsaved")));
                return focus_input();
            }
            view.edited = view.saved();
            view.discard_armed = false;
            self.status = None;
        }
        if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
            // Esc also unfocused the `>` line.
            return focus_input();
        }
        match self.popup.take() {
            Some(popup) => {
                self.reset_popup_state();
                self.close_surface(popup)
            }
            None => Task::none(),
        }
    }

    fn toggle_overlay(&mut self, overlay: Overlay) {
        self.overlay = if self.overlay == overlay {
            Overlay::None
        } else {
            overlay
        };
    }

    /// Displayed snapshots, newest first; empty unless a list succeeded.
    fn snapshots(&self) -> &[Snapshot] {
        match &self.listing {
            Listing::Loaded(list) => &list.snapshots,
            Listing::NotLoaded | Listing::Failed(_) => &[],
        }
    }

    fn tooltip(&self) -> String {
        match &self.listing {
            Listing::Loaded(list) => match list.snapshots.first() {
                Some(newest) => fl!(
                    "tooltip-last",
                    ago = fmt::ago(newest.created, jiff::Zoned::now().datetime())
                ),
                None => fl!("tooltip-none"),
            },
            Listing::NotLoaded | Listing::Failed(_) => fl!("app-title"),
        }
    }
}

// View helpers. Everything is monotext; colours come from the theme.
impl AppModel {
    /// The popup's contents: header, panes, activity, `>` line and hints. The popup wraps it in a
    /// popup container; in window mode it fills the window.
    fn surface(&self) -> Element<'_, Message> {
        // The details pane is active after Enter or a double-click; otherwise the left pane is.
        let details_active = self.overlay == Overlay::Details;
        let panes = widget::row::with_children(vec![
            pane(
                self.body_title(),
                !details_active,
                Length::FillPortion(3),
                self.body(),
            ),
            pane(
                fl!("pane-details"),
                details_active,
                Length::FillPortion(2),
                self.details(),
            ),
        ])
        .spacing(8);
        widget::column::with_children(vec![
            self.header(),
            panes.into(),
            self.activity(),
            self.prompt(),
            self.hints(),
        ])
        .spacing(6)
        .padding([10, 12])
        .into()
    }

    /// ` ~/apsis $ ls --snapshots                 rsync · 3 snapshots`
    fn header(&self) -> Element<'_, Message> {
        let summary = match &self.listing {
            Listing::Loaded(list) if self.config.native_backend => fl!(
                "summary-native",
                summary = fmt::summary(list.mode, list.snapshots.len())
            ),
            Listing::Loaded(list) => fmt::summary(list.mode, list.snapshots.len()),
            Listing::NotLoaded | Listing::Failed(_) => String::new(),
        };
        widget::row::with_children(vec![
            icon::icon(self.icon.clone())
                .size(HEADER_ICON_SIZE)
                .class(theme::Svg::Custom(Rc::new(accent_svg)))
                .into(),
            monotext("~/apsis").class(theme::Text::Accent).into(),
            monotext("$ ls --snapshots").into(),
            widget::space::horizontal().into(),
            monotext(summary)
                .class(theme::Text::Custom(dim_text))
                .into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// The left pane's title: what it shows.
    fn body_title(&self) -> String {
        match self.overlay {
            Overlay::Help => fl!("pane-help"),
            Overlay::About => fl!("pane-about"),
            Overlay::DryRun => fl!("pane-dry-run"),
            Overlay::Browse => self.browse_title(),
            Overlay::RestorePlan if self.pending_restore.is_some() => fl!("pane-restore-plan"),
            Overlay::RestorePlan => fl!("pane-restore-result"),
            Overlay::Settings => match &self.settings {
                SettingsLoad::Ready(view) if view.dirty() => fl!("pane-settings-unsaved"),
                _ => fl!("pane-settings"),
            },
            Overlay::None | Overlay::Details => fl!("pane-snapshots"),
        }
    }

    /// Height of the snapshots and details panes' contents, in rows: the list's length between
    /// [`MIN_ROWS`] and [`VISIBLE_ROWS`] (longer lists scroll), or enough for what's shown instead.
    fn pane_rows(&self) -> u16 {
        match (self.overlay, &self.listing) {
            (
                Overlay::Help
                | Overlay::Settings
                | Overlay::DryRun
                | Overlay::Browse
                | Overlay::RestorePlan,
                _,
            )
            | (Overlay::None | Overlay::Details, Listing::Failed(_)) => VISIBLE_ROWS,
            (Overlay::About, _) => MIN_ROWS + 1,
            (_, Listing::Loaded(list)) => u16::try_from(list.snapshots.len())
                .unwrap_or(u16::MAX)
                .clamp(MIN_ROWS, VISIBLE_ROWS),
            (_, Listing::NotLoaded) => MIN_ROWS,
        }
    }

    /// How much of a comment a snapshot row shows before the pane width cuts it.
    fn row_comment_chars(&self) -> usize {
        match self.mode {
            Mode::Applet => ROW_COMMENT_CHARS,
            Mode::Window => MAX_COMMENT_CHARS,
        }
    }

    /// In window mode the panes fill the window's height instead.
    fn pane_height(&self) -> Length {
        match self.mode {
            Mode::Applet => Length::Fixed(ROW_HEIGHT * f32::from(self.pane_rows())),
            Mode::Window => Length::Fill,
        }
    }

    /// The left pane: the list (or why there is none), help or About.
    fn body(&self) -> Element<'_, Message> {
        let content = match (self.overlay, &self.listing) {
            (Overlay::Settings, _) => self.settings_body(),
            (Overlay::Help, _) => scroll(help()),
            (Overlay::About, _) => scroll(about()),
            (Overlay::DryRun, _) => scroll(dry_run_view(self.dry_run_plan.as_deref())),
            (Overlay::Browse, _) => self.browse_body(),
            (Overlay::RestorePlan, _) => scroll(dry_run_view(self.restore_text.as_deref())),
            (_, Listing::Loaded(list)) if list.snapshots.is_empty() => {
                if list.device.is_none() {
                    lines([fl!("no-device"), fl!("no-device-hint")])
                } else {
                    lines([fl!("empty")])
                }
            }
            (_, Listing::Loaded(list)) => self.list(&list.snapshots),
            (_, Listing::Failed(error)) => scroll(error_view(error, self.known_uuid.as_deref())),
            (_, Listing::NotLoaded) if self.loading => lines([fl!("waiting")]),
            (_, Listing::NotLoaded) => lines([fl!("not-loaded")]),
        };
        container(content)
            .width(Length::Fill)
            .height(self.pane_height())
            .into()
    }

    /// The right pane: everything about the selected snapshot.
    fn details(&self) -> Element<'_, Message> {
        let content = match (self.overlay, self.snapshots().get(self.selected)) {
            (Overlay::Settings, _) => scroll(self.settings_details()),
            (Overlay::Browse | Overlay::RestorePlan, _) => {
                match self.browser.as_ref().and_then(|b| Some((b, b.current()?))) {
                    Some((browser, entry)) => scroll(entry_details(browser, entry)),
                    None => lines([]),
                }
            }
            (_, Some(snapshot)) => scroll(details(snapshot)),
            (_, None) => widget::column::with_children(vec![
                monotext(fl!("details-none"))
                    .class(theme::Text::Custom(dim_text))
                    .into(),
            ])
            .padding([4, 6])
            .into(),
        };
        container(content)
            .width(Length::Fill)
            .height(self.pane_height())
            .into()
    }

    /// What the activity pane says: the running create or delete, else how the last one went.
    fn activity_line(&self) -> (String, Tone) {
        let spinner = SPINNER[self.spinner];
        match (&self.running, &self.status) {
            (Some(Operation::Create(_)), _) => {
                (format!("{} {spinner}", fl!("creating")), Tone::Normal)
            }
            (Some(Operation::Delete(name)), _) => (
                format!("{} {spinner}", fl!("deleting", name = name.clone())),
                Tone::Normal,
            ),
            (Some(Operation::DryRun(_)), _) => {
                (format!("{} {spinner}", fl!("dry-running")), Tone::Normal)
            }
            (Some(Operation::Restore(request)), _) if request.dry_run => (
                format!("{} {spinner}", fl!("restore-dry-running")),
                Tone::Normal,
            ),
            (Some(Operation::Restore(_)), _) => {
                (format!("{} {spinner}", fl!("restoring")), Tone::Normal)
            }
            (None, _) if self.saving_settings => (
                format!("{} {spinner}", fl!("settings-saving")),
                Tone::Normal,
            ),
            (None, _)
                if self.overlay == Overlay::Settings
                    && matches!(self.settings, SettingsLoad::Loading { .. }) =>
            {
                (
                    format!("{} {spinner}", fl!("settings-reading")),
                    Tone::Normal,
                )
            }
            (None, Some(Status::Info(text))) => (text.clone(), Tone::Dim),
            (None, Some(Status::Error(text))) => (text.clone(), Tone::Error),
            (None, None) => (fl!("activity-idle"), Tone::Dim),
        }
    }

    /// What Timeshift complained about in the last good list (e.g. a stale mount), for the
    /// activity pane.
    fn list_warnings(&self) -> &[String] {
        match &self.listing {
            Listing::Loaded(list) => &list.warnings,
            Listing::NotLoaded | Listing::Failed(_) => &[],
        }
    }

    /// The activity pane, active (accent border) while a create or delete runs: the current or
    /// last create/delete, then the last list's warnings.
    fn activity(&self) -> Element<'_, Message> {
        let (text, tone) = self.activity_line();
        let line = monotext(text).wrapping(Wrapping::WordOrGlyph);
        let line = match tone {
            Tone::Normal => line,
            Tone::Dim => line.class(theme::Text::Custom(dim_text)),
            Tone::Error => line.class(theme::Text::Custom(error_text)),
        };
        let mut lines = vec![line.into()];
        lines.extend(self.list_warnings().iter().map(|warning| {
            monotext(fl!("activity-list-warning", warning = warning.clone()))
                .wrapping(Wrapping::WordOrGlyph)
                .class(theme::Text::Custom(warning_text))
                .into()
        }));
        if self.overlay == Overlay::Settings
            && let SettingsLoad::Ready(view) = &self.settings
            && view.info.timeshift_gui_open
        {
            lines.push(
                monotext(fl!("settings-gui-open"))
                    .wrapping(Wrapping::WordOrGlyph)
                    .class(theme::Text::Custom(warning_text))
                    .into(),
            );
        }
        pane(
            fl!("pane-activity"),
            self.running.is_some() || self.saving_settings,
            Length::Fill,
            widget::column::with_children(lines)
                .spacing(2)
                .padding([0, 6]),
        )
    }

    fn list<'a>(&'a self, snapshots: &'a [Snapshot]) -> Element<'a, Message> {
        let rows = snapshots
            .iter()
            .enumerate()
            .map(|(index, snapshot)| self.row(index, snapshot));
        widget::scrollable(widget::column::with_children(rows))
            .id(LIST_ID.clone())
            .padding(0.0)
            .direction(Direction::Vertical(thin_scrollbar()))
            .height(Length::Shrink)
            .into()
    }

    /// `▸ 2026-09-25 03:00   D     "comment"`
    fn row<'a>(&self, index: usize, snapshot: &'a Snapshot) -> Element<'a, Message> {
        let selected = index == self.selected;
        let text = format!(
            "{}   {:<4}  {}",
            fmt::when(snapshot.created),
            fmt::tag_letters(&snapshot.tags),
            fmt::truncate(&fmt::quoted_comment(snapshot), self.row_comment_chars()),
        );
        let line = widget::row::with_children(vec![
            monotext(if selected { "▸" } else { " " })
                .class(theme::Text::Accent)
                .into(),
            // One line, cut to the pane width.
            monotext(text)
                .width(Length::Fill)
                .wrapping(Wrapping::None)
                .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                .into(),
        ])
        .spacing(8);
        let mut row = container(line)
            .width(Length::Fill)
            .height(Length::Fixed(ROW_HEIGHT))
            .padding([2, 6]);
        if selected {
            row = row.class(theme::Container::custom(selected_row));
        }
        widget::mouse_area(row)
            .on_press(Message::Select(index))
            .on_double_click(Message::OpenBrowser(index))
            .interaction(mouse::Interaction::Pointer)
            .into()
    }

    /// The footer: `[c]reate  [d]elete  [r]efresh  [?]help            [esc]`. Each hint is a
    /// button.
    fn hints(&self) -> Element<'_, Message> {
        match self.overlay {
            Overlay::Settings => return self.settings_hints(),
            Overlay::Browse | Overlay::RestorePlan => return self.browse_hints(),
            _ => {}
        }
        let refresh = if matches!(self.listing, Listing::Failed(_)) {
            "[r]etry"
        } else {
            "[r]efresh"
        };
        let idle = !self.loading && self.running.is_none();
        widget::row::with_children(vec![
            hint(
                "[c]reate",
                self.can_create().then_some(Message::StartCreate),
            ),
            hint(
                "[d]elete",
                self.can_delete().then_some(Message::StartDelete),
            ),
            hint(refresh, idle.then_some(Message::Refresh)),
            hint("[s]ettings", Some(Message::OpenSettings)),
            hint("[?]help", Some(Message::ToggleHelp)),
            widget::space::horizontal().into(),
            hint("[esc]", Some(Message::Escape)),
        ])
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// The footer in the settings view: `[space]change [+] [-] [a]dd [x]remove [w]rite
    /// [r]eload            [esc]`.
    fn settings_hints(&self) -> Element<'_, Message> {
        let ready = matches!(self.settings, SettingsLoad::Ready(_)) && !self.saving_settings;
        let key = |label, action| hint(label, ready.then_some(Message::SettingsKey(action)));
        let reload =
            !self.saving_settings && !matches!(self.settings, SettingsLoad::Loading { .. });
        widget::row::with_children(vec![
            key("[space]change", KeyAction::Toggle),
            key("[+]", KeyAction::More),
            key("[-]", KeyAction::Less),
            key("[a]dd", KeyAction::Add),
            key("[x]remove", KeyAction::Remove),
            key("[w]rite", KeyAction::Write),
            hint(
                "[r]eload",
                reload.then_some(Message::SettingsKey(KeyAction::Refresh)),
            ),
            widget::space::horizontal().into(),
            hint("[esc]", Some(Message::Escape)),
        ])
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// The settings view's left pane: one row per setting, or why there are none.
    fn settings_body(&self) -> Element<'_, Message> {
        match &self.settings {
            SettingsLoad::NotLoaded | SettingsLoad::Loading { .. } => {
                lines([fl!("settings-reading")])
            }
            SettingsLoad::Failed(reason) => scroll(
                widget::column::with_children(vec![
                    monotext(fl!("settings-read-failed"))
                        .class(theme::Text::Custom(error_text))
                        .into(),
                    monotext(reason.clone())
                        .wrapping(Wrapping::WordOrGlyph)
                        .into(),
                ])
                .spacing(2)
                .padding([4, 6]),
            ),
            SettingsLoad::Ready(view) => {
                let rows = view.rows();
                let lines = rows.iter().enumerate().map(|(index, &row)| {
                    let first = index == 0 || rows[index - 1].section() != row.section();
                    settings_row(view, index, row, first)
                });
                widget::scrollable(widget::column::with_children(lines))
                    .id(SETTINGS_LIST_ID.clone())
                    .padding(0.0)
                    .direction(Direction::Vertical(thin_scrollbar()))
                    .height(Length::Shrink)
                    .into()
            }
        }
    }

    /// The settings view's right pane: what the selected row means.
    fn settings_details(&self) -> Element<'_, Message> {
        let SettingsLoad::Ready(view) = &self.settings else {
            return lines([]);
        };
        let row = view.current();
        let mut pairs: Vec<(String, String)> = Vec::new();
        let note = match row {
            Row::Device => match view.device() {
                Some(device) => {
                    pairs.extend([
                        (fl!("settings-key-path"), device.path()),
                        (fl!("settings-key-type"), device.fstype.clone()),
                        (fl!("settings-key-size"), fmt::size(device.size)),
                        (fl!("settings-key-label"), device.label.clone()),
                        (fl!("settings-key-uuid"), device.uuid.clone()),
                    ]);
                    fl!("settings-device-note")
                }
                None if view.edited.backup_device_uuid.is_empty() => fl!("settings-device-none"),
                None => {
                    let uuid = view.edited.backup_device_uuid.clone();
                    pairs.push((fl!("settings-key-uuid"), uuid));
                    fl!("settings-device-missing-note")
                }
            },
            Row::Mode if view.edited.btrfs_mode => fl!("settings-mode-btrfs"),
            Row::Mode if view.btrfs_available() => fl!("settings-mode-rsync"),
            Row::Mode => fl!("settings-mode-rsync-only"),
            Row::BtrfsHome => fl!("settings-btrfs-home-note"),
            Row::Schedule(level) => {
                pairs.push((
                    fl!("settings-key-keep"),
                    view.edited.count(level).to_string(),
                ));
                fl!("settings-schedule-note", level = level.name())
            }
            Row::Home(index) => {
                let user = view.user(index);
                pairs.extend([
                    (fl!("settings-key-user"), user.name.clone()),
                    (fl!("settings-key-home"), user.home.clone()),
                    (
                        fl!("settings-key-backup"),
                        home_state_name(view.home_state(index)),
                    ),
                ]);
                if user.encrypted_home {
                    fl!("settings-home-encrypted")
                } else {
                    fl!("settings-home-note")
                }
            }
            Row::Filter(index) => {
                let pattern = &view.edited.exclude[index];
                let kind = if pattern.starts_with("+ ") {
                    fl!("settings-filter-include")
                } else {
                    fl!("settings-filter-exclude")
                };
                pairs.push((fl!("settings-key-kind"), kind));
                fl!("settings-filter-note")
            }
            Row::AddFilter => fl!("settings-add-note"),
            Row::Backend => fl!("settings-backend-note"),
            Row::DryRun => fl!("settings-dry-run-note"),
        };
        let mut rows: Vec<Element<'_, Message>> = pairs
            .into_iter()
            .map(|(key, value)| {
                let value = monotext(value).wrapping(Wrapping::WordOrGlyph);
                key_value_row(key, value.into(), DETAILS_KEY_WIDTH)
            })
            .collect();
        rows.push(
            monotext(note)
                .wrapping(Wrapping::WordOrGlyph)
                .class(theme::Text::Custom(dim_text))
                .into(),
        );
        widget::column::with_children(rows)
            .spacing(4)
            .padding([4, 6])
            .into()
    }

    /// The browser's pane title: `2026-09-25_03-00-01:/etc · 3 marked`.
    fn browse_title(&self) -> String {
        let Some(browser) = &self.browser else {
            return fl!("pane-snapshots");
        };
        let max = match self.mode {
            Mode::Applet => BREADCRUMB_CHARS,
            Mode::Window => WINDOW_BREADCRUMB_CHARS,
        };
        let title = browser.breadcrumb(max);
        if browser.marked.is_empty() {
            return title;
        }
        let marked = fl!("browse-marked", count = browser.marked.len().to_string());
        format!("{title} · {marked}")
    }

    /// The browser's left pane: the folder's entries, or why there are none.
    fn browse_body(&self) -> Element<'_, Message> {
        let Some(browser) = &self.browser else {
            return lines([]);
        };
        match &browser.load {
            Load::Loading => lines([fl!("browse-reading", path = browser.path.to_string())]),
            Load::Failed(reason) => scroll(
                widget::column::with_children(vec![
                    monotext(fl!("browse-failed"))
                        .class(theme::Text::Custom(error_text))
                        .into(),
                    monotext(reason.clone())
                        .wrapping(Wrapping::WordOrGlyph)
                        .into(),
                ])
                .spacing(2)
                .padding([4, 6]),
            ),
            Load::Ready(listing) if listing.entries.is_empty() => lines([fl!("browse-empty")]),
            Load::Ready(listing) => {
                let mut rows: Vec<Element<'_, Message>> = listing
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| browse_row(browser, index, entry))
                    .collect();
                if listing.truncated {
                    rows.push(
                        monotext(fl!("browse-truncated"))
                            .class(theme::Text::Custom(dim_text))
                            .into(),
                    );
                }
                widget::scrollable(widget::column::with_children(rows))
                    .id(BROWSE_LIST_ID.clone())
                    .padding(0.0)
                    .direction(Direction::Vertical(thin_scrollbar()))
                    .height(Length::Shrink)
                    .into()
            }
        }
    }

    /// The footer in the browser: `[space]mark [R]estore [h]up [r]eload            [esc]`, or
    /// on a plan `[enter]run            [esc]`.
    fn browse_hints(&self) -> Element<'_, Message> {
        let idle = self.running.is_none();
        let key = |label, action, on: bool| {
            hint(label, (on && idle).then_some(Message::BrowseKey(action)))
        };
        let mut hints = if self.overlay == Overlay::RestorePlan {
            vec![key(
                "[enter]run",
                KeyAction::Details,
                self.pending_restore.is_some(),
            )]
        } else {
            let has_entry = self.browser.as_ref().is_some_and(|b| b.current().is_some());
            vec![
                key("[space]mark", KeyAction::Toggle, has_entry),
                key("[R]estore", KeyAction::Restore, has_entry),
                key("[h]up", KeyAction::Back, true),
                key("[r]eload", KeyAction::Refresh, true),
            ]
        };
        hints.push(widget::space::horizontal().into());
        hints.push(hint("[esc]", Some(Message::Escape)));
        widget::row::with_children(hints)
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
    }

    /// The right-click menu: a standard COSMIC applet menu, not the terminal look.
    fn menu_view(&self) -> Element<'_, Message> {
        let content = widget::column::with_children(vec![
            menu_button(body(fl!("menu-refresh")))
                .on_press(Message::MenuRefresh)
                .into(),
            menu_button(body(fl!("menu-settings")))
                .on_press(Message::MenuSettings)
                .into(),
            menu_button(body(fl!("menu-about")))
                .on_press(Message::MenuAbout)
                .into(),
            padded_control(widget::divider::horizontal::default()).into(),
            menu_button(body(fl!("menu-panel-settings")))
                .on_press(Message::MenuPanelSettings)
                .into(),
        ])
        .padding([8, 0]);
        self.core
            .applet
            .popup_container(content)
            .limits(
                Limits::NONE
                    .min_width(MENU_WIDTH)
                    .max_width(MENU_WIDTH)
                    .min_height(1.0)
                    .max_height(1000.0),
            )
            .into()
    }

    /// The `>` input line: a focused text input. As a command line it stays empty, and its
    /// placeholder shows a running list (`timeshift --list ⠹`); a running create or delete is
    /// shown in the activity pane. For a create or delete it asks, after a label, for the comment
    /// or the `y`.
    ///
    /// Always the same three widgets (`>`, label, input) in the same places, so switching modes
    /// never moves the input to a new spot in the widget tree, where it would get a new state
    /// and lose focus.
    fn prompt(&self) -> Element<'_, Message> {
        let spinner = SPINNER[self.spinner];
        // While a create or delete runs the line stays empty; the activity pane shows it.
        let prompt = if self.running.is_some() {
            &Prompt::Command
        } else {
            &self.prompt
        };
        let (label, value, placeholder) = match prompt {
            Prompt::Comment(comment) => {
                (Some(fl!("prompt-comment")), comment.as_str(), String::new())
            }
            Prompt::ConfirmDelete { name, typed } => (
                Some(fl!("prompt-delete", name = name.clone())),
                typed.as_str(),
                String::new(),
            ),
            Prompt::Filter(pattern) => {
                (Some(fl!("prompt-filter")), pattern.as_str(), String::new())
            }
            Prompt::Count { level, typed } => (
                Some(fl!("prompt-count", level = level.name())),
                typed.as_str(),
                String::new(),
            ),
            Prompt::RestoreWhere { count, typed } => (
                Some(fl!("prompt-restore-where", count = count.to_string())),
                typed.as_str(),
                String::new(),
            ),
            Prompt::ConfirmRestore { count, typed } => (
                Some(fl!("prompt-restore-confirm", count = count.to_string())),
                typed.as_str(),
                String::new(),
            ),
            Prompt::Command if self.loading => (None, "", format!("timeshift --list {spinner}")),
            Prompt::Command => (None, "", String::new()),
        };
        let input = widget::text_input::inline_input(placeholder, value)
            .id(INPUT_ID.clone())
            .always_active()
            .font(cosmic::font::mono())
            .size(14.0)
            .line_height(iced_text::LineHeight::Absolute(20.0.into()))
            .padding(0)
            .on_input(Message::Input)
            .on_submit(|_| Message::Submit)
            // Esc and Tab unfocus it and leave it read-only (libcosmic); take focus straight back.
            .on_unfocus(Message::InputUnfocused);
        // The label's spaces stand in for row spacing, so the row is the same in every mode.
        let label = label.map_or_else(|| " ".to_owned(), |label| format!(" {label} "));
        widget::row::with_children(vec![
            monotext(">").class(theme::Text::Accent).into(),
            monotext(label).into(),
            input.into(),
        ])
        .align_y(Alignment::Center)
        .into()
    }
}

/// `▸ schedule  [x] daily    keep 5`: one settings row, the section name on the first row of
/// each section. Click selects, double-click changes it like space.
fn settings_row<'a>(
    view: &SettingsView,
    index: usize,
    row: Row,
    first: bool,
) -> Element<'a, Message> {
    let selected = index == view.cursor;
    let section = if first {
        section_name(row.section())
    } else {
        String::new()
    };
    let (text, dim) = settings_row_text(view, row);
    let text = monotext(text)
        .width(Length::Fill)
        .wrapping(Wrapping::None)
        .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
    let text = if dim {
        text.class(theme::Text::Custom(dim_text))
    } else {
        text
    };
    let line = widget::row::with_children(vec![
        monotext(if selected { "▸" } else { " " })
            .class(theme::Text::Accent)
            .into(),
        monotext(section)
            .width(Length::Fixed(SETTINGS_KEY_WIDTH))
            .class(theme::Text::Custom(dim_text))
            .into(),
        text.into(),
    ])
    .spacing(8);
    let mut line = container(line)
        .width(Length::Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .padding([2, 6]);
    if selected {
        line = line.class(theme::Container::custom(selected_row));
    }
    widget::mouse_area(line)
        .on_press(Message::SettingsSelect(index))
        .on_double_click(Message::SettingsActivate(index))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// A settings row's text, and whether it's dimmed (off, or nothing set).
fn settings_row_text(view: &SettingsView, row: Row) -> (String, bool) {
    let check = |on: bool| if on { "[x]" } else { "[ ]" };
    match row {
        Row::Device => match view.device() {
            Some(device) => {
                let mut parts = vec![
                    device.name.clone(),
                    device.fstype.clone(),
                    fmt::size(device.size),
                ];
                if !device.label.is_empty() {
                    parts.push(device.label.clone());
                }
                (parts.join("  "), false)
            }
            None if view.edited.backup_device_uuid.is_empty() => {
                (fl!("settings-device-unset"), true)
            }
            None => (
                fl!(
                    "settings-device-away",
                    id = fmt::short_uuid(&view.edited.backup_device_uuid)
                ),
                true,
            ),
        },
        Row::Mode if view.edited.btrfs_mode => ("btrfs".to_owned(), false),
        Row::Mode if view.btrfs_available() => ("rsync".to_owned(), false),
        Row::Mode => (fl!("settings-rsync-only"), false),
        Row::BtrfsHome => (
            format!(
                "{} {}",
                check(view.edited.include_btrfs_home),
                fl!("settings-btrfs-home")
            ),
            false,
        ),
        Row::Schedule(level) => (
            format!(
                "{} {:<8} {}",
                check(view.edited.scheduled(level)),
                level.name(),
                fl!("settings-keep", count = view.edited.count(level))
            ),
            !view.edited.scheduled(level),
        ),
        Row::Home(index) => {
            let user = view.user(index);
            let state = view.home_state(index);
            let encrypted = if user.encrypted_home {
                format!("  {}", fl!("settings-encrypted"))
            } else {
                String::new()
            };
            let text = format!("{:<10} {}{encrypted}", user.name, home_state_name(state));
            (text, state == HomeState::Excluded)
        }
        Row::Filter(index) => (view.edited.exclude[index].clone(), false),
        Row::AddFilter => (fl!("settings-add-filter"), true),
        Row::Backend if view.backend.native => (fl!("settings-backend-native"), false),
        Row::Backend => (fl!("settings-backend-timeshift"), false),
        Row::DryRun => (
            format!(
                "{} {}",
                check(view.backend.dry_run),
                fl!("settings-dry-run")
            ),
            false,
        ),
    }
}

fn section_name(section: Section) -> String {
    match section {
        Section::Device => fl!("settings-device"),
        Section::Mode => fl!("settings-mode"),
        Section::Schedule => fl!("settings-schedule"),
        Section::Home => fl!("settings-home"),
        Section::Filters => fl!("settings-filters"),
        Section::Apsis => fl!("settings-apsis"),
    }
}

fn home_state_name(state: HomeState) -> String {
    match state {
        HomeState::Excluded => fl!("settings-home-excluded"),
        HomeState::Hidden => fl!("settings-home-hidden"),
        HomeState::All => fl!("settings-home-all"),
    }
}

fn hint(label: &'static str, on_press: Option<Message>) -> Element<'static, Message> {
    widget::button::custom(monotext(label))
        .class(theme::Button::Text)
        .padding([2, 4])
        .on_press_maybe(on_press)
        .into()
}

/// A rounded pane with `title` set into its top border, superfile style. The active pane's
/// border and title are in the accent colour, the others use the divider colour.
///
/// ```text
/// ╭─ title ─────╮   top row: corner piece, title, top-right piece
/// │ content     │   body: sides and bottom
/// ╰─────────────╯
/// ```
///
/// Every piece is a fill in the border colour holding a fill in the popup's background colour,
/// inset by [`LINE`] on the sides that show a line. So the pane is opaque in the theme's
/// background and its corners use the theme's radius. Everything is drawn in the popup's own
/// layer (no `Stack`).
fn pane<'a>(
    title: String,
    active: bool,
    width: Length,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let title = monotext(title).class(if active {
        theme::Text::Accent
    } else {
        theme::Text::Custom(dim_text)
    });
    // The pieces are half a line high and sit at the bottom, so the line runs through the
    // middle of the title.
    let top = widget::row::with_children(vec![
        edge_piece(Edge::TopLeft, active, Length::Fixed(TITLE_INSET)),
        container(title).padding([0, 4]).into(),
        edge_piece(Edge::TopRight, active, Length::Fill),
    ])
    .align_y(Alignment::End);
    let inner = container(content)
        .width(Length::Fill)
        .padding(Padding {
            top: 4.0,
            right: 6.0,
            bottom: 6.0,
            left: 6.0,
        })
        .class(theme::Container::custom(|theme| {
            pane_fill(theme, Edge::Bottom)
        }));
    let body = container(inner)
        .width(Length::Fill)
        .padding(Edge::Bottom.line_padding())
        .class(theme::Container::custom(move |theme| {
            pane_line(theme, Edge::Bottom, active)
        }));
    widget::column::with_children(vec![top.into(), body.into()])
        .width(width)
        .into()
}

/// A top piece of a pane's border: half a line high, a line along its top and one outer side.
fn edge_piece<'a>(edge: Edge, active: bool, width: Length) -> Element<'a, Message> {
    let inner = container(widget::space::horizontal())
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::custom(move |theme| {
            pane_fill(theme, edge)
        }));
    container(inner)
        .width(width)
        .height(Length::Fixed(TITLE_HALF))
        .padding(edge.line_padding())
        .class(theme::Container::custom(move |theme| {
            pane_line(theme, edge, active)
        }))
        .into()
}

/// Pane content that may be taller than the pane.
fn scroll<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    widget::scrollable(content)
        .direction(Direction::Vertical(thin_scrollbar()))
        .height(Length::Fill)
        .into()
}

fn thin_scrollbar() -> Scrollbar {
    Scrollbar::new().width(4.0).scroller_width(4.0).spacing(4.0)
}

fn lines<'a>(lines: impl IntoIterator<Item = String>) -> Element<'a, Message> {
    widget::column::with_children(lines.into_iter().map(|l| monotext(l).into()))
        .spacing(2)
        .padding([4, 6])
        .into()
}

/// The list's error state. `known_uuid` names the disk if it's gone missing.
fn error_view<'a>(error: &'a CliError, known_uuid: Option<&str>) -> Element<'a, Message> {
    let mut out = Vec::new();
    match error {
        CliError::NotInstalled => out.push(fl!("not-installed")),
        CliError::NotAuthorized => out.push(fl!("failed-auth")),
        CliError::DeviceNotFound { device } => out.push(disk_missing(device, known_uuid)),
        CliError::Failed { code, output } => {
            out.push(match code {
                Some(code) => fl!("failed-code", code = code.to_string()),
                None => fl!("failed-signal"),
            });
            if pkexec_refused(*code) {
                out.push(fl!("failed-auth"));
            }
            out.extend(output.iter().cloned());
        }
        CliError::Other(message) => out.push(fl!("failed-other", message = message.clone())),
    }
    let mut column = vec![
        monotext(fl!("error-label"))
            .class(theme::Text::Custom(error_text))
            .into(),
    ];
    column.extend(
        out.into_iter()
            .map(|l| monotext(l).class(theme::Text::Custom(error_text)).into()),
    );
    widget::column::with_children(column)
        .spacing(2)
        .padding([4, 6])
        .into()
}

/// Lists through `apsis-helper` when it's installed (no password for the active session),
/// else through pkexec (a password prompt each time). The native backend needs the helper.
async fn list_snapshots(pkexec: Arc<Cli>, native: bool) -> Result<SnapshotList, CliError> {
    if native {
        return native_helper()
            .await?
            .native_list()
            .await
            .map_err(CliError::from);
    }
    if let Some(helper) = HelperClient::connect().await {
        return helper.list().await.map_err(CliError::from);
    }
    blocking(move || pkexec.list()).await
}

/// `apsis-helper`, which the native backend always runs in (it needs root).
async fn native_helper() -> Result<HelperClient, CliError> {
    HelperClient::connect()
        .await
        .ok_or_else(|| CliError::Other(fl!("native-need-helper")))
}

/// The native backend's plan for a create with `comment`; nothing is written.
async fn native_dry_run(comment: &str) -> Result<String, CliError> {
    native_helper()
        .await?
        .native_dry_run(comment)
        .await
        .map_err(CliError::from)
}

/// Creates or deletes through `apsis-helper` when it's installed, else through pkexec. With
/// the helper this returns when its `Finished` signal arrives, however long Timeshift takes.
///
/// A helper that's installed but fails is reported, not replaced by pkexec, so a broken
/// install gets noticed.
///
/// With the native backend, a create is the helper's native create. A delete always goes to
/// Timeshift, which removes native snapshots like its own.
async fn operate(pkexec: Arc<Cli>, operation: Operation, native: bool) -> Result<(), CliError> {
    if native && let Operation::Create(comment) = &operation {
        return native_helper()
            .await?
            .native_create(comment)
            .await
            .map_err(CliError::from);
    }
    if let Some(helper) = HelperClient::connect().await {
        let done = match &operation {
            Operation::Create(comment) => helper.create(comment).await,
            Operation::Delete(name) => helper.delete(name).await,
            Operation::DryRun(comment) => return native_dry_run(comment).await.map(drop),
            Operation::Restore(request) => return restore(request).await.map(drop),
        };
        return done.map_err(CliError::from);
    }
    blocking(move || match &operation {
        Operation::Create(comment) => pkexec.create(comment),
        Operation::Delete(name) => pkexec.delete(name),
        Operation::DryRun(_) => Err(apsis_core::Error::Helper(fl!("native-need-helper"))),
        Operation::Restore(_) => Err(apsis_core::Error::Helper(fl!("restore-need-helper"))),
    })
    .await
}

/// `backend: native rsync, dry run` and the like, for the activity pane.
fn backend_name(choice: BackendChoice) -> String {
    match (choice.native, choice.dry_run) {
        (false, _) => fl!("backend-now-timeshift"),
        (true, true) => fl!("backend-now-native-dry-run"),
        (true, false) => fl!("backend-now-native"),
    }
}

/// The left pane after a native dry run: the plan, line by line.
fn dry_run_view(plan: Option<&str>) -> Element<'static, Message> {
    let lines: Vec<Element<'static, Message>> = plan
        .unwrap_or_default()
        .lines()
        .map(|line| {
            monotext(line.to_owned())
                .wrapping(Wrapping::WordOrGlyph)
                .into()
        })
        .collect();
    widget::column::with_children(lines)
        .spacing(2)
        .padding([4, 6])
        .into()
}

/// `▸ ~ * hosts                     1.2 KiB`: the running system's marker (`+` missing, `~`
/// changed, `=` same), the mark, the name (`/` after folders, `-> target` for links), the size.
/// Click selects, double-click goes into a folder or marks a file.
fn browse_row<'a>(browser: &Browser, index: usize, entry: &'a Entry) -> Element<'a, Message> {
    let selected = index == browser.cursor;
    let marker = monotext(browser::live_marker(entry.live).to_string());
    let marker = match entry.live {
        Live::Missing => marker.class(theme::Text::Accent),
        Live::Changed => marker.class(theme::Text::Custom(warning_text)),
        Live::Same | Live::Present => marker.class(theme::Text::Custom(dim_text)),
    };
    let name = match entry.kind {
        Kind::Dir => format!("{}/", entry.name),
        Kind::Link => format!("{} -> {}", entry.name, entry.target),
        Kind::File | Kind::Other => entry.name.clone(),
    };
    let size = if entry.kind == Kind::File {
        fmt::size(entry.size)
    } else {
        String::new()
    };
    let marked = browser.is_marked(entry);
    let line = widget::row::with_children(vec![
        monotext(if selected { "▸" } else { " " })
            .class(theme::Text::Accent)
            .into(),
        marker.into(),
        monotext(if marked { "*" } else { " " })
            .class(theme::Text::Accent)
            .into(),
        monotext(name)
            .width(Length::Fill)
            .wrapping(Wrapping::None)
            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
            .into(),
        monotext(size).class(theme::Text::Custom(dim_text)).into(),
    ])
    .spacing(8);
    let mut row = container(line)
        .width(Length::Fill)
        .height(Length::Fixed(ROW_HEIGHT))
        .padding([2, 6]);
    if selected {
        row = row.class(theme::Container::custom(selected_row));
    } else if marked {
        row = row.class(theme::Container::custom(marked_row));
    }
    widget::mouse_area(row)
        .on_press(Message::BrowseSelect(index))
        .on_double_click(Message::BrowseActivate(index))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// The browser's right pane: everything about the selected entry, and how the running system
/// compares.
fn entry_details(browser: &Browser, entry: &Entry) -> Element<'static, Message> {
    let path = browser
        .current_path()
        .map_or_else(|| entry.name.clone(), |p| p.to_string());
    let kind = match entry.kind {
        Kind::File => fl!("entry-file"),
        Kind::Dir => fl!("entry-folder"),
        Kind::Link => fl!("entry-link"),
        Kind::Other => fl!("entry-special"),
    };
    let mut fields = vec![
        (fl!("details-path"), path),
        (fl!("details-type"), kind),
        (fl!("details-size"), fmt::size(entry.size)),
        (fl!("details-modified"), local_time(entry.mtime)),
        (
            fl!("details-mode"),
            browser::mode_string(entry.kind, entry.mode),
        ),
        (fl!("details-owner"), entry.owner.clone()),
    ];
    if entry.kind == Kind::Link {
        fields.push((fl!("details-link"), entry.target.clone()));
    }
    fields.push((fl!("details-live"), live_text(entry)));
    if browser.is_marked(entry) {
        fields.push((fl!("details-restore"), fl!("details-marked")));
    }
    key_value_rows(fields, DETAILS_KEY_WIDTH)
}

/// How the running system compares, spelled out.
fn live_text(entry: &Entry) -> String {
    match entry.live {
        Live::Missing => fl!("live-missing"),
        Live::Same => fl!("live-same"),
        Live::Present => fl!("live-present"),
        Live::Changed if entry.kind == Kind::File && entry.live_size != 0 => fl!(
            "live-changed-file",
            size = fmt::size(entry.size),
            live_size = fmt::size(entry.live_size),
            time = local_time(entry.mtime),
            live_time = local_time(entry.live_mtime)
        ),
        Live::Changed => fl!("live-changed"),
    }
}

/// Unix seconds as local time, `2026-09-25 03:00:01`.
fn local_time(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds).map_or_else(
        |_| seconds.to_string(),
        |t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M:%S")
                .to_string()
        },
    )
}

/// `apsis-helper`, which browsing and restoring always need (they read root-only files).
async fn restore_helper() -> Result<HelperClient, CliError> {
    HelperClient::connect()
        .await
        .ok_or_else(|| CliError::Other(fl!("restore-need-helper")))
}

/// Folder `path` of `snapshot`, compared with the running system.
async fn browse(snapshot: &str, path: &str) -> Result<FolderListing, CliError> {
    restore_helper()
        .await?
        .browse(snapshot, path)
        .await
        .map_err(CliError::from)
}

/// Runs `request` (dry run or for real); the plan's or result's text.
async fn restore(request: &Request) -> Result<String, CliError> {
    restore_helper()
        .await?
        .restore(request)
        .await
        .map_err(CliError::from)
}

/// Timeshift's settings, the devices and the users, through `apsis-helper` (there's no pkexec
/// fallback: writing the file safely needs the helper).
async fn read_settings() -> Result<SettingsInfo, CliError> {
    let helper = HelperClient::connect()
        .await
        .ok_or_else(|| CliError::Other(fl!("settings-need-helper")))?;
    helper.read_settings().await.map_err(CliError::from)
}

/// Writes `settings` through `apsis-helper` if the file still reads `expected`.
async fn write_settings(expected: String, settings: Settings) -> Result<String, CliError> {
    let helper = HelperClient::connect()
        .await
        .ok_or_else(|| CliError::Other(fl!("settings-need-helper")))?;
    helper
        .write_settings(&expected, &settings)
        .await
        .map_err(CliError::from)
}

/// Scrolls list `id` so row `index` of `count` equal-height rows is in view: selected /
/// (count - 1) of the way down keeps it there, from the first row at the top to the last at
/// the bottom.
fn scroll_to(id: &widget::Id, index: usize, count: usize) -> Task<cosmic::Action<Message>> {
    #[allow(clippy::cast_precision_loss, reason = "a handful of rows")]
    let y = if count > 1 {
        index as f32 / (count - 1) as f32
    } else {
        0.0
    };
    snap_to(
        id.clone(),
        RelativeOffset {
            x: None,
            y: Some(y),
        },
    )
}

/// Runs blocking pkexec work off the UI's runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> apsis_core::Result<T> + Send + 'static,
) -> Result<T, CliError> {
    match tokio::task::spawn_blocking(work).await {
        Ok(done) => done.map_err(CliError::from),
        Err(join) => Err(CliError::Other(join.to_string())),
    }
}

/// pkexec: 126 = not authorised or dialog dismissed, 127 = couldn't authenticate. Timeshift
/// didn't run.
fn pkexec_refused(code: Option<i32>) -> bool {
    matches!(code, Some(126 | 127))
}

/// Why a create or delete failed, for the activity pane: Timeshift's last lines (one per
/// line), or what else went wrong.
fn error_summary(error: &CliError, known_uuid: Option<&str>) -> String {
    match error {
        CliError::NotInstalled => fl!("not-installed"),
        CliError::NotAuthorized => fl!("failed-auth"),
        CliError::DeviceNotFound { device } => disk_missing(device, known_uuid),
        CliError::Failed { code, .. } if pkexec_refused(*code) => fl!("failed-auth"),
        CliError::Failed { code, output } if output.is_empty() => match code {
            Some(code) => fl!("failed-code", code = code.to_string()),
            None => fl!("failed-signal"),
        },
        CliError::Failed { output, .. } => output.join("\n"),
        CliError::Other(message) => message.clone(),
    }
}

/// `backup disk not connected (UUID 1a2b…): plug it in and press r`. Names the disk by the
/// UUID from the last good list; Timeshift's own message often has a `/dev` name instead,
/// which changes when a USB disk reconnects.
fn disk_missing(device: &str, known_uuid: Option<&str>) -> String {
    let id = match known_uuid {
        Some(uuid) => format!("UUID {}", fmt::short_uuid(uuid)),
        None if !device.starts_with('/') => format!("UUID {}", fmt::short_uuid(device)),
        None => device.to_owned(),
    };
    fl!("disk-missing", id = id)
}

/// Full name, creation time, age, tags spelled out and the whole comment.
fn details(snapshot: &Snapshot) -> Element<'_, Message> {
    let comment = snapshot.comment.clone().unwrap_or_else(|| "-".to_owned());
    let fields = [
        (fl!("details-name"), snapshot.name.clone()),
        (
            fl!("details-created"),
            snapshot.created.strftime("%Y-%m-%d %H:%M:%S").to_string(),
        ),
        (
            fl!("details-age"),
            fmt::ago(snapshot.created, jiff::Zoned::now().datetime()),
        ),
        (fl!("details-tags"), fmt::tag_names(&snapshot.tags)),
        (fl!("details-comment"), comment),
    ];
    key_value_rows(fields, DETAILS_KEY_WIDTH)
}

fn help() -> Element<'static, Message> {
    let keys = [
        ("↑ ↓  j k", fl!("help-move")),
        ("Home End", fl!("help-ends")),
        ("Enter", fl!("help-browse")),
        ("Tab", fl!("help-details")),
        ("c", fl!("help-create")),
        ("d", fl!("help-delete")),
        ("r", fl!("help-refresh")),
        ("s", fl!("help-settings")),
        ("?", fl!("help-help")),
        ("Esc", fl!("help-escape")),
        ("", String::new()),
        ("space", fl!("help-settings-change")),
        ("+ - e", fl!("help-settings-count")),
        ("a x", fl!("help-settings-filters")),
        ("w", fl!("help-settings-write")),
        ("r", fl!("help-settings-reload")),
        ("", String::new()),
        ("Enter l", fl!("help-browse-into")),
        ("⌫ h", fl!("help-browse-up")),
        ("space", fl!("help-browse-mark")),
        ("J", fl!("help-browse-mark-down")),
        ("R", fl!("help-browse-restore")),
        ("r", fl!("help-browse-reload")),
    ];
    key_value_rows(keys.map(|(k, v)| (k.to_owned(), v)), HELP_KEY_WIDTH)
}

/// ```text
/// Apsis 0.1.0
/// Timeshift-style system snapshots for the COSMIC™ desktop
///
/// license   GPL-3.0-only
/// source    https://github.com/atraxsrc/apsis
/// ```
fn about() -> Element<'static, Message> {
    let link = widget::button::custom(monotext(REPOSITORY))
        .class(theme::Button::Link)
        .padding(0)
        .on_press(Message::OpenRepository);
    widget::column::with_children(vec![
        widget::row::with_children(vec![
            monotext(fl!("app-title")).class(theme::Text::Accent).into(),
            monotext(VERSION).into(),
        ])
        .spacing(8)
        .into(),
        monotext(fl!("app-comment")).into(),
        widget::space::vertical().height(Length::Fixed(10.0)).into(),
        key_value_row(
            fl!("about-license"),
            monotext(LICENSE).into(),
            HELP_KEY_WIDTH,
        ),
        key_value_row(fl!("about-source"), link.into(), HELP_KEY_WIDTH),
    ])
    .spacing(2)
    .padding([4, 6])
    .into()
}

fn key_value_rows<'a>(
    rows: impl IntoIterator<Item = (String, String)>,
    key_width: f32,
) -> Element<'a, Message> {
    let rows = rows.into_iter().map(|(key, value)| {
        let value = monotext(value).wrapping(Wrapping::WordOrGlyph);
        key_value_row(key, value.into(), key_width)
    });
    widget::column::with_children(rows)
        .spacing(2)
        .padding([4, 6])
        .into()
}

fn key_value_row(key: String, value: Element<'_, Message>, key_width: f32) -> Element<'_, Message> {
    widget::row::with_children(vec![
        monotext(key)
            .class(theme::Text::Accent)
            .width(Length::Fixed(key_width))
            .into(),
        value,
    ])
    .spacing(8)
    .into()
}

/// The installed symbolic icon, or the embedded copy when the icon theme doesn't have it.
///
/// No name fallback: libcosmic would otherwise retry `io.github.atraxsrc.Apsis` (the full-colour
/// icon) when only the `-symbolic` one is missing.
fn symbolic_icon() -> icon::Handle {
    let named = icon::from_name(SYMBOLIC_ICON).fallback(None);
    if named.clone().path().is_some() {
        return named.symbolic(true).handle();
    }
    let mut handle = icon::from_svg_bytes(SYMBOLIC_ICON_SVG);
    handle.symbolic = true;
    handle
}

/// Starts `command` detached from the applet (double fork), so it outlives a panel restart and
/// leaves no zombie. A missing program is only logged.
fn spawn(command: std::process::Command) -> Task<cosmic::Action<Message>> {
    Task::future(async move {
        let program = command.get_program().to_owned();
        if cosmic::process::spawn(command).await.is_none() {
            eprintln!("apsis: could not start {}", program.display());
        }
    })
    .discard()
}

/// Focuses the `>` line, with the cursor after what's in it. Focusing also clears the
/// read-only state libcosmic leaves after Esc or Tab.
fn focus_input() -> Task<cosmic::Action<Message>> {
    widget::text_input::focus(INPUT_ID.clone())
        .chain(widget::text_input::move_cursor_to_end(INPUT_ID.clone()))
}

/// Command keys that are typed as characters.
fn char_action(c: char) -> Option<KeyAction> {
    match c {
        'k' => Some(KeyAction::Up),
        'j' => Some(KeyAction::Down),
        'r' => Some(KeyAction::Refresh),
        'c' => Some(KeyAction::Create),
        'd' => Some(KeyAction::Delete),
        '?' => Some(KeyAction::Help),
        's' => Some(KeyAction::Settings),
        ' ' => Some(KeyAction::Toggle),
        '+' | '=' => Some(KeyAction::More),
        '-' => Some(KeyAction::Less),
        'e' => Some(KeyAction::Edit),
        'a' => Some(KeyAction::Add),
        'x' => Some(KeyAction::Remove),
        'w' => Some(KeyAction::Write),
        'h' => Some(KeyAction::Back),
        'l' => Some(KeyAction::Into),
        'R' => Some(KeyAction::Restore),
        'J' => Some(KeyAction::MarkDown),
        _ => None,
    }
}

/// Maps key presses to [`KeyAction`]s. Ctrl/Alt/Super combinations are left alone.
///
/// Characters and Enter only count when no widget took the key (`Ignored`): when the `>` line
/// has focus it captures them and reports them through `on_input` / `on_submit` instead, so they
/// aren't handled twice. Arrows, Home/End and Esc count either way.
fn key_action(event: event::Event, status: event::Status, window: Id) -> Option<Message> {
    if *DEBUG_KEYS {
        log_focus_event(&event, window);
    }
    let event::Event::Keyboard(keyboard::Event::KeyPressed {
        modified_key,
        modifiers,
        ..
    }) = event
    else {
        return None;
    };
    if *DEBUG_KEYS {
        eprintln!("apsis: key {modified_key:?} {status:?} window {window:?}");
    }
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    let uncaptured = status == event::Status::Ignored;
    let action = match modified_key.as_ref() {
        Key::Named(Named::ArrowUp) => KeyAction::Up,
        Key::Named(Named::ArrowDown) => KeyAction::Down,
        Key::Named(Named::Home) => KeyAction::First,
        Key::Named(Named::End) => KeyAction::Last,
        Key::Named(Named::Escape) => KeyAction::Escape,
        // The `>` line captures these even when it's empty; they only act as commands when no
        // prompt is open (see `on_key`).
        Key::Named(Named::Backspace) => KeyAction::Back,
        Key::Named(Named::Tab) => KeyAction::FocusDetails,
        Key::Named(Named::Enter) if uncaptured => KeyAction::Details,
        Key::Character(c) if uncaptured => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => char_action(c)?,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(Message::Key(window, action))
}

/// Window mode: the main window got focus, so it's on screen.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the signature event::listen_with takes"
)]
fn window_focused(event: event::Event, _status: event::Status, _window: Id) -> Option<Message> {
    matches!(event, event::Event::Window(window::Event::Focused)).then_some(Message::WindowShown)
}

/// Debug aid: popup keyboard focus changes, as the Wayland backend reports them.
fn log_focus_event(event: &event::Event, window: Id) {
    use cosmic::iced::event::PlatformSpecific;
    use cosmic::iced::event::wayland::{Event as WaylandEvent, PopupEvent};
    if let event::Event::PlatformSpecific(PlatformSpecific::Wayland(WaylandEvent::Popup(
        popup_event @ (PopupEvent::Focused | PopupEvent::Unfocused),
        _,
        _,
    ))) = event
    {
        eprintln!("apsis: popup {popup_event:?} window {window:?}");
    }
}

/// A pane's border colour, as a fill: accent when active, else the theme's divider colour.
fn pane_line(theme: &Theme, edge: Edge, active: bool) -> container::Style {
    let cosmic = theme.cosmic();
    let color: Color = if active {
        cosmic.accent_color().into()
    } else {
        cosmic.background(theme.transparent).divider.into()
    };
    container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            radius: edge.radius(cosmic.corner_radii.radius_s[0]),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Inside a pane's border: the popup's background colour, rounded to fit inside the line.
fn pane_fill(theme: &Theme, edge: Edge) -> container::Style {
    let cosmic = theme.cosmic();
    let background = cosmic.background(theme.transparent).base;
    let radius = (cosmic.corner_radii.radius_s[0] - LINE).max(0.0);
    container::Style {
        background: Some(Background::Color(background.into())),
        border: Border {
            radius: edge.radius(radius),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Selected row: the accent colour, faded, as a background.
fn selected_row(theme: &Theme) -> container::Style {
    let cosmic = theme.cosmic();
    container::Style {
        background: Some(Background::Color(
            Color::from(cosmic.accent_color()).scale_alpha(0.15),
        )),
        border: Border {
            radius: cosmic.corner_radii.radius_s.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A marked browser row that isn't selected: a fainter accent background than the selection.
fn marked_row(theme: &Theme) -> container::Style {
    let cosmic = theme.cosmic();
    container::Style {
        background: Some(Background::Color(
            Color::from(cosmic.accent_color()).scale_alpha(0.07),
        )),
        border: Border {
            radius: cosmic.corner_radii.radius_s.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Symbolic icons in the accent colour, like the accent text beside them.
fn accent_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(theme.cosmic().accent_text_color().into()),
    }
}

/// Secondary text: the normal text colour, faded.
fn dim_text(theme: &Theme) -> iced_text::Style {
    let on = theme.cosmic().background(theme.transparent).on;
    text_style(theme, Color::from(on).scale_alpha(0.7))
}

/// Timeshift's warnings in the activity pane.
fn warning_text(theme: &Theme) -> iced_text::Style {
    text_style(theme, theme.cosmic().warning_text_color().into())
}

fn error_text(theme: &Theme) -> iced_text::Style {
    text_style(theme, theme.cosmic().destructive_text_color().into())
}

/// A text style in `color`, with the theme's usual selection colours.
fn text_style(theme: &Theme, color: Color) -> iced_text::Style {
    let cosmic = theme.cosmic();
    iced_text::Style {
        color: Some(color),
        selected_fill: cosmic.accent.base.into(),
        selected_text_color: Some(cosmic.on_accent_color().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::keyboard::key::{NativeCode, Physical};
    use cosmic::iced::keyboard::{Location, Modifiers};

    fn press(key: Key, modifiers: Modifiers) -> event::Event {
        event::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Unidentified(NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    fn action(key: Key, status: event::Status) -> Option<KeyAction> {
        let window = Id::unique();
        match key_action(press(key, Modifiers::empty()), status, window) {
            Some(Message::Key(id, action)) if id == window => Some(action),
            Some(other) => panic!("unexpected message {other:?}"),
            None => None,
        }
    }

    fn character(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn command_characters_map_to_actions() {
        assert_eq!(char_action('k'), Some(KeyAction::Up));
        assert_eq!(char_action('j'), Some(KeyAction::Down));
        assert_eq!(char_action('r'), Some(KeyAction::Refresh));
        assert_eq!(char_action('?'), Some(KeyAction::Help));
        assert_eq!(char_action('c'), Some(KeyAction::Create));
        assert_eq!(char_action('d'), Some(KeyAction::Delete));
        assert_eq!(char_action('s'), Some(KeyAction::Settings));
        assert_eq!(char_action(' '), Some(KeyAction::Toggle));
        assert_eq!(char_action('+'), Some(KeyAction::More));
        assert_eq!(char_action('-'), Some(KeyAction::Less));
        assert_eq!(char_action('e'), Some(KeyAction::Edit));
        assert_eq!(char_action('a'), Some(KeyAction::Add));
        assert_eq!(char_action('x'), Some(KeyAction::Remove));
        assert_eq!(char_action('w'), Some(KeyAction::Write));
        assert_eq!(char_action('h'), Some(KeyAction::Back));
        assert_eq!(char_action('l'), Some(KeyAction::Into));
        assert_eq!(char_action('R'), Some(KeyAction::Restore));
        assert_eq!(char_action('z'), None);
        assert_eq!(char_action('J'), Some(KeyAction::MarkDown));
        assert_eq!(char_action('K'), None);
    }

    #[test]
    fn uncaptured_keys_are_handled_from_the_event_stream() {
        let ignored = event::Status::Ignored;
        assert_eq!(action(character("j"), ignored), Some(KeyAction::Down));
        assert_eq!(action(character("?"), ignored), Some(KeyAction::Help));
        assert_eq!(
            action(Key::Named(Named::Enter), ignored),
            Some(KeyAction::Details)
        );
        assert_eq!(action(character(" "), ignored), Some(KeyAction::Toggle));
        assert_eq!(action(character("z"), ignored), None);
        assert_eq!(action(character("jj"), ignored), None);
    }

    #[test]
    fn keys_the_input_line_captured_are_not_handled_twice() {
        // The focused `>` line reports these through on_input / on_submit.
        let captured = event::Status::Captured;
        assert_eq!(action(character("j"), captured), None);
        assert_eq!(action(character("r"), captured), None);
        assert_eq!(action(Key::Named(Named::Enter), captured), None);
    }

    #[test]
    fn navigation_keys_count_even_when_captured() {
        for status in [event::Status::Ignored, event::Status::Captured] {
            let table = [
                (Named::ArrowUp, KeyAction::Up),
                (Named::ArrowDown, KeyAction::Down),
                (Named::Home, KeyAction::First),
                (Named::End, KeyAction::Last),
                (Named::Escape, KeyAction::Escape),
                (Named::Backspace, KeyAction::Back),
                (Named::Tab, KeyAction::FocusDetails),
            ];
            for (named, want) in table {
                assert_eq!(action(Key::Named(named), status), Some(want), "{named:?}");
            }
        }
    }

    /// An applet with a main window, nothing open, and a failed list (so opening the popup
    /// doesn't start one by itself). Tasks returned by `update` are never run.
    fn model() -> AppModel {
        let mut core = cosmic::Core::default();
        core.set_main_window_id(Some(Id::unique()));
        AppModel {
            core,
            mode: Mode::Applet,
            popup: None,
            menu: None,
            config: Config::default(),
            pkexec: Arc::new(TimeshiftCli::new(PkexecRunner)),
            listing: Listing::Failed(CliError::NotInstalled),
            known_uuid: None,
            loading: false,
            running: None,
            prompt: Prompt::Command,
            status: None,
            spinner: 0,
            selected: 0,
            overlay: Overlay::None,
            settings: SettingsLoad::NotLoaded,
            saving_settings: false,
            dry_run_plan: None,
            browser: None,
            pending_restore: None,
            restore_text: None,
            window_resizable: false,
            icon: symbolic_icon(),
        }
    }

    fn send(app: &mut AppModel, message: Message) {
        let _ = cosmic::Application::update(app, message);
    }

    #[test]
    fn right_click_swaps_the_popup_for_the_menu() {
        let mut app = model();
        send(&mut app, Message::TogglePopup);
        send(&mut app, Message::ToggleHelp);
        send(&mut app, Message::ToggleMenu);
        assert!(app.popup.is_none());
        assert!(app.menu.is_some());
        assert_eq!(app.overlay, Overlay::None);
        send(&mut app, Message::ToggleMenu);
        assert!(app.menu.is_none());
    }

    #[test]
    fn left_click_swaps_the_menu_for_the_popup() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        send(&mut app, Message::TogglePopup);
        assert!(app.menu.is_none());
        assert!(app.popup.is_some());
    }

    #[test]
    fn menu_about_opens_the_popup_on_the_about_view() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        send(&mut app, Message::MenuAbout);
        assert!(app.menu.is_none());
        assert!(app.popup.is_some());
        assert_eq!(app.overlay, Overlay::About);
        send(&mut app, Message::Escape);
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.popup.is_some());
    }

    #[test]
    fn menu_refresh_opens_the_popup_and_lists() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        send(&mut app, Message::MenuRefresh);
        assert!(app.menu.is_none());
        assert!(app.popup.is_some());
        assert!(app.loading);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn menu_keys_only_escape_counts() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        let menu = app.menu.expect("menu open");
        send(&mut app, Message::Key(menu, KeyAction::Refresh));
        assert!(!app.loading);
        assert_eq!(app.menu, Some(menu));
        send(&mut app, Message::Key(menu, KeyAction::Escape));
        assert!(app.menu.is_none());
    }

    #[test]
    fn menu_dismissed_by_the_compositor_leaves_the_popup_state_alone() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        let menu = app.menu.expect("menu open");
        send(&mut app, Message::PopupClosed(menu));
        assert!(app.menu.is_none());
        assert!(app.popup.is_none());
    }

    /// Window mode, as `init` leaves it: the main window is the popup and a list is running.
    fn window_model() -> AppModel {
        let mut app = model();
        app.mode = Mode::Window;
        app.listing = Listing::NotLoaded;
        let _ = app.open_window();
        app
    }

    #[test]
    fn window_mode_uses_the_main_window_and_lists_at_once() {
        let app = window_model();
        assert!(app.popup.is_some());
        assert_eq!(app.popup, app.core.main_window_id());
        assert!(app.loading);
    }

    #[test]
    fn window_opens_at_the_popup_size_and_can_shrink_but_not_below_the_footer() {
        assert_eq!(WINDOW_SIZE.width, POPUP_WIDTH);
        let app = window_model();
        assert_eq!(app.pane_height(), Length::Fill);
    }

    #[test]
    fn window_becomes_resizable_once_shown() {
        let mut app = window_model();
        assert!(!app.window_resizable);
        send(&mut app, Message::WindowShown);
        assert!(app.window_resizable);
        // Later focus changes and ticks change nothing.
        send(&mut app, Message::WindowShown);
        assert!(app.window_resizable);
        let focused = event::Event::Window(window::Event::Focused);
        assert!(window_focused(focused, event::Status::Ignored, Id::unique()).is_some());

        let mut applet = model();
        send(&mut applet, Message::WindowShown);
        assert!(!applet.window_resizable);
    }

    #[test]
    fn window_rows_leave_long_comments_to_the_pane_width() {
        let mut app = window_model();
        assert_eq!(app.row_comment_chars(), MAX_COMMENT_CHARS);
        app.mode = Mode::Applet;
        assert_eq!(app.row_comment_chars(), ROW_COMMENT_CHARS);
    }

    #[test]
    fn window_mode_takes_keys_for_the_main_window() {
        let mut app = window_model();
        app.on_listed(Ok(apsis_core::parse_list(DEVICE_LIST).unwrap()));
        let window = app.popup.expect("window open");
        send(&mut app, Message::Key(window, KeyAction::Down));
        assert_eq!(app.selected, 1);
        send(&mut app, Message::Key(window, KeyAction::Create));
        assert_eq!(app.prompt, Prompt::Comment(String::new()));
    }

    #[test]
    fn window_mode_esc_backs_out_then_closes_the_window() {
        let mut app = window_model();
        let window = app.popup.expect("window open");
        send(&mut app, Message::ToggleHelp);
        send(&mut app, Message::Key(window, KeyAction::Escape));
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.popup, Some(window));
        send(&mut app, Message::Key(window, KeyAction::Escape));
        assert!(app.popup.is_none());
    }

    const DEVICE_LIST: &str = include_str!("../../apsis-core/tests/fixtures/list-rsync-device.txt");
    const UNCONFIGURED_LIST: &str =
        include_str!("../../apsis-core/tests/fixtures/list-unconfigured.txt");

    /// The popup open on a listed fixture. Nothing here runs timeshift: `update`'s tasks are
    /// dropped without being run.
    fn listed(fixture: &str) -> AppModel {
        let mut app = model();
        app.on_listed(Ok(apsis_core::parse_list(fixture).unwrap()));
        send(&mut app, Message::TogglePopup);
        assert!(app.popup.is_some() && !app.loading);
        app
    }

    fn typed(app: &mut AppModel, text: &str) {
        send(app, Message::Input(text.to_owned()));
    }

    fn failed(code: i32) -> Result<(), CliError> {
        Err(CliError::Failed {
            code: Some(code),
            output: vec!["E: boom".to_owned()],
        })
    }

    #[test]
    fn create_prompt_takes_text_not_commands() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        assert_eq!(app.prompt, Prompt::Comment(String::new()));
        typed(&mut app, "r and k");
        assert_eq!(app.prompt, Prompt::Comment("r and k".to_owned()));
        assert!(!app.loading);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn comment_is_capped_while_typing() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        typed(&mut app, &"é".repeat(MAX_COMMENT_CHARS + 5));
        let Prompt::Comment(comment) = &app.prompt else {
            panic!("{:?}", app.prompt)
        };
        assert_eq!(comment.chars().count(), MAX_COMMENT_CHARS);
    }

    #[test]
    fn esc_cancels_the_prompt_and_keeps_the_popup() {
        let mut app = listed(DEVICE_LIST);
        let popup = app.popup.unwrap();
        typed(&mut app, "c");
        typed(&mut app, "half");
        send(&mut app, Message::Key(popup, KeyAction::Escape));
        assert_eq!(app.prompt, Prompt::Command);
        assert_eq!(app.popup, Some(popup));
        assert!(app.running.is_none());
    }

    #[test]
    fn bad_comment_stays_in_the_prompt_without_running() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        typed(&mut app, "--yes");
        send(&mut app, Message::Submit);
        assert_eq!(app.prompt, Prompt::Comment("--yes".to_owned()));
        assert!(app.running.is_none());
        assert!(matches!(app.status, Some(Status::Error(_))));
    }

    #[test]
    fn enter_on_a_comment_starts_the_create() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        typed(&mut app, "before update");
        send(&mut app, Message::Submit);
        assert_eq!(
            app.running,
            Some(Operation::Create("before update".to_owned()))
        );
        assert_eq!(app.prompt, Prompt::Command);
    }

    #[test]
    fn delete_asks_for_y_about_the_selected_snapshot() {
        let mut app = listed(DEVICE_LIST);
        let popup = app.popup.unwrap();
        send(&mut app, Message::Key(popup, KeyAction::Down));
        let name = app.snapshots()[1].name.clone();

        typed(&mut app, "d");
        assert_eq!(
            app.prompt,
            Prompt::ConfirmDelete {
                name: name.clone(),
                typed: String::new()
            }
        );
        typed(&mut app, "n");
        send(&mut app, Message::Submit);
        assert_eq!(app.prompt, Prompt::Command);
        assert!(app.running.is_none());

        for no in ["", "yes", "j"] {
            typed(&mut app, "d");
            typed(&mut app, no);
            send(&mut app, Message::Submit);
            assert!(app.running.is_none(), "{no:?}");
        }

        typed(&mut app, "d");
        typed(&mut app, "Y");
        send(&mut app, Message::Submit);
        assert_eq!(app.running, Some(Operation::Delete(name)));
    }

    #[test]
    fn create_and_delete_need_a_listed_device() {
        for mut app in [listed(UNCONFIGURED_LIST), model()] {
            send(&mut app, Message::StartCreate);
            send(&mut app, Message::StartDelete);
            assert_eq!(app.prompt, Prompt::Command);
        }
    }

    #[test]
    fn delete_needs_a_snapshot_but_create_does_not() {
        let mut app = listed(DEVICE_LIST);
        if let Listing::Loaded(list) = &mut app.listing {
            list.snapshots.clear();
        }
        typed(&mut app, "d");
        assert_eq!(app.prompt, Prompt::Command);
        typed(&mut app, "c");
        assert_eq!(app.prompt, Prompt::Comment(String::new()));
    }

    #[test]
    fn only_esc_works_while_running() {
        let mut app = listed(DEVICE_LIST);
        let popup = app.popup.unwrap();
        app.running = Some(Operation::Create(String::new()));
        typed(&mut app, "rcdj");
        send(&mut app, Message::Key(popup, KeyAction::Down));
        send(&mut app, Message::Refresh);
        send(&mut app, Message::MenuRefresh);
        assert!(!app.loading);
        assert_eq!(app.prompt, Prompt::Command);
        assert_eq!(app.selected, 0);
        send(&mut app, Message::Key(popup, KeyAction::Escape));
        assert!(app.popup.is_none());
        assert!(app.running.is_some(), "Esc doesn't cancel a root operation");
    }

    #[test]
    fn finishing_refreshes_only_if_timeshift_ran() {
        let create = Operation::Create(String::new());
        for (result, refresh) in [
            (Ok(()), true),
            (failed(1), true),
            (failed(126), false),
            (failed(127), false),
            (Err(CliError::NotInstalled), false),
            (Err(CliError::NotAuthorized), false),
            (Err(CliError::Other("no snapshot device".to_owned())), false),
        ] {
            let mut app = listed(DEVICE_LIST);
            app.running = Some(create.clone());
            let ok = result.is_ok();
            send(&mut app, Message::Finished(create.clone(), result));
            assert!(app.running.is_none());
            assert_eq!(app.loading, refresh);
            assert_eq!(matches!(app.status, Some(Status::Info(_))), ok);
        }
    }

    #[test]
    fn helper_refusal_reads_as_not_authorised() {
        let mut app = listed(DEVICE_LIST);
        let create = Operation::Create(String::new());
        let refused = apsis_core::Error::NotAuthorized;
        send(&mut app, Message::Finished(create, Err(refused.into())));
        let Some(Status::Error(text)) = &app.status else {
            panic!("{:?}", app.status)
        };
        assert!(text.ends_with(&fl!("failed-auth")), "{text}");
    }

    const STALE_MOUNT_LIST: &str =
        include_str!("../../apsis-core/tests/fixtures/list-rsync-stale-mount.txt");

    fn status_error(app: &AppModel) -> &str {
        match &app.status {
            Some(Status::Error(text)) => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_disk_is_named_by_its_short_uuid_and_not_retried() {
        let mut app = listed(DEVICE_LIST);
        let create = Operation::Create(String::new());
        app.running = Some(create.clone());
        let missing = apsis_core::Error::DeviceNotFound {
            device: "/dev/sdX1".to_owned(),
        };
        send(&mut app, Message::Finished(create, Err(missing.into())));
        assert!(!app.loading, "a list would only fail again");
        let want = fl!("disk-missing", id = "UUID 0000…");
        assert!(
            status_error(&app).ends_with(&want),
            "{}",
            status_error(&app)
        );
    }

    #[test]
    fn missing_disk_keeps_the_uuid_it_was_last_seen_with() {
        let mut app = listed(DEVICE_LIST);
        let missing = CliError::DeviceNotFound {
            device: "/dev/sdX1".to_owned(),
        };
        app.on_listed(Err(missing.clone()));
        assert_eq!(
            error_summary(&missing, app.known_uuid.as_deref()),
            fl!("disk-missing", id = "UUID 0000…")
        );
        // Never seen: Timeshift's own name for it.
        assert_eq!(
            error_summary(&missing, None),
            fl!("disk-missing", id = "/dev/sdX1")
        );
    }

    #[test]
    fn timeshift_output_is_shown_not_just_the_exit_code() {
        let mut app = listed(DEVICE_LIST);
        let failed = apsis_core::Error::Failed {
            code: Some(1),
            output: "E: first\nE: second".to_owned(),
        };
        let delete = Operation::Delete("2026-09-19_09-29-57".to_owned());
        send(&mut app, Message::Finished(delete, Err(failed.into())));
        assert!(
            status_error(&app).ends_with("E: first\nE: second"),
            "{}",
            status_error(&app)
        );
    }

    #[test]
    fn list_warnings_reach_the_activity_pane() {
        let mut app = listed(DEVICE_LIST);
        assert!(app.list_warnings().is_empty());
        app.on_listed(Ok(apsis_core::parse_list(STALE_MOUNT_LIST).unwrap()));
        assert_eq!(app.snapshots().len(), 5);
        assert_eq!(app.list_warnings(), ["E: Failed to remove directory"]);
    }

    #[test]
    fn losing_focus_keeps_what_was_typed() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        typed(&mut app, "half");
        send(&mut app, Message::InputUnfocused);
        assert_eq!(app.prompt, Prompt::Comment("half".to_owned()));
    }

    #[test]
    fn one_c_opens_the_comment_prompt_and_is_not_typed() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        assert_eq!(app.prompt, Prompt::Comment(String::new()));
        typed(&mut app, "c");
        assert_eq!(
            app.prompt,
            Prompt::Comment("c".to_owned()),
            "second c is text"
        );

        let mut app = listed(DEVICE_LIST);
        let popup = app.popup.unwrap();
        // The same when the key arrives through the event stream (input not focused).
        send(&mut app, Message::Key(popup, KeyAction::Delete));
        assert!(matches!(&app.prompt, Prompt::ConfirmDelete { typed, .. } if typed.is_empty()));
    }

    #[test]
    fn failure_shows_the_last_stderr_line() {
        let mut app = listed(DEVICE_LIST);
        let delete = Operation::Delete("2026-09-19_09-29-57".to_owned());
        send(&mut app, Message::Finished(delete, failed(1)));
        let Some(Status::Error(text)) = &app.status else {
            panic!("{:?}", app.status)
        };
        assert!(text.ends_with("E: boom"), "{text}");
    }

    #[test]
    fn activity_shows_the_running_operation_then_its_result() {
        let mut app = listed(DEVICE_LIST);
        assert_eq!(app.activity_line(), (fl!("activity-idle"), Tone::Dim));

        typed(&mut app, "c");
        typed(&mut app, "before update");
        send(&mut app, Message::Submit);
        let (text, tone) = app.activity_line();
        assert!(text.starts_with(&fl!("creating")), "{text}");
        assert_eq!(tone, Tone::Normal);

        let create = app.running.clone().expect("create running");
        send(&mut app, Message::Finished(create, Ok(())));
        assert_eq!(app.activity_line(), (fl!("created"), Tone::Dim));

        let delete = Operation::Delete("2026-09-19_09-29-57".to_owned());
        send(&mut app, Message::Finished(delete, failed(1)));
        assert_eq!(app.activity_line().1, Tone::Error);
    }

    #[test]
    fn tab_makes_details_the_active_pane() {
        let mut app = listed(DEVICE_LIST);
        let popup = app.popup.unwrap();
        send(&mut app, Message::Key(popup, KeyAction::FocusDetails));
        assert_eq!(app.overlay, Overlay::Details);
        send(&mut app, Message::Key(popup, KeyAction::FocusDetails));
        assert_eq!(app.overlay, Overlay::None);
        send(&mut app, Message::Key(popup, KeyAction::FocusDetails));
        send(&mut app, Message::Select(2));
        assert_eq!((app.overlay, app.selected), (Overlay::None, 2));
        send(&mut app, Message::Key(popup, KeyAction::FocusDetails));
        send(&mut app, Message::Escape);
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.popup.is_some());
    }

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            size: 3,
            mtime: 1_700_000_000,
            mode: 0o100_644,
            uid: 0,
            gid: 0,
            owner: "root:root".to_owned(),
            target: String::new(),
            live: Live::Changed,
            live_size: 5,
            live_mtime: 1_700_000_100,
        }
    }

    fn folder(entries: &[(&str, Kind)]) -> Result<FolderListing, CliError> {
        Ok(FolderListing {
            entries: entries.iter().map(|(n, k)| entry(n, *k)).collect(),
            truncated: false,
        })
    }

    /// The browser open on the second snapshot's `/`, with `etc/` and `vmlinuz` in it.
    fn browsing() -> AppModel {
        let mut app = listed(DEVICE_LIST);
        send(&mut app, Message::OpenBrowser(1));
        assert_eq!(app.overlay, Overlay::Browse);
        let snapshot = app.snapshots()[1].name.clone();
        assert_eq!(app.browser.as_ref().unwrap().snapshot, snapshot);
        assert!(app.browser.as_ref().unwrap().is_loading());
        let root = folder(&[("vmlinuz", Kind::Link), ("etc", Kind::Dir)]);
        send(&mut app, Message::Browsed(snapshot, SnapPath::root(), root));
        app
    }

    fn browsed(app: &mut AppModel, path: &str, entries: &[(&str, Kind)]) {
        let snapshot = app.browser.as_ref().unwrap().snapshot.clone();
        let path = SnapPath::parse(path).unwrap();
        send(app, Message::Browsed(snapshot, path, folder(entries)));
    }

    #[test]
    fn enter_opens_the_browser_and_esc_closes_it() {
        let mut app = listed(DEVICE_LIST);
        send(&mut app, Message::Submit);
        assert_eq!(app.overlay, Overlay::Browse);
        assert_eq!(
            app.browser.as_ref().unwrap().snapshot,
            app.snapshots()[0].name
        );
        send(&mut app, Message::Escape);
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.browser.is_none());
        assert!(app.popup.is_some());
        // Not while something runs.
        app.running = Some(Operation::Create(String::new()));
        send(&mut app, Message::OpenBrowser(0));
        assert!(app.browser.is_none());
    }

    #[test]
    fn browser_keys_move_enter_go_up_and_mark() {
        let mut app = browsing();
        let popup = app.popup.unwrap();
        assert_eq!(app.browser.as_ref().unwrap().current().unwrap().name, "etc");
        // Enter (the `>` line's submit) goes into the folder.
        send(&mut app, Message::Submit);
        assert_eq!(app.browser.as_ref().unwrap().path.to_string(), "/etc");
        browsed(
            &mut app,
            "/etc",
            &[("hosts", Kind::File), ("fstab", Kind::File)],
        );
        assert!(app.body_title().ends_with(":/etc"), "{}", app.body_title());
        typed(&mut app, "j");
        typed(&mut app, " ");
        assert!(
            app.body_title().ends_with("1 marked"),
            "{}",
            app.body_title()
        );
        // Space stays on the marked row; J marks and moves down.
        assert_eq!(app.browser.as_ref().unwrap().cursor, 1);
        typed(&mut app, "k");
        typed(&mut app, "J");
        assert_eq!(app.browser.as_ref().unwrap().cursor, 1);
        assert!(
            app.body_title().ends_with("2 marked"),
            "{}",
            app.body_title()
        );
        // Backspace and h go up, selecting the folder we left.
        send(&mut app, Message::Key(popup, KeyAction::Back));
        browsed(
            &mut app,
            "/",
            &[("etc", Kind::Dir), ("vmlinuz", Kind::Link)],
        );
        assert_eq!(app.browser.as_ref().unwrap().current().unwrap().name, "etc");
        typed(&mut app, "l");
        assert_eq!(app.browser.as_ref().unwrap().path.to_string(), "/etc");
        typed(&mut app, "h");
        assert!(app.browser.as_ref().unwrap().path.is_root());
        // Snapshot keys do nothing in the browser.
        typed(&mut app, "cd");
        assert_eq!(app.prompt, Prompt::Command);
        assert!(!app.loading);
    }

    #[test]
    fn folder_restore_runs_after_its_dry_run() {
        let mut app = browsing();
        typed(&mut app, "R");
        assert_eq!(
            app.prompt,
            Prompt::RestoreWhere {
                count: 1,
                typed: String::new()
            }
        );
        send(&mut app, Message::Submit);
        let Some(Operation::Restore(request)) = app.running.clone() else {
            panic!("{:?}", app.running)
        };
        assert!(request.dry_run);
        assert_eq!(request.destination, Destination::Folder);
        assert_eq!(request.paths, ["/etc"]);
        assert!(
            app.activity_line()
                .0
                .starts_with(&fl!("restore-dry-running"))
        );

        send(
            &mut app,
            Message::RestoreDone(request.clone(), Ok("the plan".to_owned())),
        );
        assert_eq!(app.overlay, Overlay::RestorePlan);
        assert_eq!(app.body_title(), fl!("pane-restore-plan"));
        // Enter runs it for real.
        send(&mut app, Message::Submit);
        let Some(Operation::Restore(real)) = app.running.clone() else {
            panic!("{:?}", app.running)
        };
        assert!(!real.dry_run);
        assert_eq!(real.paths, request.paths);
        send(
            &mut app,
            Message::RestoreDone(real, Ok("restored".to_owned())),
        );
        assert_eq!(app.body_title(), fl!("pane-restore-result"));
        assert!(matches!(app.status, Some(Status::Info(_))));
        // Esc goes back to the browser, which reads the folder again.
        send(&mut app, Message::Escape);
        assert_eq!(app.overlay, Overlay::Browse);
        assert!(app.browser.as_ref().unwrap().is_loading());
    }

    #[test]
    fn original_restore_asks_for_y_after_its_plan() {
        let mut app = browsing();
        typed(&mut app, "R");
        typed(&mut app, "o");
        send(&mut app, Message::Submit);
        let Some(Operation::Restore(request)) = app.running.clone() else {
            panic!("{:?}", app.running)
        };
        assert_eq!(request.destination, Destination::Original);
        send(
            &mut app,
            Message::RestoreDone(request, Ok("plan".to_owned())),
        );
        send(&mut app, Message::Submit);
        assert!(app.running.is_none(), "not without y");
        assert!(matches!(
            app.prompt,
            Prompt::ConfirmRestore { count: 1, .. }
        ));
        for no in ["", "n", "yes"] {
            typed(&mut app, no);
            send(&mut app, Message::Submit);
            assert!(app.running.is_none(), "{no:?}");
            send(&mut app, Message::Submit);
        }
        typed(&mut app, "y");
        send(&mut app, Message::Submit);
        assert!(
            matches!(&app.running, Some(Operation::Restore(r)) if !r.dry_run && r.destination == Destination::Original)
        );
    }

    #[test]
    fn a_failed_dry_run_or_odd_answer_changes_nothing() {
        let mut app = browsing();
        typed(&mut app, "R");
        typed(&mut app, "x");
        send(&mut app, Message::Submit);
        assert!(app.running.is_none());
        assert_eq!(app.overlay, Overlay::Browse);

        typed(&mut app, "R");
        send(&mut app, Message::Submit);
        let Some(Operation::Restore(request)) = app.running.clone() else {
            panic!()
        };
        let refused = apsis_core::Error::InvalidInput("/etc/x is a symlink".to_owned());
        send(&mut app, Message::RestoreDone(request, Err(refused.into())));
        assert_eq!(app.overlay, Overlay::Browse);
        assert!(app.pending_restore.is_none());
        assert!(
            status_error(&app).contains("symlink"),
            "{}",
            status_error(&app)
        );
    }

    #[test]
    fn entry_details_spell_out_the_difference() {
        let changed = entry("hosts", Kind::File);
        assert!(
            live_text(&changed).contains("size 3B here, 5B now"),
            "{}",
            live_text(&changed)
        );
        let mut missing = entry("x", Kind::File);
        missing.live = Live::Missing;
        assert_eq!(live_text(&missing), fl!("live-missing"));
    }

    #[test]
    fn panes_fit_the_list_between_min_and_max_rows() {
        let mut app = listed(DEVICE_LIST);
        let Listing::Loaded(list) = &mut app.listing else {
            panic!("listed")
        };
        let one = list.snapshots[0].clone();
        for (count, rows) in [
            (0, MIN_ROWS),
            (3, MIN_ROWS),
            (6, 6),
            (8, 8),
            (20, VISIBLE_ROWS),
        ] {
            if let Listing::Loaded(list) = &mut app.listing {
                list.snapshots = vec![one.clone(); count];
            }
            assert_eq!(app.pane_rows(), rows, "{count} snapshots");
        }
        send(&mut app, Message::ToggleHelp);
        assert_eq!(app.pane_rows(), VISIBLE_ROWS);
    }

    #[test]
    fn selection_stays_in_place_when_the_selected_snapshot_is_gone() {
        let mut app = listed(DEVICE_LIST);
        app.selected = 2;
        let mut list = apsis_core::parse_list(DEVICE_LIST).unwrap();
        let gone = app.snapshots()[2].name.clone();
        list.snapshots.retain(|s| s.name != gone);
        app.on_listed(Ok(list));
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn closing_the_popup_drops_a_half_typed_prompt() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        send(&mut app, Message::TogglePopup);
        assert_eq!(app.prompt, Prompt::Command);
    }

    #[test]
    fn about_values_come_from_cargo() {
        assert_eq!(LICENSE, "GPL-3.0-only");
        assert!(REPOSITORY.starts_with("https://"));
        assert!(!VERSION.is_empty());
    }

    const CONFIG: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");

    fn settings_info() -> SettingsInfo {
        let users = vec![("root".to_owned(), "/root".to_owned(), false)];
        apsis_core::helper::info_from_wire((CONFIG.to_owned(), LSBLK.to_owned(), users, false))
            .unwrap()
    }

    /// The popup on the settings view, read from the fixtures.
    fn in_settings() -> AppModel {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "s");
        assert_eq!(app.overlay, Overlay::Settings);
        assert!(matches!(app.settings, SettingsLoad::Loading { .. }));
        send(&mut app, Message::SettingsRead(Ok(settings_info())));
        app
    }

    fn view(app: &AppModel) -> &SettingsView {
        match &app.settings {
            SettingsLoad::Ready(view) => view,
            other => panic!("settings not ready: {other:?}"),
        }
    }

    /// Moves the cursor to `row` with j.
    fn go_to(app: &mut AppModel, row: Row) {
        while view(app).current() != row {
            let before = view(app).cursor;
            typed(app, "j");
            assert_ne!(view(app).cursor, before, "no row {row:?}");
        }
    }

    #[test]
    fn settings_keys_leave_snapshots_alone() {
        let mut app = in_settings();
        typed(&mut app, "c");
        typed(&mut app, "d");
        assert_eq!(app.prompt, Prompt::Command);
        assert_eq!(app.selected, 0);
        // r reads the settings again, it doesn't list.
        typed(&mut app, "r");
        assert!(!app.loading);
        assert!(matches!(app.settings, SettingsLoad::Loading { .. }));
    }

    #[test]
    fn space_enter_and_double_click_change_a_row() {
        let mut app = in_settings();
        go_to(&mut app, Row::Mode);
        typed(&mut app, " ");
        assert!(view(&app).edited.btrfs_mode);
        send(&mut app, Message::Submit);
        assert!(!view(&app).edited.btrfs_mode);
        send(&mut app, Message::SettingsActivate(1));
        assert!(view(&app).edited.btrfs_mode);
        assert_eq!(app.body_title(), fl!("pane-settings-unsaved"));
    }

    #[test]
    fn filters_are_added_at_the_prompt() {
        let mut app = in_settings();
        typed(&mut app, "a");
        assert_eq!(app.prompt, Prompt::Filter(String::new()));
        typed(&mut app, "+ ");
        send(&mut app, Message::Submit);
        // Blank: stays in the prompt, says why.
        assert_eq!(app.prompt, Prompt::Filter("+ ".to_owned()));
        assert!(matches!(app.status, Some(Status::Error(_))));
        typed(&mut app, "*.mp3");
        send(&mut app, Message::Submit);
        assert_eq!(app.prompt, Prompt::Command);
        assert_eq!(view(&app).edited.exclude.last().unwrap(), "*.mp3");
        assert_eq!(view(&app).current(), Row::Filter(4));
        typed(&mut app, "x");
        assert_eq!(view(&app).edited.exclude.len(), 4);
    }

    #[test]
    fn counts_take_digits_at_the_prompt() {
        let mut app = in_settings();
        go_to(&mut app, Row::Schedule(Level::Daily));
        typed(&mut app, "e");
        assert_eq!(
            app.prompt,
            Prompt::Count {
                level: Level::Daily,
                typed: "5".to_owned()
            }
        );
        typed(&mut app, "12x");
        send(&mut app, Message::Submit);
        assert_eq!(view(&app).edited.count(Level::Daily), 12);
        typed(&mut app, "+");
        typed(&mut app, "+");
        typed(&mut app, "-");
        assert_eq!(view(&app).edited.count(Level::Daily), 13);
    }

    #[test]
    fn esc_with_unsaved_changes_asks_first() {
        let mut app = in_settings();
        go_to(&mut app, Row::Schedule(Level::Boot));
        typed(&mut app, " ");
        let popup = app.popup.unwrap();
        send(&mut app, Message::Key(popup, KeyAction::Escape));
        assert_eq!(app.overlay, Overlay::Settings);
        assert!(matches!(app.status, Some(Status::Info(_))));
        send(&mut app, Message::Key(popup, KeyAction::Escape));
        assert_eq!(app.overlay, Overlay::None);
        assert!(!view(&app).dirty());
        assert_eq!(app.popup, Some(popup));
    }

    #[test]
    fn write_checks_first_then_reads_back_and_lists() {
        let mut app = in_settings();
        // Nothing to write.
        typed(&mut app, "w");
        assert!(!app.saving_settings);
        go_to(&mut app, Row::Schedule(Level::Daily));
        typed(&mut app, " ");
        typed(&mut app, "w");
        assert!(app.saving_settings);
        // Keys wait while it writes.
        typed(&mut app, " ");
        assert!(view(&app).edited.scheduled(Level::Daily));

        send(&mut app, Message::SettingsWritten(Ok(String::new())));
        assert!(!app.saving_settings);
        assert!(matches!(app.status, Some(Status::Info(_))));
        assert!(matches!(app.settings, SettingsLoad::Loading { .. }));
        assert!(app.loading);
    }

    #[test]
    fn a_refused_write_keeps_the_edits() {
        let mut app = in_settings();
        go_to(&mut app, Row::Schedule(Level::Daily));
        typed(&mut app, " ");
        typed(&mut app, "w");
        send(
            &mut app,
            Message::SettingsWritten(Err(CliError::from(apsis_core::Error::SettingsChanged))),
        );
        assert!(matches!(app.status, Some(Status::Error(_))));
        assert!(view(&app).dirty());
        assert!(!app.loading);
    }

    #[test]
    fn menu_settings_opens_the_popup_on_settings() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        send(&mut app, Message::MenuSettings);
        assert!(app.menu.is_none() && app.popup.is_some());
        assert_eq!(app.overlay, Overlay::Settings);
        assert!(matches!(app.settings, SettingsLoad::Loading { .. }));
    }

    #[test]
    fn failed_settings_read_is_shown() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "s");
        send(
            &mut app,
            Message::SettingsRead(Err(CliError::Other("no helper".to_owned()))),
        );
        assert!(
            matches!(&app.settings, SettingsLoad::Failed(reason) if reason.contains("no helper"))
        );
    }

    #[test]
    fn modifier_combinations_are_left_alone() {
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            let event = press(character("r"), modifiers);
            assert!(key_action(event, event::Status::Ignored, Id::unique()).is_none());
        }
    }
}
