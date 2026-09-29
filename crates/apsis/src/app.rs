// SPDX-License-Identifier: GPL-3.0-only

use std::collections::BTreeSet;
use std::future::Future;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use apsis_core::config::{Config as ApsisConfig, ConfigInfo};
use apsis_core::helper::HelperClient;
use apsis_core::restore::{
    Destination, Entry, Kind, Listing as FolderListing, Live, Request, SnapPath,
};
use apsis_core::retention::{self, Kept, ManualPlan};
use apsis_core::settings::HomeState;
use apsis_core::status::{DISK_CRITICAL, DISK_LOW};
use apsis_core::{
    ApsisStatus, DiskUsage, MAX_COMMENT_CHARS, Progress, Severity, Snapshot, SnapshotList,
    validate_comment,
};
use cosmic::applet::{menu_button, padded_control};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::advanced::text::EllipsizeHeightLimit;
use cosmic::iced::border::Radius;
use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::{StreamExt, stream};
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
use crate::settings_view::{ApsisChoice, Counted, Row, Section, SettingsView, Step};
use crate::status::{DiskStrip, StatusView, Strip};
use crate::tallest::Tallest;

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
/// The read-only overview's width: a narrow card, not the window's 720.
const OVERVIEW_WIDTH: f32 = 400.0;
/// Newest snapshots the overview lists.
const OVERVIEW_ROWS: usize = 5;
/// Longest comment on an overview row (the date and tags come first).
const OVERVIEW_COMMENT_CHARS: usize = 14;
/// Width of the right-click menu.
const MENU_WIDTH: f32 = 240.0;
/// Height of one snapshot row: monotext line height (20) plus vertical padding.
const ROW_HEIGHT: f32 = 24.0;
/// The popup's padding either side of the panes, and the space between them.
const POPUP_PADDING: f32 = 12.0;
const PANE_SPACING: f32 = 8.0;
/// The popup's details pane: two fifths of the panes' width, as `FillPortion(3)` and
/// `FillPortion(2)` share it.
const POPUP_DETAILS_WIDTH: f32 = (POPUP_WIDTH - 2.0 * POPUP_PADDING - PANE_SPACING) * 2.0 / 5.0;
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
/// Width of the `>` line's input when a long label (the names of a bulk delete) takes the rest
/// of the row and wraps.
const SHORT_INPUT_WIDTH: f32 = 48.0;
/// Advance of one monotext cell (14 px text; monospace fonts are about 0.6 em wide), for how
/// many block characters fit in the disk bar. A slightly wider font only clips the bar's end.
const MONO_CELL_WIDTH: f32 = 14.0 * 0.6;
/// Height of one monotext line.
const LINE_HEIGHT: f32 = 20.0;
/// How often the panel lists in the background (through the helper only) for the reminder.
const BACKGROUND_LIST_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
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

/// The applet: a panel button and a terminal-style popup listing snapshots. In
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
    listing: Listing,
    /// A list is running in the background.
    loading: bool,
    /// UUID of the backup disk from the last good list, to name it when it goes missing.
    known_uuid: Option<String>,
    /// A create, delete or restore is running in the background (as root, in the helper).
    running: Option<Operation>,
    /// What the `>` line is asking for.
    prompt: Prompt,
    /// How the last create or delete went, or why a comment was refused.
    status: Option<Status>,
    /// The running create's or restore's last `Progress` from the helper. `None` until the
    /// first one.
    progress: Option<Progress>,
    /// When the running create or restore started, for `working · 1m 08s elapsed`.
    run_started: Option<Instant>,
    /// The panel's popup: an overview that shows the last list and changes nothing. Only
    /// `apsis --window` creates, deletes, restores and edits settings.
    read_only: bool,
    /// The window's room: the dock's active cell.
    room: Room,
    /// The schedule room's selected row: 0 keep, 1 remind.
    schedule_row: usize,
    /// What happened this session, oldest first.
    log: Vec<LogEntry>,
    spinner: usize,
    /// Index into the displayed (newest first) snapshots.
    selected: usize,
    /// Names of the snapshots marked for a bulk delete (space / `J`).
    marked: BTreeSet<String>,
    overlay: Overlay,
    /// Apsis's config, for the settings view.
    settings: SettingsLoad,
    /// A config write is running in the helper.
    saving_settings: bool,
    /// The snapshot browser, while it's open ([`Overlay::Browse`]).
    browser: Option<Browser>,
    /// A restore whose dry run was shown: Enter runs it for real.
    pending_restore: Option<Request>,
    /// A create finished: when its list arrives, offer to prune old manual snapshots.
    prune_after_list: bool,
    /// A restore ran in the browser: list again once it's left, for the disk line (see
    /// [`AppModel::list_if_restored`]).
    list_after_browser: bool,
    /// `Browse` calls not answered yet. They hold the helper's lock, so a list waits for them.
    browse_calls: usize,
    /// The last restore's plan or result, shown by [`Overlay::RestorePlan`].
    restore_text: Option<String>,
    /// Window mode: the window is on screen and its size limits are relaxed (see
    /// [`run_window`]).
    window_resizable: bool,
    /// The symbolic Apsis icon, from the icon theme or embedded.
    icon: icon::Handle,
    /// A background list went through `apsis-helper` (so more can, without a password).
    helper_found: bool,
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
    /// Nothing listed yet.
    NotLoaded,
    /// Snapshots newest first.
    Loaded(SnapshotList),
    Failed(CliError),
}

/// A `Clone`able summary of [`apsis_core::Error`] for messages and the view.
#[derive(Debug, Clone)]
pub enum CliError {
    /// `apsis-helper` isn't installed (or the system bus can't be reached): nothing works
    /// without it.
    NoHelper,
    /// polkit refused `apsis-helper`, or the password dialog was dismissed. Nothing ran.
    NotAuthorized,
    /// The backup disk isn't there (unplugged, or dropped off the USB bus). `device` is its
    /// UUID.
    DeviceNotFound {
        device: String,
    },
    Other(String),
}

impl From<apsis_core::Error> for CliError {
    fn from(error: apsis_core::Error) -> Self {
        match error {
            apsis_core::Error::NotAuthorized => Self::NotAuthorized,
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
    /// `> delete 3 snapshots: <name>, <name>, <name>? [y/N] _`, for the marked ones, in list
    /// order. Enter with `y` deletes them one by one, anything else cancels.
    ConfirmDeleteMany { names: Vec<String>, typed: String },
    /// `> remove 3 old manual snapshots? [y/N] _` under the prune preview. Enter with `y`
    /// deletes them one by one (as a bulk delete), anything else cancels.
    ConfirmPrune { names: Vec<String>, typed: String },
    /// Settings: `> add filter: _`. Enter adds it.
    Filter(String),
    /// Settings: `> keep daily: 5_` (or keep manual, remind after). Enter sets the number.
    Count { counted: Counted, typed: String },
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
    /// The marked snapshots, deleted one at a time; the first `done` are gone. Stops at the
    /// first failure.
    DeleteMany {
        names: Vec<String>,
        done: usize,
    },
    /// A file-level restore, or its dry run.
    Restore(Request),
}

/// The window's rooms (the dock under the activity pane). `1 2 3 4` jump to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Room {
    /// The snapshot list and its details: home.
    Snapshots,
    /// The list, with the create form where the details are.
    Create,
    /// Keep and remind (there is no scheduler yet).
    Schedule,
    /// What happened this session.
    Log,
}

impl Room {
    const ALL: [Self; 4] = [Self::Snapshots, Self::Create, Self::Schedule, Self::Log];

    fn label(self) -> String {
        match self {
            Self::Snapshots => fl!("room-snapshots"),
            Self::Create => fl!("room-create"),
            Self::Schedule => fl!("room-schedule"),
            Self::Log => fl!("room-log"),
        }
    }
}

/// One line of the log room: when, what, and whether it was a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogEntry {
    time: String,
    text: String,
    error: bool,
}

/// Most lines the log room keeps.
const LOG_LINES: usize = 200;

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

/// The activity pane's first line while a create or restore reports progress.
#[derive(Debug, Clone, PartialEq)]
enum ProgressLine {
    /// Progress will come, but there's no number yet:
    /// `creating snapshot · working · 1m 08s elapsed ⠋`.
    Working(String),
    /// `creating snapshot · 58% · 3m 12s left`, then a bar for `fraction` in the rest of the line.
    Bar { text: String, fraction: f64 },
}

/// How a line in the activity pane looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    Dim,
    Error,
}

/// How full the backup disk is, for the disk bar's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    /// Accent.
    Plenty,
    /// Under [`DISK_LOW`] free: the theme's warning colour.
    Low,
    /// Under [`DISK_CRITICAL`] free: the destructive colour.
    Critical,
}

impl Space {
    fn of(usage: &DiskUsage) -> Self {
        let free = usage.free_fraction();
        if free < DISK_CRITICAL {
            Self::Critical
        } else if free < DISK_LOW {
            Self::Low
        } else {
            Self::Plenty
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Details,
    Help,
    About,
    Settings,
    /// A snapshot's files, in the left pane; the selected entry's details on the right.
    Browse,
    /// A restore's plan (Enter runs it) or result, in the left pane.
    RestorePlan,
    /// What "keep last N manual" would delete and keep, while `y` is asked.
    Prune,
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
    /// Browser and snapshot list: `J` marks or unmarks, then moves down (space marks in
    /// place).
    MarkDown,
    /// Browser: `R` restores the marked entries.
    Restore,
    /// `p`: preview pruning old manual snapshots, then `y`.
    Prune,
    /// Tab: the details pane of the selected snapshot.
    FocusDetails,
    /// `o`: open the Apsis window (the panel's overview only).
    OpenWindow,
    /// `1 2 3 4`: the window's rooms.
    Room(Room),
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
    /// Menu: `cosmic-settings panel`, where the applet is removed or moved.
    MenuPanelSettings,
    /// Menu: closes it, like Esc.
    MenuClose,
    /// Opens the Apsis window in its own process (`apsis --window`, or with `--settings` or
    /// `--about`), and closes the popup.
    OpenWindow(Option<&'static str>),
    /// A click on a dock cell.
    Room(Room),
    /// A click on `[-]`, `[+]` or `[on]` in the schedule room.
    Schedule(Counted, Step),
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
    /// The running create or restore got this far.
    Progress(Progress),
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
    SettingsRead(Result<ConfigInfo, CliError>),
    /// A settings write finished: a note from the helper (empty when all went well).
    SettingsWritten(Result<String, CliError>),
    /// A click on a settings row, and a double-click (changes it, like space).
    SettingsSelect(usize),
    SettingsActivate(usize),
    /// A settings hint button.
    SettingsKey(KeyAction),
    /// Window mode: the window is on screen (first focus, or the fallback timer).
    WindowShown,
    /// Every [`BACKGROUND_LIST_EVERY`] while the popup is closed: list again for the reminder.
    BackgroundRefresh,
    /// A background list ended: `None` when there's no helper (nothing was run).
    BackgroundListed(Option<Result<SnapshotList, CliError>>),
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
            listing: Listing::NotLoaded,
            known_uuid: None,
            loading: false,
            running: None,
            progress: None,
            run_started: None,
            read_only: mode == Mode::Applet,
            room: Room::Snapshots,
            schedule_row: 0,
            log: Vec::new(),
            prompt: Prompt::Command,
            status: None,
            spinner: 0,
            selected: 0,
            marked: BTreeSet::new(),
            overlay: Overlay::None,
            settings: SettingsLoad::NotLoaded,
            saving_settings: false,
            browser: None,
            pending_restore: None,
            list_after_browser: false,
            prune_after_list: false,
            browse_calls: 0,
            restore_text: None,
            window_resizable: false,
            icon: symbolic_icon(),
            helper_found: false,
        };
        let task = match mode {
            // Lists at once for the reminder, but only through the helper (no password).
            // `loading` keeps the popup from starting a second list meanwhile.
            Mode::Applet => {
                app.loading = true;
                background_list()
            }
            Mode::Window => {
                let open = app.open_window();
                match startup_overlay(std::env::args_os().skip(1)) {
                    Some(Overlay::Settings) => Task::batch([open, app.open_settings_popup()]),
                    Some(overlay) => {
                        app.overlay = overlay;
                        open
                    }
                    None => open,
                }
            }
        };
        (app, task)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// The panel button: the icon (in the warning or destructive colour when the reminder is due
    /// or the disk is low), the optional `12h · 62%` label beside it on a horizontal panel, and
    /// a tooltip with the last snapshot and the disk. Left click opens the popup, right click
    /// the menu. The button itself only reacts to the left button.
    ///
    /// In window mode, the popup's UI instead.
    fn view(&self) -> Element<'_, Self::Message> {
        if self.mode == Mode::Window {
            return self.surface();
        }
        let status = self.status_view();
        let severity = status.severity();
        let button = if self.config.show_label && self.core.applet.is_horizontal() {
            self.label_button(&status)
        } else if severity == Severity::None {
            self.core.applet.icon_button_from_handle(self.icon.clone())
        } else {
            self.core
                .applet
                .button_from_element(self.panel_icon(severity), true)
        }
        .on_press(Message::TogglePopup);
        let button = widget::mouse_area(button).on_right_release(Message::ToggleMenu);
        self.core
            .applet
            .applet_tooltip(
                button,
                status.tooltip(),
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
        let (content, width) = if self.read_only {
            (self.overview(), OVERVIEW_WIDTH)
        } else {
            (self.surface(), POPUP_WIDTH)
        };
        self.core
            .applet
            .popup_container(content)
            .limits(
                Limits::NONE
                    .min_width(width)
                    .max_width(width)
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
        // The reminder's list, while the popup is closed; only with the helper.
        if self.mode == Mode::Applet && self.helper_found && self.popup.is_none() {
            subscriptions
                .push(time::every(BACKGROUND_LIST_EVERY).map(|_| Message::BackgroundRefresh));
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

    /// Every message goes through `handle`; what it left in the activity pane's status, or a
    /// list that failed, is also written to the log room.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        let status = self.status.clone();
        let list_failed = matches!(self.listing, Listing::Failed(_));
        let task = self.handle(message);
        self.log_changes(status.as_ref(), list_failed);
        task
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
    #[allow(clippy::too_many_lines, reason = "one arm per message")]
    fn handle(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
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
            Message::OpenWindow(flag) => return self.launch_window(flag),
            Message::Room(room) => return self.go_room(room),
            Message::Schedule(counted, step) => return self.step_setting(counted, step),
            Message::MenuAbout if self.read_only => return self.launch_window(Some("--about")),
            Message::MenuSettings if self.read_only => {
                return self.launch_window(Some("--settings"));
            }
            Message::Escape if self.read_only => return self.toggle_popup(),
            Message::MenuAbout => return self.open_popup(Overlay::About),
            Message::MenuSettings => return self.open_settings_popup(),
            Message::OpenSettings => return self.on_key(KeyAction::Settings),
            Message::SettingsRead(result) => return self.on_settings_read(result),
            Message::SettingsWritten(result) => return self.on_settings_written(result),
            Message::SettingsSelect(index) => return self.select_setting(index, false),
            Message::SettingsActivate(index) => return self.select_setting(index, true),
            Message::SettingsKey(action) => return self.on_key(action),
            Message::WindowShown => return self.make_window_resizable(),
            Message::BackgroundRefresh => {
                if self.popup.is_none() && !self.loading && self.running.is_none() {
                    self.loading = true;
                    return background_list();
                }
            }
            Message::BackgroundListed(result) => {
                self.loading = false;
                match result {
                    Some(result) => {
                        self.helper_found = true;
                        // A failure (disk unplugged) clears the reminder: nothing to judge by.
                        self.on_listed(result);
                    }
                    None => {
                        self.helper_found = false;
                        // The popup opened meanwhile and waits for a list: the usual one.
                        if self.popup.is_some() && matches!(self.listing, Listing::NotLoaded) {
                            return self.start_list();
                        }
                    }
                }
            }
            Message::MenuClose => return self.close_menu(),
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
                Prompt::Filter(pattern) => {
                    *pattern = text.chars().take(FILTER_CHARS).collect();
                }
                Prompt::Count { typed, .. } => {
                    *typed = text.chars().filter(char::is_ascii_digit).take(3).collect();
                }
                Prompt::ConfirmDelete { typed, .. }
                | Prompt::ConfirmDeleteMany { typed, .. }
                | Prompt::ConfirmPrune { typed, .. }
                | Prompt::RestoreWhere { typed, .. }
                | Prompt::ConfirmRestore { typed, .. } => {
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
            // A late one, after its operation ended, is dropped.
            Message::Progress(progress) => {
                if self.running.is_some() {
                    self.progress = Some(progress);
                }
            }
            Message::Listed(result) => {
                self.on_listed(result);
                // After a create: offer to prune, quietly (only if there's something to do).
                if std::mem::take(&mut self.prune_after_list) {
                    let prune = self.open_prune(true);
                    return Task::batch([prune, focus_input()]);
                }
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

    /// Starts `apsis --window` (or `--settings`, `--about`) as its own process, and closes the
    /// popup and the menu: the work happens in the window.
    fn launch_window(&mut self, flag: Option<&'static str>) -> Task<cosmic::Action<Message>> {
        let command = window_command(std::env::current_exe().ok(), flag);
        let close_menu = self.close_menu();
        let close_popup = match self.popup.take() {
            Some(popup) => {
                self.reset_popup_state();
                destroy_popup(popup)
            }
            None => Task::none(),
        };
        Task::batch([close_menu, close_popup, spawn(command)])
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
        let width = if self.read_only {
            OVERVIEW_WIDTH
        } else {
            POPUP_WIDTH
        };
        settings.positioner.size_limits = Limits::NONE
            .min_width(width)
            .max_width(width)
            .min_height(1.0)
            .max_height(1080.0);
        let open = close.chain(Task::batch([get_popup(settings), focus_input()]));
        // A restore ran in the browser the popup was closed on.
        let open = Task::batch([open, self.list_if_restored()]);
        if matches!(self.listing, Listing::NotLoaded) {
            Task::batch([open, self.start_list()])
        } else {
            open
        }
    }

    /// The popup closed: drop overlays, the browser, marks and any half-typed prompt. A running
    /// operation (a bulk delete keeps its own list) and its status line stay.
    fn reset_popup_state(&mut self) {
        self.overlay = Overlay::None;
        self.prompt = Prompt::Command;
        self.marked.clear();
        self.browser = None;
        self.pending_restore = None;
    }

    /// Lists in the background, through `apsis-helper`. Does nothing while a list, create or
    /// delete is running (the helper runs one at a time).
    fn start_list(&mut self) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        self.loading = true;
        self.spinner = 0;
        cosmic::task::future(async move { Message::Listed(list_snapshots().await) })
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
                // Marks on snapshots that are gone go too.
                self.marked
                    .retain(|name| list.snapshots.iter().any(|s| &s.name == name));
                self.listing = Listing::Loaded(list);
            }
            Err(error) => {
                self.listing = Listing::Failed(error);
                self.selected = 0;
                self.marked.clear();
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
        // The panel's overview only looks: refresh, open the window, close.
        if self.read_only {
            return match action {
                KeyAction::Escape => self.toggle_popup(),
                KeyAction::Refresh => self.start_list(),
                KeyAction::OpenWindow => self.launch_window(None),
                _ => Task::none(),
            };
        }
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
        if let KeyAction::Room(room) = action {
            return self.go_room(room);
        }
        if self.room == Room::Schedule
            && let Some(task) = self.schedule_key(action)
        {
            return task;
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
            KeyAction::Prune => return self.open_prune(false),
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
                    let names = self.marked_names();
                    self.prompt = if names.is_empty() {
                        Prompt::ConfirmDelete {
                            name: self.snapshots()[self.selected].name.clone(),
                            typed: String::new(),
                        }
                    } else {
                        Prompt::ConfirmDeleteMany {
                            names,
                            typed: String::new(),
                        }
                    };
                }
                return focus_input();
            }
            KeyAction::Toggle | KeyAction::MarkDown
                if matches!(self.overlay, Overlay::None | Overlay::Details) =>
            {
                let Some(name) = self.snapshots().get(self.selected).map(|s| s.name.clone())
                else {
                    return Task::none();
                };
                if !self.marked.remove(&name) {
                    self.marked.insert(name);
                }
                (action == KeyAction::MarkDown)
                    .then(|| self.selected + 1)
                    .filter(|&i| i < count)
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
            | KeyAction::Restore
            // Only the panel's overview opens a window; the window has it already.
            | KeyAction::OpenWindow
            // Handled above, before the list keys.
            | KeyAction::Room(_) => None,
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
            KeyAction::Edit => match view.counted() {
                Some(counted) => {
                    let typed = view.count_of(counted).to_string();
                    self.prompt = Prompt::Count { counted, typed };
                    self.status = None;
                    return focus_input();
                }
                None => Err(fl!("settings-edit-hint")),
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
        cosmic::task::future(async { Message::SettingsRead(read_config().await) })
    }

    fn on_settings_read(
        &mut self,
        result: Result<ConfigInfo, CliError>,
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
        let (expected, config) = (view.info.text.clone(), view.edited.clone());
        self.saving_settings = true;
        self.status = None;
        self.spinner = 0;
        cosmic::task::future(async move {
            Message::SettingsWritten(write_config(expected, config).await)
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
            Prompt::Count { counted, typed } => {
                let SettingsLoad::Ready(view) = &mut self.settings else {
                    return Task::none();
                };
                match view.set_count(counted, &typed) {
                    Ok(()) => self.status = None,
                    Err(reason) => {
                        self.status = Some(Status::Error(reason));
                        self.prompt = Prompt::Count { counted, typed };
                    }
                }
                let choice = view.backend;
                if choice != self.config.backend() {
                    return self.set_backend(choice);
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
                self.run(Operation::Create(comment))
            }
            Prompt::ConfirmDelete { name, typed } => {
                if typed.trim().eq_ignore_ascii_case("y") {
                    self.run(Operation::Delete(name))
                } else {
                    self.status = Some(Status::Info(fl!("delete-cancelled")));
                    Task::none()
                }
            }
            Prompt::ConfirmDeleteMany { names, typed } => {
                if typed.trim().eq_ignore_ascii_case("y") {
                    self.run(Operation::DeleteMany { names, done: 0 })
                } else {
                    self.status = Some(Status::Info(fl!("delete-cancelled")));
                    Task::none()
                }
            }
            Prompt::ConfirmPrune { names, typed } => {
                self.overlay = Overlay::None;
                if typed.trim().eq_ignore_ascii_case("y") {
                    self.run(Operation::DeleteMany { names, done: 0 })
                } else {
                    self.status = Some(Status::Info(fl!("delete-cancelled")));
                    Task::none()
                }
            }
        }
    }

    /// Creates, deletes or restores in the background, through `apsis-helper`.
    fn run(&mut self, operation: Operation) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        self.running = Some(operation.clone());
        self.status = None;
        self.progress = None;
        self.run_started = Some(Instant::now());
        self.spinner = 0;
        if let Operation::Restore(request) = operation {
            return with_progress(move |mut progress| async move {
                let result = restore(&request, &mut progress).await;
                Message::RestoreDone(request, result)
            });
        }
        with_progress(move |mut progress| async move {
            let result = operate(operation.clone(), &mut progress).await;
            Message::Finished(operation, result)
        })
    }

    /// Shows how it went and refreshes the list if anything may have changed (not when polkit
    /// refused, or the helper or the disk isn't there).
    fn on_finished(
        &mut self,
        operation: &Operation,
        result: Result<(), CliError>,
    ) -> Task<cosmic::Action<Message>> {
        self.running = None;
        self.progress = None;
        self.run_started = None;
        if let Operation::DeleteMany { names, done } = operation
            && result.is_ok()
            && done + 1 < names.len()
        {
            // The next one; the password is cached (`auth_admin_keep`) through the helper.
            return self.run(Operation::DeleteMany {
                names: names.clone(),
                done: done + 1,
            });
        }
        let ran = match &result {
            // A failed create or delete may have left something (a staging folder, a partly
            // deleted snapshot): the list shows it.
            Ok(()) | Err(CliError::Other(_)) => true,
            // Nothing ran, or (disk missing) a list would only fail again.
            Err(CliError::NoHelper | CliError::NotAuthorized | CliError::DeviceNotFound { .. }) => {
                false
            }
        };
        // A bulk delete that got past its first snapshot changed the list either way.
        let ran = ran || matches!(operation, Operation::DeleteMany { done, .. } if *done > 0);
        if matches!((operation, &result), (Operation::Create(_), Ok(()))) {
            self.prune_after_list = self.config.keep_manual > 0;
        }
        self.status = Some(match (operation, result) {
            (Operation::Create(_), Ok(())) => Status::Info(fl!("created")),
            (Operation::Delete(name), Ok(())) => {
                Status::Info(fl!("deleted", name = self.snapshot_label(name)))
            }
            (Operation::DeleteMany { names, .. }, Ok(())) => {
                self.marked.clear();
                Status::Info(fl!("deleted-many", count = names.len().to_string()))
            }
            (Operation::DeleteMany { names, done }, Err(error)) => {
                let (deleted, left) = names.split_at((*done).min(names.len()));
                for name in deleted {
                    self.marked.remove(name);
                }
                let reason = error_summary(&error, self.known_uuid.as_deref());
                let labels = |names: &[String]| -> Vec<String> {
                    names.iter().map(|n| self.snapshot_label(n)).collect()
                };
                Status::Error(bulk_delete_failed(&labels(deleted), &labels(left), &reason))
            }
            (Operation::Create(_), Err(error)) => {
                let reason = error_summary(&error, self.known_uuid.as_deref());
                Status::Error(fl!("create-failed", reason = reason))
            }
            (Operation::Delete(_), Err(error)) => {
                let reason = error_summary(&error, self.known_uuid.as_deref());
                Status::Error(fl!("delete-failed", reason = reason))
            }
            // Restores end in `on_restore_done`.
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
        self.browse_calls += 1;
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
        self.browse_calls = self.browse_calls.saturating_sub(1);
        let result = result.map_err(|e| error_summary(&e, self.known_uuid.as_deref()));
        if let Some(browser) = &mut self.browser
            && browser.snapshot == snapshot
        {
            browser.loaded(path, result);
        }
        // The browser may have been closed while this call ran.
        let list = self.list_if_restored();
        // The polkit dialog took keyboard focus; hand it back to the `>` line.
        if self.popup.is_some() {
            return Task::batch([list, focus_input()]);
        }
        list
    }

    /// Lists again after a restore (for the disk line) once the browser is closed and no
    /// `Browse` call holds the helper's lock; until then the flag waits.
    fn list_if_restored(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.list_after_browser || self.browser.is_some() || self.browse_calls > 0 {
            return Task::none();
        }
        self.list_after_browser = false;
        self.start_list()
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
        self.progress = None;
        self.run_started = None;
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
                // A restore can write to the backup disk (a home on it), so the disk line is
                // refreshed: at once if the browser is gone, else when it's left. Its folder
                // reload holds the helper's lock, and a list beside it would be refused as busy.
                if let Some(browser) = &mut self.browser {
                    let path = browser.reload();
                    tasks.push(self.fetch_browse(path));
                    self.list_after_browser = true;
                } else {
                    self.list_after_browser = true;
                    tasks.push(self.list_if_restored());
                }
            }
        }
        // The polkit dialog took keyboard focus; hand it back to the `>` line.
        if self.popup.is_some() {
            tasks.push(focus_input());
        }
        Task::batch(tasks)
    }

    /// Saves Apsis's own settings (changed in the settings view) to cosmic-config.
    fn set_backend(&mut self, choice: ApsisChoice) -> Task<cosmic::Action<Message>> {
        let saved =
            cosmic_config::Config::new(<Self as cosmic::Application>::APP_ID, Config::VERSION)
                .and_then(|context| {
                    self.config.set_keep_manual(&context, choice.keep_manual)?;
                    self.config.set_remind_days(&context, choice.remind_days)?;
                    self.config.set_show_label(&context, choice.show_label)
                });
        self.status = Some(match saved {
            Ok(_) => Status::Info(fl!("settings-apsis-saved")),
            Err(error) => Status::Error(fl!("backend-save-failed", reason = error.to_string())),
        });
        Task::none()
    }

    /// Esc cancels a prompt, then closes the overlay (a restore plan goes back to the
    /// browser), then clears the snapshot marks, then closes the popup (or the window).
    fn escape(&mut self) -> Task<cosmic::Action<Message>> {
        if self.prompt != Prompt::Command {
            self.prompt = Prompt::Command;
            self.status = None;
            // The prune preview only goes with its question.
            if self.overlay == Overlay::Prune {
                self.overlay = Overlay::None;
            }
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
                return Task::batch([self.list_if_restored(), focus_input()]);
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
            view.discard();
            self.status = None;
        }
        if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
            // Esc also unfocused the `>` line.
            return focus_input();
        }
        if self.room != Room::Snapshots {
            self.room = Room::Snapshots;
            return focus_input();
        }
        if !self.marked.is_empty() {
            self.marked.clear();
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

    /// A dock cell or `1 2 3 4`: the room's content replaces the details (create) or both
    /// panes (schedule, log). Create also starts the comment prompt, as `c` does.
    fn go_room(&mut self, room: Room) -> Task<cosmic::Action<Message>> {
        if self.mode != Mode::Window || self.running.is_some() {
            return Task::none();
        }
        self.room = room;
        if matches!(
            self.overlay,
            Overlay::Details | Overlay::Help | Overlay::About
        ) {
            self.overlay = Overlay::None;
        }
        if room == Room::Create && self.prompt == Prompt::Command {
            return self.on_key(KeyAction::Create);
        }
        focus_input()
    }

    /// Keys in the schedule room: up and down pick keep or remind, space turns it on or off,
    /// `+` and `-` change it. `None` for any other key: it acts as everywhere else.
    fn schedule_key(&mut self, action: KeyAction) -> Option<Task<cosmic::Action<Message>>> {
        let counted = if self.schedule_row == 0 {
            Counted::KeepManual
        } else {
            Counted::Remind
        };
        match action {
            KeyAction::Up | KeyAction::First => self.schedule_row = 0,
            KeyAction::Down | KeyAction::Last => self.schedule_row = 1,
            KeyAction::Toggle => return Some(self.step_setting(counted, Step::Toggle)),
            KeyAction::More => return Some(self.step_setting(counted, Step::More)),
            KeyAction::Less => return Some(self.step_setting(counted, Step::Less)),
            _ => return None,
        }
        Some(Task::none())
    }

    /// Steps keep or remind and saves it, if that changed anything.
    fn step_setting(&mut self, counted: Counted, step: Step) -> Task<cosmic::Action<Message>> {
        self.schedule_row = match counted {
            Counted::KeepManual => 0,
            Counted::Remind => 1,
        };
        let choice = self.config.backend().stepped(counted, step);
        if choice == self.config.backend() {
            return Task::none();
        }
        self.set_backend(choice)
    }

    /// Writes to the log room what a message left in the status line, and a list that failed.
    fn log_changes(&mut self, before: Option<&Status>, list_failed: bool) {
        let mut new = Vec::new();
        if let Some(status) = &self.status
            && Some(status) != before
        {
            new.push(match status {
                Status::Info(text) => (text.clone(), false),
                Status::Error(text) => (text.clone(), true),
            });
        }
        if let Listing::Failed(error) = &self.listing
            && !list_failed
        {
            new.push((error_summary(error, self.known_uuid.as_deref()), true));
        }
        if new.is_empty() {
            return;
        }
        let time = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
        for (text, error) in new {
            self.log.push(LogEntry {
                time: time.clone(),
                text,
                error,
            });
        }
        if self.log.len() > LOG_LINES {
            self.log.drain(..self.log.len() - LOG_LINES);
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

    /// "Keep last N manual" for the current list: what goes and what stays. `None` when it's
    /// off or nothing is listed.
    fn prune_plan(&self) -> Option<ManualPlan> {
        let keep = self.config.keep_manual;
        match &self.listing {
            Listing::Loaded(list) if keep > 0 => Some(retention::manual(&list.snapshots, keep)),
            _ => None,
        }
    }

    /// Shows the prune preview and asks `y` before anything is deleted. `quiet` (after a
    /// create): says nothing when it's off or there's nothing to prune.
    fn open_prune(&mut self, quiet: bool) -> Task<cosmic::Action<Message>> {
        // The popup (or window) must be open: the question is asked there.
        if self.loading || self.running.is_some() || self.popup.is_none() {
            return Task::none();
        }
        let status = match self.prune_plan() {
            None => Some(fl!("prune-off")),
            Some(plan) if plan.is_empty() => Some(fl!(
                "prune-nothing",
                keep = self.config.keep_manual.to_string()
            )),
            Some(plan) => {
                self.overlay = Overlay::Prune;
                self.status = None;
                self.prompt = Prompt::ConfirmPrune {
                    names: plan.delete,
                    typed: String::new(),
                };
                return focus_input();
            }
        };
        if !quiet {
            self.status = status.map(Status::Info);
        }
        Task::none()
    }

    /// The prune preview's lines: what goes (oldest first), then what stays and why.
    fn prune_lines(&self) -> Vec<String> {
        let Some(plan) = self.prune_plan() else {
            return Vec::new();
        };
        let keep = self.config.keep_manual.to_string();
        let mut lines: Vec<String> = plan
            .delete
            .iter()
            .map(|name| fl!("prune-delete", name = self.snapshot_label(name)))
            .collect();
        lines.extend(plan.kept.iter().map(|(name, why)| {
            let label = self.snapshot_label(name);
            match why {
                Kept::Recent => fl!("prune-keep-recent", name = label, count = keep.clone()),
                Kept::Comment => fl!("prune-keep-comment", name = label),
                Kept::Newest => fl!("prune-keep-newest", name = label),
            }
        }));
        lines
    }

    /// `09-27 09:12 "bulk 1"` for snapshot `name` in the last list (see [`fmt::label`]), or
    /// `name` itself if it isn't there.
    fn snapshot_label(&self, name: &str) -> String {
        self.snapshots()
            .iter()
            .find(|s| s.name == name)
            .map_or_else(|| name.to_owned(), fmt::label)
    }

    /// Labels of `names`, joined with commas.
    fn snapshot_labels(&self, names: &[String]) -> String {
        names
            .iter()
            .map(|name| self.snapshot_label(name))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The marked snapshots' names, in list order (newest first).
    fn marked_names(&self) -> Vec<String> {
        self.snapshots()
            .iter()
            .filter(|s| self.marked.contains(&s.name))
            .map(|s| s.name.clone())
            .collect()
    }

    /// What the panel shows, from the last list and the reminder setting. The one place the
    /// panel label, tooltip and icon colour come from.
    fn status_view(&self) -> StatusView {
        self.status_view_at(jiff::Zoned::now().datetime())
    }

    fn status_view_at(&self, now: jiff::civil::DateTime) -> StatusView {
        match &self.listing {
            Listing::Loaded(list) => {
                StatusView::Loaded(ApsisStatus::from_list(list, self.config.remind_days, now))
            }
            Listing::Failed(_) => StatusView::Failed,
            Listing::NotLoaded => StatusView::Unloaded,
        }
    }

    /// The panel icon, coloured by the theme's warning or destructive role.
    fn panel_icon(&self, severity: Severity) -> icon::Icon {
        let (width, height) = self.core.applet.suggested_size(true);
        let class: fn(&Theme) -> iced_svg::Style = match severity {
            Severity::None => plain_svg,
            Severity::Warning => warning_svg,
            Severity::Critical => critical_svg,
        };
        icon::icon(self.icon.clone())
            .class(theme::Svg::Custom(Rc::new(class)))
            .width(Length::Fixed(f32::from(width)))
            .height(Length::Fixed(f32::from(height)))
    }

    /// The panel button with the label beside the icon. Only the icon takes a warning colour.
    fn label_button(&self, status: &StatusView) -> widget::Button<'_, Message> {
        let (_, height) = self.core.applet.suggested_size(true);
        let (major, minor) = self.core.applet.suggested_padding(true);
        let content = widget::row::with_children(vec![
            self.panel_icon(status.severity()).into(),
            monotext(status.label()).into(),
        ])
        .spacing(6)
        .align_y(Alignment::Center);
        widget::button::custom(content)
            .padding([0, major])
            .height(Length::Fixed(f32::from(height + 2 * minor)))
            .class(theme::Button::AppletIcon)
    }
}

// View helpers. Everything is monotext; colours come from the theme.
impl AppModel {
    /// The popup's contents: header, panes, activity, `>` line and hints. The popup wraps it in a
    /// popup container; in window mode it fills the window.
    fn surface(&self) -> Element<'_, Message> {
        // The details pane is active after Enter or a double-click; otherwise the left pane is.
        let details_active = self.overlay == Overlay::Details;
        let panes: Element<'_, Message> = match (self.mode, self.room) {
            (Mode::Window, Room::Schedule) => pane(
                fl!("pane-schedule"),
                true,
                Length::Fill,
                self.schedule_body(),
            ),
            (Mode::Window, Room::Log) => pane(fl!("pane-log"), true, Length::Fill, self.log_body()),
            (mode, room) => {
                let right = if mode == Mode::Window && room == Room::Create {
                    pane(
                        fl!("pane-create"),
                        matches!(self.prompt, Prompt::Comment(_)),
                        Length::FillPortion(2),
                        self.create_body(),
                    )
                } else {
                    pane(
                        fl!("pane-details"),
                        details_active,
                        // A fixed width lets the row size the details pane first, so the list
                        // beside it can stretch to its height (see `settings_details_grow`).
                        if self.settings_details_grow() {
                            Length::Fixed(POPUP_DETAILS_WIDTH)
                        } else {
                            Length::FillPortion(2)
                        },
                        self.details(),
                    )
                };
                widget::row::with_children(vec![
                    pane(
                        self.body_title(),
                        !details_active,
                        Length::FillPortion(3),
                        self.body(),
                    ),
                    right,
                ])
                .spacing(PANE_SPACING)
                // A popup's panes are as high as their content (see `settings_details_grow`); a
                // window's fill it.
                .height(match mode {
                    Mode::Applet => Length::Shrink,
                    Mode::Window => Length::Fill,
                })
                .into()
            }
        };
        let mut children = vec![self.header()];
        children.extend(self.strip());
        children.push(panes);
        // The window has the strip; the popup keeps the disk line until it becomes the overview.
        if self.mode == Mode::Applet {
            children.extend(self.disk_line());
        }
        children.extend([self.activity(), self.prompt()]);
        children.extend(self.dock());
        children.push(self.hints());
        widget::column::with_children(children)
            .spacing(6)
            .padding([10.0, POPUP_PADDING])
            .into()
    }

    /// The `apsis` pane above the panes, window only: time on the left (`last`, `next`), the
    /// backup disk on the right (device and sizes, the bar, used and free). From
    /// [`StatusView::strip`], so it says what the panel tooltip says. Not in the settings view,
    /// whose notes are sized to fit the smallest window without it.
    fn strip(&self) -> Option<Element<'_, Message>> {
        if self.mode != Mode::Window || self.overlay == Overlay::Settings {
            return None;
        }
        let (time, disk) = strip_columns(self.status_view().strip());
        Some(pane(
            fl!("pane-apsis"),
            false,
            Length::Fill,
            widget::row::with_children(vec![
                container(time).width(Length::FillPortion(1)).into(),
                container(disk).width(Length::FillPortion(1)).into(),
            ])
            .spacing(24),
        ))
    }

    /// The panel popup: a read-only overview in the same terminal look, narrower than the
    /// window. `~/apsis $ status`, the `apsis` pane (time, then disk), the newest snapshots, and
    /// the keys that work here.
    fn overview(&self) -> Element<'_, Message> {
        let (time, disk) = strip_columns(self.status_view().strip());
        let apsis = pane(
            fl!("pane-apsis"),
            false,
            Length::Fill,
            widget::column::with_children(vec![time, disk]).spacing(6),
        );
        let snapshots = pane(
            fl!("pane-snapshots"),
            false,
            Length::Fill,
            self.overview_snapshots(),
        );
        let refresh = if matches!(self.listing, Listing::Failed(_)) {
            "[r]etry"
        } else {
            "[r]efresh"
        };
        let hints = widget::row::with_children(vec![
            hint("[o]pen apsis", Some(Message::OpenWindow(None))),
            hint(refresh, (!self.loading).then_some(Message::Refresh)),
            widget::space::horizontal().into(),
            hint("[esc]", Some(Message::Escape)),
        ])
        .spacing(8)
        .align_y(Alignment::Center);
        widget::column::with_children(vec![self.overview_header(), apsis, snapshots, hints.into()])
            .spacing(6)
            .padding([10.0, POPUP_PADDING])
            .into()
    }

    /// `~/apsis $ status                 rsync · 9 snapshots`
    fn overview_header(&self) -> Element<'_, Message> {
        let summary = match &self.listing {
            Listing::Loaded(list) => fmt::summary(list.mode, list.snapshots.len()),
            Listing::NotLoaded | Listing::Failed(_) => String::new(),
        };
        widget::row::with_children(vec![
            icon::icon(self.icon.clone())
                .size(HEADER_ICON_SIZE)
                .class(theme::Svg::Custom(Rc::new(accent_svg)))
                .into(),
            monotext("~/apsis").class(theme::Text::Accent).into(),
            monotext("$ status").into(),
            widget::space::horizontal().into(),
            monotext(summary)
                .class(theme::Text::Custom(dim_text))
                .into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// The overview's newest snapshots as plain rows (no selection, nothing to click), or what
    /// the list says when there are none: no device, empty, failed, waiting.
    fn overview_snapshots(&self) -> Element<'_, Message> {
        match &self.listing {
            Listing::Loaded(list) if !list.snapshots.is_empty() => {
                let mut rows: Vec<Element<'_, Message>> = list
                    .snapshots
                    .iter()
                    .take(OVERVIEW_ROWS)
                    .map(|snapshot| {
                        monotext(fmt::overview_row(snapshot, OVERVIEW_COMMENT_CHARS))
                            .wrapping(Wrapping::None)
                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                            .into()
                    })
                    .collect();
                if let Some(more) = fmt::older_count(list.snapshots.len(), OVERVIEW_ROWS) {
                    rows.push(
                        monotext(fl!("overview-more", count = more.to_string()))
                            .class(theme::Text::Custom(dim_text))
                            .into(),
                    );
                }
                widget::column::with_children(rows)
                    .spacing(2)
                    .padding([4, 6])
                    .into()
            }
            Listing::Loaded(list) if list.device.is_none() => {
                lines([fl!("no-device"), fl!("no-device-hint")])
            }
            Listing::Loaded(_) => lines([fl!("empty")]),
            Listing::Failed(error) => error_view(error, self.known_uuid.as_deref()),
            Listing::NotLoaded if self.loading => lines([fl!("waiting")]),
            Listing::NotLoaded => lines([fl!("not-loaded")]),
        }
    }

    /// The dock, window only: four outlined cells in the accent colour; the active room's cell
    /// has the selected row's fill. Not in the settings view or the browser, which have their own
    /// footers and need the room.
    fn dock(&self) -> Option<Element<'_, Message>> {
        if self.mode != Mode::Window || !matches!(self.overlay, Overlay::None | Overlay::Details) {
            return None;
        }
        let cells: Vec<Element<'_, Message>> = Room::ALL
            .into_iter()
            .map(|room| {
                let active = self.room == room;
                widget::mouse_area(
                    container(
                        monotext(room.label())
                            .class(theme::Text::Accent)
                            .width(Length::Fill)
                            .align_x(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([3, 8])
                    .class(theme::Container::custom(move |theme| {
                        dock_cell(theme, active)
                    })),
                )
                .on_press(Message::Room(room))
                .interaction(mouse::Interaction::Pointer)
                .into()
            })
            .collect();
        Some(
            widget::row::with_children(cells)
                .spacing(PANE_SPACING)
                .into(),
        )
    }

    /// The create room's form, beside the list: what will be made, and the two keys. The text
    /// itself is typed on the `>` line (one input, one focus); this shows it as it goes.
    fn create_body(&self) -> Element<'_, Message> {
        let label = |text: String| {
            monotext(text)
                .class(theme::Text::Accent)
                .width(Length::Fixed(MONO_CELL_WIDTH * 9.0))
        };
        let value = match &self.prompt {
            Prompt::Comment(comment) => format!("{comment}▏"),
            _ => fl!("create-comment-none"),
        };
        let mut children: Vec<Element<'_, Message>> = vec![
            widget::row::with_children(vec![
                label(fl!("create-label-comment")).into(),
                monotext(value)
                    .wrapping(Wrapping::WordOrGlyph)
                    .width(Length::Fill)
                    .into(),
            ])
            .spacing(8)
            .into(),
            widget::row::with_children(vec![
                label(fl!("create-label-tag")).into(),
                monotext(fl!("create-tag")).into(),
            ])
            .spacing(8)
            .into(),
        ];
        let asking = matches!(self.prompt, Prompt::Comment(_));
        children.push(
            widget::row::with_children(if asking {
                vec![
                    hint("[enter]create", Some(Message::Submit)),
                    hint("[esc]cancel", Some(Message::Escape)),
                ]
            } else {
                vec![hint(
                    "[c]reate",
                    self.can_create().then_some(Message::StartCreate),
                )]
            })
            .spacing(8)
            .into(),
        );
        if !asking && !self.can_create() {
            children.push(
                monotext(fl!("create-unavailable"))
                    .class(theme::Text::Custom(dim_text))
                    .wrapping(Wrapping::WordOrGlyph)
                    .into(),
            );
        }
        widget::column::with_children(children)
            .spacing(6)
            .padding([4, 6])
            .into()
    }

    /// The schedule room: `next`, and the two settings there are (keep and remind), with their
    /// buttons. There is no scheduler yet.
    fn schedule_body(&self) -> Element<'_, Message> {
        let choice = self.config.backend();
        let label = |text: String| {
            monotext(text)
                .class(theme::Text::Accent)
                .width(Length::Fixed(MONO_CELL_WIDTH * 8.0))
        };
        let dim = |text: String| {
            monotext(text)
                .class(theme::Text::Custom(dim_text))
                .wrapping(Wrapping::WordOrGlyph)
        };
        let setting = |row: usize, counted: Counted, name: String, text: String, on: bool| {
            let mark = if self.schedule_row == row { "▸" } else { " " };
            widget::row::with_children(vec![
                monotext(mark).class(theme::Text::Accent).into(),
                label(name).into(),
                monotext(text).width(Length::Fill).into(),
                hint("[-]", Some(Message::Schedule(counted, Step::Less))),
                hint("[+]", Some(Message::Schedule(counted, Step::More))),
                hint(
                    if on { "[off]" } else { "[on]" },
                    Some(Message::Schedule(counted, Step::Toggle)),
                ),
            ])
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        };
        let keep = match choice.keep_manual {
            0 => fl!("settings-keep-manual-off"),
            count => fl!("settings-keep-manual", count = count.to_string()),
        };
        let remind = match choice.remind_days {
            0 => fl!("settings-remind-off"),
            days => fl!("settings-remind", days = days.to_string()),
        };
        let content = widget::column::with_children(vec![
            widget::row::with_children(vec![
                monotext(" ").into(),
                label(fl!("strip-label-next")).into(),
                monotext(fl!("strip-next-manual")).into(),
            ])
            .spacing(8)
            .into(),
            setting(
                0,
                Counted::KeepManual,
                fl!("schedule-label-keep"),
                keep,
                choice.keep_manual > 0,
            ),
            dim(fl!("settings-keep-manual-note")).into(),
            setting(
                1,
                Counted::Remind,
                fl!("schedule-label-remind"),
                remind,
                choice.remind_days > 0,
            ),
            dim(fl!("settings-remind-note")).into(),
            dim(fl!("schedule-note")).into(),
        ])
        .spacing(6)
        .padding([4, 6]);
        // Scrolls in a small window rather than pushing the dock and footer out of it.
        widget::scrollable(content)
            .direction(Direction::Vertical(thin_scrollbar()))
            .height(Length::Fill)
            .into()
    }

    /// The log room: what happened this session, newest first. A failure is marked by the word
    /// `error` in the error colour; the rest is text.
    fn log_body(&self) -> Element<'_, Message> {
        if self.log.is_empty() {
            return lines([fl!("log-empty")]);
        }
        let rows = self.log.iter().rev().map(|entry| {
            let mut parts: Vec<Element<'_, Message>> = vec![
                monotext(entry.time.clone())
                    .class(theme::Text::Custom(dim_text))
                    .into(),
            ];
            if entry.error {
                parts.push(
                    monotext(fl!("log-error"))
                        .class(theme::Text::Custom(error_text))
                        .into(),
                );
            }
            parts.push(
                monotext(entry.text.clone())
                    .wrapping(Wrapping::WordOrGlyph)
                    .width(Length::Fill)
                    .into(),
            );
            widget::row::with_children(parts).spacing(8).into()
        });
        widget::scrollable(
            widget::column::with_children(rows.collect::<Vec<_>>())
                .spacing(2)
                .padding([4, 6]),
        )
        .direction(Direction::Vertical(thin_scrollbar()))
        .height(Length::Fill)
        .into()
    }

    /// ` ~/apsis $ ls --snapshots                 rsync · 3 snapshots`
    fn header(&self) -> Element<'_, Message> {
        let summary = match &self.listing {
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

    /// `disk  sdX1  ████████░░░░  448G used · 483G free · 9 snapshots` under the panes, from
    /// the last list. Without `statvfs` numbers, no line at all. Not in the settings view: its notes are sized to fit the
    /// smallest window without it (`settings_details_fit...` test).
    fn disk_line(&self) -> Option<Element<'_, Message>> {
        let Listing::Loaded(list) = &self.listing else {
            return None;
        };
        if self.overlay == Overlay::Settings {
            return None;
        }
        let text = fmt::disk_text(list.usage, list.snapshots.len())?;
        let mut children = vec![
            monotext(fl!("disk-label"))
                .class(theme::Text::Custom(dim_text))
                .into(),
        ];
        if let Some(device) = &list.device {
            children.push(monotext(fmt::device_name(device).to_owned()).into());
        }
        match list.usage {
            Some(usage) => children.push(disk_bar(usage)),
            None => children.push(widget::space::horizontal().into()),
        }
        children.push(
            monotext(text)
                .wrapping(Wrapping::None)
                .class(theme::Text::Custom(dim_text))
                .into(),
        );
        Some(
            widget::row::with_children(children)
                .spacing(16)
                .padding([0.0, TITLE_INSET])
                .align_y(Alignment::Center)
                .into(),
        )
    }

    /// The left pane's title: what it shows.
    fn body_title(&self) -> String {
        match self.overlay {
            Overlay::Help => fl!("pane-help"),
            Overlay::About => fl!("pane-about"),
            Overlay::Prune => fl!("pane-prune"),
            Overlay::Browse => self.browse_title(),
            Overlay::RestorePlan if self.pending_restore.is_some() => fl!("pane-restore-plan"),
            Overlay::RestorePlan => fl!("pane-restore-result"),
            Overlay::Settings => match &self.settings {
                SettingsLoad::Ready(view) if view.dirty() => fl!("pane-settings-unsaved"),
                _ => fl!("pane-settings"),
            },
            Overlay::None | Overlay::Details if self.marked.is_empty() => fl!("pane-snapshots"),
            Overlay::None | Overlay::Details => {
                let marked = fl!("browse-marked", count = self.marked.len().to_string());
                format!("{} · {marked}", fl!("pane-snapshots"))
            }
        }
    }

    /// Height of the snapshots and details panes' contents, in rows: the list's length between
    /// [`MIN_ROWS`] and [`VISIBLE_ROWS`] (longer lists scroll), or enough for what's shown instead.
    fn pane_rows(&self) -> u16 {
        match (self.overlay, &self.listing) {
            (
                Overlay::Help
                | Overlay::Settings
                | Overlay::Prune
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

    /// Popup settings view: the details pane is as high as the tallest row's details (at least
    /// [`VISIBLE_ROWS`]), so nothing scrolls or is cut off and the popup keeps one height while
    /// moving between rows; the settings list stretches to match. A window's panes have the
    /// window's height, so there the notes are kept short enough to fit.
    fn settings_details_grow(&self) -> bool {
        self.mode == Mode::Applet && self.overlay == Overlay::Settings
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
            (Overlay::Prune, _) => scroll(lines(self.prune_lines())),
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
            .height(if self.settings_details_grow() {
                // As high as the details pane beside it.
                Length::Fill
            } else {
                self.pane_height()
            })
            .into()
    }

    /// The right pane: everything about the selected snapshot.
    fn details(&self) -> Element<'_, Message> {
        if self.settings_details_grow() {
            // Unscrolled, as high as the tallest row's details, beside a zero-width spacer that
            // keeps it at least as high as a list.
            let min_height = ROW_HEIGHT * f32::from(VISIBLE_ROWS);
            return widget::row::with_children(vec![
                container(self.settings_details_tallest())
                    .width(Length::Fill)
                    .into(),
                widget::space::vertical()
                    .height(Length::Fixed(min_height))
                    .into(),
            ])
            .into();
        }
        let content = match (self.overlay, self.snapshots().get(self.selected)) {
            (Overlay::Settings, _) => scroll(self.settings_details()),
            (Overlay::Browse | Overlay::RestorePlan, _) => {
                match self.browser.as_ref().and_then(|b| Some((b, b.current()?))) {
                    Some((browser, entry)) => scroll(entry_details(browser, entry)),
                    None => lines([]),
                }
            }
            (_, Some(snapshot)) => scroll(details(snapshot, self.marked.contains(&snapshot.name))),
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
                format!(
                    "{} {spinner}",
                    fl!("deleting", name = self.snapshot_label(name))
                ),
                Tone::Normal,
            ),
            (Some(Operation::DeleteMany { names, done }), _) => (
                format!(
                    "{} {spinner}",
                    fl!(
                        "deleting-many",
                        step = (done + 1).to_string(),
                        count = names.len().to_string(),
                        name = names
                            .get(*done)
                            .map(|name| self.snapshot_label(name))
                            .unwrap_or_default()
                    )
                ),
                Tone::Normal,
            ),
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

    /// The progress line for a running create or real restore, once the helper has sent a
    /// `Progress` (see [`AppModel::progress`]). `None` otherwise: the spinner line.
    fn progress_line(&self) -> Option<ProgressLine> {
        self.progress_line_at(Instant::now())
    }

    fn progress_line_at(&self, now: Instant) -> Option<ProgressLine> {
        let label = match &self.running {
            Some(Operation::Create(_)) => fl!("progress-creating"),
            Some(Operation::Restore(request)) if !request.dry_run => fl!("progress-restoring"),
            _ => return None,
        };
        let progress = self.progress.as_ref()?;
        let percent = match progress.percent {
            Some(percent) if progress.has_estimate() => percent,
            _ => {
                let elapsed = self.run_started.map_or(0, |started| {
                    now.saturating_duration_since(started).as_secs()
                });
                return Some(ProgressLine::Working(format!(
                    "{} {}",
                    fl!(
                        "progress-working",
                        label = label,
                        elapsed = fmt::duration(elapsed)
                    ),
                    SPINNER[self.spinner]
                )));
            }
        };
        let text = match progress.eta_seconds {
            Some(eta) => fl!(
                "progress-percent-left",
                label = label,
                percent = fmt::percent(percent),
                time = fmt::duration(eta)
            ),
            None => fl!(
                "progress-percent",
                label = label,
                percent = fmt::percent(percent)
            ),
        };
        Some(ProgressLine::Bar {
            text,
            fraction: percent / 100.0,
        })
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
        let first: Element<'_, Message> = match self.progress_line() {
            Some(ProgressLine::Working(text)) => monotext(text).into(),
            Some(ProgressLine::Bar { text, fraction }) => widget::row::with_children(vec![
                monotext(text).wrapping(Wrapping::None).into(),
                bar(fraction, theme::Text::Custom(accent_text)),
            ])
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            None => {
                let (text, tone) = self.activity_line();
                let line = monotext(text).wrapping(Wrapping::WordOrGlyph);
                match tone {
                    Tone::Normal => line,
                    Tone::Dim => line.class(theme::Text::Custom(dim_text)),
                    Tone::Error => line.class(theme::Text::Custom(error_text)),
                }
                .into()
            }
        };
        let mut lines = vec![first];
        lines.extend(self.list_warnings().iter().map(|warning| {
            monotext(fl!("activity-list-warning", warning = warning.clone()))
                .wrapping(Wrapping::WordOrGlyph)
                .class(theme::Text::Custom(warning_text))
                .into()
        }));
        // Before the first save: what was imported from Timeshift's settings.
        if self.overlay == Overlay::Settings
            && let SettingsLoad::Ready(view) = &self.settings
            && view.imported()
            && view.dirty()
        {
            lines.extend(view.info.imported.iter().map(|note| {
                monotext(note.clone())
                    .wrapping(Wrapping::WordOrGlyph)
                    .class(theme::Text::Custom(warning_text))
                    .into()
            }));
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

    /// `▸ * 2026-09-25 03:00   D     "comment"`: `*` if marked for deletion.
    fn row<'a>(&self, index: usize, snapshot: &'a Snapshot) -> Element<'a, Message> {
        let selected = index == self.selected;
        let marked = self.marked.contains(&snapshot.name);
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
            monotext(if marked { "*" } else { " " })
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
        } else if marked {
            row = row.class(theme::Container::custom(marked_row));
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
        let mut keys = vec![
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
        ];
        // The dock's keys, as plain text: each cell is clickable, and `1 2 3 4` do the same.
        if self.mode == Mode::Window {
            keys.push(
                monotext("[1-4]rooms")
                    .class(theme::Text::Custom(dim_text))
                    .into(),
            );
        }
        keys.push(widget::space::horizontal().into());
        keys.push(hint("[esc]", Some(Message::Escape)));
        widget::row::with_children(keys)
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
        settings_row_details(view, view.current())
    }

    /// The popup settings view's right pane: the selected row's details, in a space as high as
    /// the tallest row's, so the popup keeps its height while moving between rows.
    fn settings_details_tallest(&self) -> Element<'_, Message> {
        let SettingsLoad::Ready(view) = &self.settings else {
            return lines([]);
        };
        let all = view
            .rows()
            .into_iter()
            .map(|row| settings_row_details(view, row))
            .collect();
        Tallest::new(all, view.cursor).into()
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
            menu_button(body(fl!("menu-open")))
                .on_press(Message::OpenWindow(None))
                .into(),
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
            menu_button(body(fl!("menu-remove-or-move")))
                .on_press(Message::MenuPanelSettings)
                .into(),
            menu_button(body(fl!("menu-close")))
                .on_press(Message::MenuClose)
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
                Some(fl!("prompt-delete", name = self.snapshot_label(name))),
                typed.as_str(),
                String::new(),
            ),
            Prompt::ConfirmDeleteMany { names, typed } => (
                Some(fl!(
                    "prompt-delete-many",
                    count = names.len().to_string(),
                    names = self.snapshot_labels(names)
                )),
                typed.as_str(),
                String::new(),
            ),
            Prompt::ConfirmPrune { names, typed } => (
                Some(fl!("prompt-prune", count = names.len().to_string())),
                typed.as_str(),
                String::new(),
            ),
            Prompt::Filter(pattern) => {
                (Some(fl!("prompt-filter")), pattern.as_str(), String::new())
            }
            Prompt::Count { counted, typed } => (
                Some(match counted {
                    Counted::KeepManual => fl!("prompt-keep-manual"),
                    Counted::Remind => fl!("prompt-remind"),
                }),
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
        // A delete's snapshot labels (with their comments) can be longer than the row: the label
        // takes the row and wraps, and the input (only ever `y`) keeps a small fixed width.
        let long_label = matches!(
            prompt,
            Prompt::ConfirmDelete { .. } | Prompt::ConfirmDeleteMany { .. }
        );
        let input = widget::text_input::inline_input(placeholder, value)
            .width(if long_label {
                Length::Fixed(SHORT_INPUT_WIDTH)
            } else {
                Length::Fill
            })
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
            monotext(label)
                .width(if long_label {
                    Length::Fill
                } else {
                    Length::Shrink
                })
                .wrapping(Wrapping::WordOrGlyph)
                .into(),
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
        // Home folder patterns are shown dimmed: the home rows above change them.
        Row::Filter(index) => {
            let filter = &view.edited.filters[index];
            (filter.clone(), view.is_home_pattern(filter))
        }
        Row::AddFilter => (fl!("settings-add-filter"), true),
        Row::KeepManual => match view.backend.keep_manual {
            0 => (fl!("settings-keep-manual-off"), true),
            keep => (fl!("settings-keep-manual", count = keep.to_string()), false),
        },
        Row::Remind => match view.backend.remind_days {
            0 => (fl!("settings-remind-off"), true),
            days => (fl!("settings-remind", days = days.to_string()), false),
        },
        Row::PanelLabel => match view.backend.show_label {
            false => (fl!("settings-panel-label-off"), true),
            true => (fl!("settings-panel-label"), false),
        },
    }
}

/// What settings row `row` means: its values, then a note. Text only; it wraps.
fn settings_row_details(view: &SettingsView, row: Row) -> Element<'static, Message> {
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
                [
                    fl!("settings-home-excluded-note"),
                    fl!("settings-home-hidden-note"),
                    fl!("settings-home-all-note"),
                ]
                .join("\n")
            }
        }
        Row::Filter(index) => {
            let pattern = &view.edited.filters[index];
            let kind = if view.is_home_pattern(pattern) {
                fl!("settings-filter-home")
            } else if pattern.starts_with("+ ") {
                fl!("settings-filter-include")
            } else {
                fl!("settings-filter-exclude")
            };
            pairs.push((fl!("settings-key-kind"), kind));
            fl!("settings-filter-note")
        }
        Row::AddFilter => fl!("settings-add-note"),
        Row::KeepManual => fl!("settings-keep-manual-note"),
        Row::Remind => fl!("settings-remind-note"),
        Row::PanelLabel => fl!("settings-panel-label-note"),
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

fn section_name(section: Section) -> String {
    match section {
        Section::Device => fl!("settings-device"),
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

/// The strip's two columns as widgets: time (`last`, `next`) and disk (device and sizes, the
/// bar, used and free). The window puts them side by side, the popup's overview stacks them.
fn strip_columns(strip: Strip) -> (Element<'static, Message>, Element<'static, Message>) {
    let label = |text: String| {
        monotext(text)
            .class(theme::Text::Accent)
            .width(Length::Fixed(MONO_CELL_WIDTH * 7.0))
    };
    let note = |text: String, severity: Severity| {
        monotext(text).class(match severity {
            Severity::Critical => theme::Text::Custom(error_text),
            Severity::Warning | Severity::None => theme::Text::Custom(warning_text),
        })
    };

    let mut last = vec![
        label(fl!("strip-label-last")).into(),
        monotext(strip.last).into(),
    ];
    if let Some(text) = strip.last_note {
        last.push(note(text, Severity::Warning).into());
    }
    let time = widget::column::with_children(vec![
        widget::row::with_children(last).spacing(8).into(),
        widget::row::with_children(vec![
            label(fl!("strip-label-next")).into(),
            monotext(strip.next).into(),
        ])
        .spacing(8)
        .into(),
    ]);

    let disk = match strip.disk {
        DiskStrip::Mounted {
            device,
            size,
            usage,
            used_free,
            note: low,
        } => {
            let mut free = vec![
                monotext(used_free)
                    .wrapping(Wrapping::None)
                    .class(theme::Text::Custom(dim_text))
                    .into(),
            ];
            if let Some((text, severity)) = low {
                free.push(note(text, severity).wrapping(Wrapping::None).into());
            }
            widget::column::with_children(vec![
                widget::row::with_children(vec![
                    monotext(device).class(theme::Text::Accent).into(),
                    monotext(size).into(),
                ])
                .spacing(8)
                .into(),
                disk_bar(usage),
                widget::row::with_children(free).spacing(8).into(),
            ])
        }
        DiskStrip::Text(text) => widget::column::with_children(vec![
            widget::row::with_children(vec![
                label(fl!("disk-label")).into(),
                monotext(text).into(),
            ])
            .spacing(8)
            .into(),
        ]),
    };
    (time.into(), disk.into())
}

/// The disk bar: [`bar`] in the accent, warning or destructive colour by [`Space`].
fn disk_bar(usage: DiskUsage) -> Element<'static, Message> {
    let class = match Space::of(&usage) {
        Space::Plenty => theme::Text::Custom(accent_text),
        Space::Low => theme::Text::Custom(warning_text),
        Space::Critical => theme::Text::Custom(error_text),
    };
    bar(usage.used_fraction(), class)
}

/// The one bar of the disk meter and the activity pane: as many monospace cells as fit the
/// space left in the line, `fraction` of them filled (`█`) in `class`, the rest `░`, dimmed.
fn bar(fraction: f64, class: theme::Text) -> Element<'static, Message> {
    let bar = widget::responsive(move |size| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a cell count from a width in pixels"
        )]
        let cells = (size.width / MONO_CELL_WIDTH).floor().max(0.0) as usize;
        let (filled, empty) = fmt::bar_cells(fraction, cells);
        widget::row::with_children(vec![
            monotext("█".repeat(filled))
                .wrapping(Wrapping::None)
                .class(class)
                .into(),
            monotext("░".repeat(empty))
                .wrapping(Wrapping::None)
                .class(theme::Text::Custom(dim_text))
                .into(),
        ])
        .into()
    });
    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(LINE_HEIGHT))
        .clip(true)
        .into()
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
        CliError::NoHelper => out.push(fl!("need-helper")),
        CliError::NotAuthorized => out.push(fl!("failed-auth")),
        CliError::DeviceNotFound { device } => out.push(disk_missing(device, known_uuid)),
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

/// `apsis-helper`, which does everything that needs root. Apsis can't work without it.
async fn helper() -> Result<HelperClient, CliError> {
    HelperClient::connect().await.ok_or(CliError::NoHelper)
}

/// Lists through `apsis-helper` (no password for the active session).
async fn list_snapshots() -> Result<SnapshotList, CliError> {
    helper().await?.list().await.map_err(CliError::from)
}

/// Lists through `apsis-helper` for the reminder; `None` without a helper.
fn background_list() -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        let Some(helper) = HelperClient::connect().await else {
            return Message::BackgroundListed(None);
        };
        Message::BackgroundListed(Some(helper.list().await.map_err(CliError::from)))
    })
}

/// Creates, deletes or restores through `apsis-helper`; returns when its `Finished` signal
/// arrives, however long rsync takes.
///
/// A bulk delete runs one step per call: the snapshot at `done`. The password is asked once
/// (`auth_admin_keep`).
async fn operate(
    operation: Operation,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), CliError> {
    let helper = helper().await?;
    let done = match &operation {
        Operation::Create(comment) => helper.create_with_progress(comment, progress).await,
        Operation::Delete(name) => helper.delete(name).await,
        Operation::DeleteMany { names, done } => match names.get(*done) {
            Some(name) => helper.delete(name).await,
            None => Ok(()),
        },
        Operation::Restore(request) => return restore(request, progress).await.map(drop),
    };
    done.map_err(CliError::from)
}

/// A plan or result in the left pane (a restore's), line by line.
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
async fn restore(
    request: &Request,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<String, CliError> {
    restore_helper()
        .await?
        .restore_with_progress(request, progress)
        .await
        .map_err(CliError::from)
}

/// Runs the operation `start` builds, as a task that also yields a [`Message::Progress`] for
/// each update it's handed, then its own message. The updates end when the operation does.
fn with_progress<F, Fut>(start: F) -> Task<cosmic::Action<Message>>
where
    F: FnOnce(Box<dyn FnMut(Progress) + Send>) -> Fut,
    Fut: Future<Output = Message> + Send + 'static,
{
    let (sender, updates) = mpsc::unbounded();
    let report: Box<dyn FnMut(Progress) + Send> = Box::new(move |progress| {
        // The receiver only goes away with the task.
        let _ = sender.unbounded_send(progress);
    });
    let done = stream::once(start(report));
    cosmic::task::stream(stream::select(updates.map(Message::Progress), done))
}

/// Apsis's config (or the import from Timeshift's settings), the devices and the users,
/// through `apsis-helper`.
async fn read_config() -> Result<ConfigInfo, CliError> {
    helper().await?.read_config().await.map_err(CliError::from)
}

/// Writes `config` through `apsis-helper` if `config.toml` still reads `expected`.
async fn write_config(expected: String, config: ApsisConfig) -> Result<String, CliError> {
    helper()
        .await?
        .write_config(&expected, &config)
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

/// Why a create or delete failed, for the activity pane.
fn error_summary(error: &CliError, known_uuid: Option<&str>) -> String {
    match error {
        CliError::NoHelper => fl!("need-helper"),
        CliError::NotAuthorized => fl!("failed-auth"),
        CliError::DeviceNotFound { device } => disk_missing(device, known_uuid),
        CliError::Other(message) => message.clone(),
    }
}

/// A bulk delete that stopped: which snapshot failed and why, then what was deleted and what
/// wasn't (including the one that failed). Takes the snapshots' labels, not their names.
fn bulk_delete_failed(deleted: &[String], left: &[String], reason: &str) -> String {
    let list = |names: &[String]| {
        if names.is_empty() {
            fl!("delete-none")
        } else {
            names.join(", ")
        }
    };
    [
        fl!(
            "delete-many-stopped",
            name = left.first().cloned().unwrap_or_default(),
            reason = reason
        ),
        fl!(
            "delete-many-deleted",
            count = deleted.len().to_string(),
            names = list(deleted)
        ),
        fl!(
            "delete-many-kept",
            count = left.len().to_string(),
            names = list(left)
        ),
    ]
    .join("\n")
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

/// Full name, creation time, age, tags spelled out and the whole comment; `delete   marked` if
/// it's marked.
fn details(snapshot: &Snapshot, marked: bool) -> Element<'_, Message> {
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
    let mark = marked.then(|| (fl!("details-delete"), fl!("details-marked")));
    key_value_rows(fields.into_iter().chain(mark), DETAILS_KEY_WIDTH)
}

fn help() -> Element<'static, Message> {
    let keys = [
        ("↑ ↓  j k", fl!("help-move")),
        ("Home End", fl!("help-ends")),
        ("Enter", fl!("help-browse")),
        ("Tab", fl!("help-details")),
        ("c", fl!("help-create")),
        ("space", fl!("help-mark")),
        ("J", fl!("help-mark-down")),
        ("d", fl!("help-delete")),
        ("p", fl!("help-prune")),
        ("r", fl!("help-refresh")),
        ("s", fl!("help-settings")),
        ("1 2 3 4", fl!("help-rooms")),
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
/// Apsis 0.2.0
/// Simple system snapshots and file restore for the COSMIC™ desktop
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

/// The command that opens the window: this program (as `current_exe` names it; a package
/// upgrade leaves `... (deleted)` on the path of a running program, cut off here) or `apsis` from
/// the `PATH`, with `--window` or `flag`.
fn window_command(exe: Option<std::path::PathBuf>, flag: Option<&str>) -> std::process::Command {
    let program = exe
        .map(|path| {
            let text = path.to_string_lossy();
            std::path::PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(&text))
        })
        .unwrap_or_else(|| "apsis".into());
    let mut command = std::process::Command::new(program);
    command.arg(flag.unwrap_or("--window"));
    command
}

/// The view `apsis --settings` or `--about` opens on, if any.
fn startup_overlay(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<Overlay> {
    args.into_iter().find_map(|arg| match arg.to_str()? {
        "--settings" => Some(Overlay::Settings),
        "--about" => Some(Overlay::About),
        _ => None,
    })
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
        'p' => Some(KeyAction::Prune),
        'o' => Some(KeyAction::OpenWindow),
        '1' => Some(KeyAction::Room(Room::Snapshots)),
        '2' => Some(KeyAction::Room(Room::Create)),
        '3' => Some(KeyAction::Room(Room::Schedule)),
        '4' => Some(KeyAction::Room(Room::Log)),
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

/// A dock cell: a thin accent outline; the active room's cell has the selected row's fill.
fn dock_cell(theme: &Theme, active: bool) -> container::Style {
    let cosmic = theme.cosmic();
    let accent = Color::from(cosmic.accent_color());
    container::Style {
        background: active.then(|| Background::Color(accent.scale_alpha(0.15))),
        border: Border {
            color: accent,
            width: LINE,
            radius: cosmic.corner_radii.radius_s.into(),
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

/// The disk bar's filled cells while there's plenty of space.
fn accent_text(theme: &Theme) -> iced_text::Style {
    text_style(theme, theme.cosmic().accent_text_color().into())
}

/// The panel icon as the panel draws it: the normal text colour.
fn plain_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(Color::from(theme.cosmic().background(theme.transparent).on)),
    }
}

/// The panel icon while the reminder is due or the disk is low.
fn warning_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(theme.cosmic().warning_text_color().into()),
    }
}

/// The panel icon while the disk is nearly full.
fn critical_svg(theme: &Theme) -> iced_svg::Style {
    iced_svg::Style {
        color: Some(theme.cosmic().destructive_text_color().into()),
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
            listing: Listing::Failed(CliError::NoHelper),
            known_uuid: None,
            loading: false,
            running: None,
            progress: None,
            run_started: None,
            read_only: false,
            room: Room::Snapshots,
            schedule_row: 0,
            log: Vec::new(),
            prompt: Prompt::Command,
            status: None,
            spinner: 0,
            selected: 0,
            marked: BTreeSet::new(),
            overlay: Overlay::None,
            settings: SettingsLoad::NotLoaded,
            saving_settings: false,
            browser: None,
            pending_restore: None,
            list_after_browser: false,
            prune_after_list: false,
            browse_calls: 0,
            restore_text: None,
            window_resizable: false,
            icon: symbolic_icon(),
            helper_found: false,
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

    #[test]
    fn menu_close_closes_only_the_menu() {
        let mut app = model();
        send(&mut app, Message::ToggleMenu);
        send(&mut app, Message::MenuClose);
        assert!(app.menu.is_none());
        assert!(app.popup.is_none());
        assert!(!app.loading);
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
        app.on_listed(Ok(fixture(DEVICE_LIST)));
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

    /// Five snapshots on a configured device, the newest with a comment (as the helper would
    /// list them, oldest first).
    const DEVICE_LIST: &str = "device";
    /// No backup device chosen: no snapshots.
    const UNCONFIGURED_LIST: &str = "unconfigured";
    /// [`DEVICE_LIST`] with a warning about an incomplete folder.
    const STALE_MOUNT_LIST: &str = "warning";

    /// The list `name` stands for (see the constants above).
    fn fixture(name: &str) -> SnapshotList {
        if name == UNCONFIGURED_LIST {
            return SnapshotList::default();
        }
        let snapshot = |name: &str, comment: Option<&str>| Snapshot {
            name: name.to_owned(),
            created: apsis_core::parse_snapshot_name(name).unwrap(),
            tags: vec![apsis_core::Tag::OnDemand],
            comment: comment.map(str::to_owned),
        };
        let mut list = SnapshotList {
            device: Some("/dev/sdX1".to_owned()),
            uuid: Some("00000000-0000-0000-0000-000000000000".to_owned()),
            mode: Some(apsis_core::Mode::Rsync),
            snapshots: vec![
                snapshot("2026-09-19_09-29-57", None),
                snapshot("2026-09-19_09-58-41", None),
                snapshot("2026-09-22_13-28-36", None),
                snapshot("2026-09-23_08-33-55", None),
                snapshot(
                    "2026-09-25_11-28-53",
                    Some("apsis test: comment with spaces"),
                ),
            ],
            warnings: Vec::new(),
            usage: None,
        };
        if name == STALE_MOUNT_LIST {
            list.warnings = vec!["2026-09-02_09-00-00: incomplete: no info.json".to_owned()];
        }
        list
    }

    /// The popup open on a listed fixture. Nothing here runs anything as root: `update`'s
    /// tasks are dropped without being run.
    fn listed(fixture_name: &str) -> AppModel {
        let mut app = model();
        app.on_listed(Ok(fixture(fixture_name)));
        send(&mut app, Message::TogglePopup);
        assert!(app.popup.is_some() && !app.loading);
        app
    }

    fn typed(app: &mut AppModel, text: &str) {
        send(app, Message::Input(text.to_owned()));
    }

    /// A create or delete that ran and failed (rsync, a write): the list may have changed.
    fn failed() -> Result<(), CliError> {
        Err(CliError::Other(
            "rsync exited with code 11: boom".to_owned(),
        ))
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

    /// Marks rows `indices` (0 = newest) with space, from the top.
    fn mark(app: &mut AppModel, indices: &[usize]) {
        for &index in indices {
            app.selected = index;
            typed(app, " ");
        }
    }

    #[test]
    fn space_marks_in_place_and_capital_j_marks_and_moves_down() {
        let mut app = listed(DEVICE_LIST);
        let popup = app.popup.unwrap();
        typed(&mut app, " ");
        assert_eq!(app.selected, 0);
        assert!(app.marked.contains(&app.snapshots()[0].name));
        assert!(
            app.body_title().ends_with("· 1 marked"),
            "{}",
            app.body_title()
        );

        typed(&mut app, "j");
        typed(&mut app, "J");
        typed(&mut app, "J");
        assert_eq!(app.selected, 3);
        assert_eq!(app.marked.len(), 3);
        // Space again unmarks.
        typed(&mut app, "k");
        typed(&mut app, " ");
        assert_eq!(app.marked.len(), 2);
        assert!(app.body_title().ends_with("· 2 marked"));

        // J on the last row marks it and stays.
        send(&mut app, Message::Key(popup, KeyAction::Last));
        typed(&mut app, "J");
        assert_eq!(app.selected, 4);
        assert_eq!(app.marked.len(), 3);

        // Esc clears the marks first, then closes the popup.
        send(&mut app, Message::Key(popup, KeyAction::Escape));
        assert!(app.marked.is_empty());
        assert_eq!(app.popup, Some(popup));
        assert_eq!(app.body_title(), fl!("pane-snapshots"));
        send(&mut app, Message::Key(popup, KeyAction::Escape));
        assert!(app.popup.is_none());
    }

    #[test]
    fn marks_do_nothing_on_help_and_are_dropped_with_their_snapshots() {
        let mut app = listed(DEVICE_LIST);
        send(&mut app, Message::ToggleHelp);
        typed(&mut app, " ");
        assert!(app.marked.is_empty());
        send(&mut app, Message::ToggleHelp);

        mark(&mut app, &[0, 1]);
        let mut list = fixture(DEVICE_LIST);
        list.snapshots.retain(|s| s.name != app.snapshots()[0].name);
        send(&mut app, Message::Listed(Ok(list)));
        assert_eq!(app.marked.len(), 1);
        send(&mut app, Message::Listed(Err(CliError::NoHelper)));
        assert!(app.marked.is_empty());
    }

    #[test]
    fn d_with_marks_confirms_all_of_them_in_list_order() {
        let mut app = listed(DEVICE_LIST);
        mark(&mut app, &[3, 0, 1]);
        let names: Vec<String> = [0, 1, 3]
            .iter()
            .map(|&i| app.snapshots()[i].name.clone())
            .collect();

        typed(&mut app, "d");
        assert_eq!(
            app.prompt,
            Prompt::ConfirmDeleteMany {
                names: names.clone(),
                typed: String::new()
            }
        );
        typed(&mut app, "n");
        send(&mut app, Message::Submit);
        assert!(app.running.is_none());
        assert_eq!(app.marked.len(), 3, "a cancel keeps the marks");

        typed(&mut app, "d");
        typed(&mut app, "y");
        send(&mut app, Message::Submit);
        assert_eq!(app.running, Some(Operation::DeleteMany { names, done: 0 }));
        let (activity, _) = app.activity_line();
        assert!(
            activity.starts_with("deleting 1/3: 09-25 11:28 \"apsis test"),
            "{activity}"
        );
        // The prompt and the results show labels; the helper still gets the names.
        assert_eq!(
            app.snapshot_labels(&[
                "2026-09-25_11-28-53".to_owned(),
                "2026-09-23_08-33-55".to_owned()
            ]),
            "09-25 11:28 \"apsis test: comment with spaces\", 09-23 08:33"
        );
        assert_eq!(
            app.snapshot_label("2020-01-01_00-00-00"),
            "2020-01-01_00-00-00"
        );
    }

    #[test]
    fn bulk_delete_goes_one_by_one_then_refreshes() {
        let mut app = listed(DEVICE_LIST);
        mark(&mut app, &[0, 1, 2]);
        let names = app.marked_names();
        let step = |done| Operation::DeleteMany {
            names: names.clone(),
            done,
        };
        app.running = Some(step(0));

        send(&mut app, Message::Finished(step(0), Ok(())));
        assert_eq!(app.running, Some(step(1)));
        assert!(!app.loading, "no list between steps");
        let (activity, _) = app.activity_line();
        assert!(
            activity.starts_with("deleting 2/3: 09-23 08:33…"),
            "{activity}"
        );
        // Busy: no marking, no second delete.
        typed(&mut app, " d");
        assert_eq!(app.marked.len(), 3);
        assert_eq!(app.prompt, Prompt::Command);

        send(&mut app, Message::Finished(step(1), Ok(())));
        send(&mut app, Message::Finished(step(2), Ok(())));
        assert!(app.running.is_none());
        assert!(app.loading);
        assert!(app.marked.is_empty());
        assert_eq!(
            app.status,
            Some(Status::Info("deleted 3 snapshots".to_owned()))
        );
    }

    #[test]
    fn bulk_delete_stops_at_the_first_failure_and_says_what_was_deleted() {
        let mut app = listed(DEVICE_LIST);
        mark(&mut app, &[0, 1, 2]);
        let names = app.marked_names();
        let step = |done| Operation::DeleteMany {
            names: names.clone(),
            done,
        };
        app.running = Some(step(0));
        send(&mut app, Message::Finished(step(0), Ok(())));
        send(&mut app, Message::Finished(step(1), failed()));

        assert!(app.running.is_none());
        assert!(app.loading, "one was deleted, so the list changed");
        assert_eq!(
            status_error(&app),
            "delete stopped at 09-23 08:33: rsync exited with code 11: boom\n\
             deleted (1): 09-25 11:28 \"apsis test: comment with spaces\"\n\
             not deleted (2): 09-23 08:33, 09-22 13:28"
        );
        // The ones left stay marked, for another try.
        assert_eq!(app.marked_names(), names[1..]);
    }

    #[test]
    fn refused_bulk_delete_deletes_nothing_and_does_not_refresh() {
        let mut app = listed(DEVICE_LIST);
        mark(&mut app, &[0, 1]);
        let names = app.marked_names();
        let first = Operation::DeleteMany {
            names: names.clone(),
            done: 0,
        };
        app.running = Some(first.clone());
        send(
            &mut app,
            Message::Finished(first, Err(CliError::NotAuthorized)),
        );
        assert!(!app.loading);
        assert!(status_error(&app).contains("deleted (0): none"));
        assert_eq!(app.marked.len(), 2);
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
    fn finishing_refreshes_only_if_something_may_have_changed() {
        let create = Operation::Create(String::new());
        for (result, refresh) in [
            (Ok(()), true),
            (failed(), true),
            (Err(CliError::NoHelper), false),
            (Err(CliError::NotAuthorized), false),
            (
                Err(CliError::DeviceNotFound {
                    device: "00000000".to_owned(),
                }),
                false,
            ),
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
    fn the_helpers_reason_is_shown() {
        let mut app = listed(DEVICE_LIST);
        let failed = apsis_core::Error::InvalidInput(
            "not deleting 2026-09-19_09-29-57: something is mounted inside it".to_owned(),
        );
        let delete = Operation::Delete("2026-09-19_09-29-57".to_owned());
        send(&mut app, Message::Finished(delete, Err(failed.into())));
        assert!(
            status_error(&app).ends_with("mounted inside it"),
            "{}",
            status_error(&app)
        );
    }

    #[test]
    fn list_warnings_reach_the_activity_pane() {
        let mut app = listed(DEVICE_LIST);
        assert!(app.list_warnings().is_empty());
        app.on_listed(Ok(fixture(STALE_MOUNT_LIST)));
        assert_eq!(app.snapshots().len(), 5);
        assert_eq!(
            app.list_warnings(),
            ["2026-09-02_09-00-00: incomplete: no info.json"]
        );
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
    fn failure_shows_the_helpers_reason() {
        let mut app = listed(DEVICE_LIST);
        let delete = Operation::Delete("2026-09-19_09-29-57".to_owned());
        send(&mut app, Message::Finished(delete, failed()));
        let Some(Status::Error(text)) = &app.status else {
            panic!("{:?}", app.status)
        };
        assert!(text.ends_with("rsync exited with code 11: boom"), "{text}");
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
        send(&mut app, Message::Finished(delete, failed()));
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
        // No list beside the browser: its calls hold the helper's lock.
        assert!(!app.loading);
        // Leaving the browser while its reload runs still waits for the reload...
        let snapshot = app.browser.as_ref().unwrap().snapshot.clone();
        send(&mut app, Message::Escape);
        assert!(app.browser.is_none() && !app.loading);
        send(
            &mut app,
            Message::Browsed(snapshot, SnapPath::root(), folder(&[("etc", Kind::Dir)])),
        );
        // ...then lists, for the disk line.
        assert!(app.loading);
    }

    #[test]
    fn leaving_the_browser_after_a_restore_lists_once() {
        let mut app = browsing();
        let request = Request {
            snapshot: app.browser.as_ref().unwrap().snapshot.clone(),
            paths: vec!["/etc".to_owned()],
            destination: Destination::Folder,
            dry_run: false,
        };
        app.running = Some(Operation::Restore(request.clone()));
        send(
            &mut app,
            Message::RestoreDone(request, Ok("restored".to_owned())),
        );
        browsed(&mut app, "/", &[("etc", Kind::Dir)]);
        assert!(!app.loading, "the browser is still open");
        send(&mut app, Message::Escape);
        send(&mut app, Message::Escape);
        assert!(app.browser.is_none() && app.loading);
        app.on_listed(Ok(SnapshotList::default()));
        // Browsing again and leaving without a restore doesn't list.
        assert!(!app.list_after_browser);
    }

    #[test]
    fn create_progress_goes_from_spinner_to_working_to_a_bar() {
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "c");
        send(&mut app, Message::Submit);
        assert!(matches!(app.running, Some(Operation::Create(_))));
        assert!(app.run_started.is_some());
        // Nothing from the helper yet (or ever, through pkexec): the plain spinner line.
        assert_eq!(app.progress_line(), None);
        let at = |percent: Option<f64>, eta: Option<u64>| Progress {
            percent,
            eta_seconds: eta,
            text: String::new(),
        };
        let now = Instant::now();
        app.run_started = Some(now.checked_sub(Duration::from_secs(68)).unwrap());
        app.spinner = 2;
        send(&mut app, Message::Progress(at(None, None)));
        assert_eq!(
            app.progress_line_at(now),
            Some(ProgressLine::Working(
                "creating snapshot · working · 1m 08s elapsed ⠹".to_owned()
            ))
        );
        // Timeshift's `0.00% complete (??? remaining)` is no number yet either.
        send(&mut app, Message::Progress(at(Some(0.0), None)));
        assert!(matches!(
            app.progress_line_at(now),
            Some(ProgressLine::Working(_))
        ));
        send(&mut app, Message::Progress(at(Some(58.23), Some(192))));
        let bar = |app: &AppModel| match app.progress_line_at(now) {
            Some(ProgressLine::Bar { text, fraction }) => (text, fraction),
            other => panic!("{other:?}"),
        };
        let (text, fraction) = bar(&app);
        assert_eq!(text, "creating snapshot · 58% · 3m 12s left");
        assert!((fraction - 0.5823).abs() < 1e-9);
        // No time left known: the percent alone.
        send(&mut app, Message::Progress(at(Some(58.23), None)));
        assert_eq!(bar(&app).0, "creating snapshot · 58%");
        // Done: back to the status line; a late update is dropped.
        send(
            &mut app,
            Message::Finished(Operation::Create(String::new()), Ok(())),
        );
        assert_eq!(app.progress_line(), None);
        assert!(app.run_started.is_none());
        send(&mut app, Message::Progress(at(Some(99.0), Some(1))));
        assert!(app.progress.is_none());
    }

    #[test]
    fn a_restore_uses_the_same_line() {
        let mut app = listed(DEVICE_LIST);
        app.running = Some(Operation::Restore(Request {
            snapshot: "2026-09-25_10-00-00".to_owned(),
            paths: vec!["/etc/hosts".to_owned()],
            destination: Destination::Original,
            dry_run: false,
        }));
        app.progress = Some(Progress {
            percent: Some(12.9),
            eta_seconds: Some(45),
            text: String::new(),
        });
        let Some(ProgressLine::Bar { text, .. }) = app.progress_line() else {
            panic!()
        };
        assert_eq!(text, "restoring · 12% · 45s left");
    }

    #[test]
    fn deletes_have_no_progress_line() {
        let mut app = listed(DEVICE_LIST);
        app.running = Some(Operation::Delete(app.snapshots()[0].name.clone()));
        app.progress = Some(Progress {
            percent: Some(50.0),
            eta_seconds: None,
            text: String::new(),
        });
        assert_eq!(app.progress_line(), None);
    }

    /// Six on-demand snapshots a day apart, the oldest commented, the newest last.
    fn manual_list() -> SnapshotList {
        let mut list = fixture(DEVICE_LIST);
        list.snapshots = (20..26)
            .map(|day| {
                let name = format!("2026-09-{day}_10-00-00");
                Snapshot {
                    created: apsis_core::parse_snapshot_name(&name).unwrap(),
                    name,
                    tags: vec![apsis_core::Tag::OnDemand],
                    comment: None,
                }
            })
            .collect();
        list.snapshots[0].comment = Some("before upgrade".to_owned());
        list
    }

    fn with_list(list: SnapshotList, keep_manual: u32) -> AppModel {
        let mut app = model();
        app.config.keep_manual = keep_manual;
        app.on_listed(Ok(list));
        send(&mut app, Message::TogglePopup);
        app
    }

    #[test]
    fn p_previews_the_prune_and_y_deletes_oldest_first() {
        let mut app = with_list(manual_list(), 2);
        typed(&mut app, "p");
        assert_eq!(app.overlay, Overlay::Prune);
        assert_eq!(app.body_title(), fl!("pane-prune"));
        let Prompt::ConfirmPrune { names, .. } = app.prompt.clone() else {
            panic!("{:?}", app.prompt)
        };
        // 20 is commented (pinned, not counted); 24 and 25 are the newest two uncommented.
        assert_eq!(
            names,
            [
                "2026-09-21_10-00-00",
                "2026-09-22_10-00-00",
                "2026-09-23_10-00-00"
            ]
        );
        let lines = app.prune_lines();
        assert_eq!(lines.len(), 6);
        assert!(lines[0].starts_with("delete"), "{lines:?}");
        assert!(lines.iter().any(|l| l.ends_with("comment")), "{lines:?}");
        typed(&mut app, "y");
        send(&mut app, Message::Submit);
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.running, Some(Operation::DeleteMany { names, done: 0 }));
    }

    #[test]
    fn anything_but_y_or_esc_deletes_nothing_and_closes_the_preview() {
        for answer in ["", "n", "yes please"] {
            let mut app = with_list(manual_list(), 2);
            typed(&mut app, "p");
            typed(&mut app, answer);
            send(&mut app, Message::Submit);
            assert!(app.running.is_none(), "{answer:?}");
            assert_eq!(app.overlay, Overlay::None);
        }
        let mut app = with_list(manual_list(), 2);
        typed(&mut app, "p");
        send(&mut app, Message::Escape);
        assert_eq!(
            (app.overlay, app.prompt.clone()),
            (Overlay::None, Prompt::Command)
        );
    }

    #[test]
    fn p_says_why_there_is_nothing_to_prune() {
        let mut app = with_list(manual_list(), 0);
        typed(&mut app, "p");
        assert_eq!(app.overlay, Overlay::None);
        assert!(matches!(&app.status, Some(Status::Info(t)) if *t == fl!("prune-off")));
        let mut app = with_list(manual_list(), 6);
        typed(&mut app, "p");
        assert_eq!(app.prompt, Prompt::Command);
        assert!(matches!(&app.status, Some(Status::Info(t)) if t.starts_with("nothing")));
    }

    #[test]
    fn a_create_offers_the_prune_only_when_there_is_something() {
        for (keep, offered) in [(0, false), (6, false), (3, true)] {
            let mut app = with_list(manual_list(), keep);
            app.running = Some(Operation::Create(String::new()));
            send(
                &mut app,
                Message::Finished(Operation::Create(String::new()), Ok(())),
            );
            assert!(app.loading, "a create lists again");
            send(&mut app, Message::Listed(Ok(manual_list())));
            assert_eq!(app.overlay == Overlay::Prune, offered, "keep {keep}");
            // Quiet when there's nothing: the "created" status stays.
            if !offered {
                assert!(matches!(&app.status, Some(Status::Info(t)) if *t == fl!("created")));
            }
        }
        // A failed create offers nothing.
        let mut app = with_list(manual_list(), 3);
        app.running = Some(Operation::Create(String::new()));
        send(
            &mut app,
            Message::Finished(Operation::Create(String::new()), failed()),
        );
        send(&mut app, Message::Listed(Ok(manual_list())));
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn the_reminder_needs_a_device_and_an_old_newest_snapshot() {
        use apsis_core::Due;
        let now = jiff::civil::date(2026, 10, 3).at(10, 0, 0, 0);
        let due = |app: &AppModel| match app.status_view_at(now) {
            StatusView::Loaded(status) => status.due(),
            other => panic!("{other:?}"),
        };
        let mut app = with_list(manual_list(), 0);
        // Newest 09-25 10:00: 8 days.
        app.config.remind_days = 7;
        assert_eq!(due(&app), Due::Overdue { days: 7 });
        assert_eq!(app.status_view_at(now).severity(), Severity::Warning);
        app.config.remind_days = 8;
        assert_eq!(due(&app), Due::Ok, "exactly 8 days isn't over 8");
        app.config.remind_days = 0;
        assert_eq!(due(&app), Due::Off, "off");
        // No snapshots yet, but a device: remind.
        app.config.remind_days = 7;
        let mut empty = manual_list();
        empty.snapshots.clear();
        app.on_listed(Ok(empty.clone()));
        assert_eq!(due(&app), Due::Never { days: 7 });
        // No device selected, or nothing listed: nothing to remind about.
        empty.device = None;
        app.on_listed(Ok(empty));
        assert_eq!(due(&app), Due::Off);
        app.on_listed(Err(CliError::NotAuthorized));
        assert_eq!(app.status_view_at(now), StatusView::Failed);
        assert_eq!(app.status_view_at(now).severity(), Severity::None);
    }

    #[test]
    fn background_lists_never_start_pkexec() {
        let mut app = model();
        app.listing = Listing::NotLoaded;
        app.loading = true;
        // No helper: nothing listed, and no second list while the popup is closed.
        send(&mut app, Message::BackgroundListed(None));
        assert!(!app.loading && !app.helper_found);
        assert!(matches!(app.listing, Listing::NotLoaded));
        // The popup opened while it ran: then the usual list (which may ask for a password,
        // because the user asked for it).
        app.loading = true;
        send(&mut app, Message::TogglePopup);
        send(&mut app, Message::BackgroundListed(None));
        assert!(app.loading);
        send(&mut app, Message::TogglePopup);
        app.loading = false;
        // With the helper, the list lands like any other.
        send(&mut app, Message::BackgroundListed(Some(Ok(manual_list()))));
        assert!(app.helper_found);
        assert_eq!(app.snapshots().len(), 6);
        // A refresh while the popup is open waits for the popup to close.
        send(&mut app, Message::TogglePopup);
        send(&mut app, Message::BackgroundRefresh);
        assert!(!app.loading);
    }

    #[test]
    fn free_space_levels_follow_the_share_left() {
        let usage = |used: u64, free: u64| DiskUsage {
            total: used + free,
            used,
            free,
        };
        assert_eq!(Space::of(&usage(50, 50)), Space::Plenty);
        assert_eq!(Space::of(&usage(900, 100)), Space::Plenty);
        assert_eq!(Space::of(&usage(901, 99)), Space::Low);
        assert_eq!(Space::of(&usage(950, 50)), Space::Low);
        assert_eq!(Space::of(&usage(951, 49)), Space::Critical);
        assert_eq!(Space::of(&usage(1, 0)), Space::Critical);
        // Blocks kept back for root count as neither used nor free, as in df.
        let reserved = DiskUsage {
            total: 1000,
            used: 850,
            free: 100,
        };
        assert_eq!(Space::of(&reserved), Space::Plenty);
    }

    #[test]
    fn disk_line_needs_a_list_and_some_numbers() {
        let mut app = model();
        assert!(app.disk_line().is_none());
        app.on_listed(Ok(SnapshotList::default()));
        assert!(app.disk_line().is_none(), "unknown usage: no line");
        // A list without `statvfs` numbers (the helper couldn't get them): no line either.
        let mut app = listed(DEVICE_LIST);
        assert!(app.disk_line().is_none());
        assert!(
            !app.status_view().tooltip().contains("free"),
            "{}",
            app.status_view().tooltip()
        );
        let Listing::Loaded(list) = &mut app.listing else {
            panic!()
        };
        list.usage = DiskUsage::from_statvfs(1000, 400, 350, 1024 * 1024);
        assert!(app.disk_line().is_some());
        assert!(
            app.status_view().tooltip().ends_with("350M free"),
            "{}",
            app.status_view().tooltip()
        );
    }

    /// With the strip in, the snapshot panes still have room for a few rows at the smallest
    /// window, and nothing overlaps. `APSIS_LAYOUT_TEST=1`, as the other layout tests.
    #[test]
    fn the_strip_fits_the_smallest_window() {
        use cosmic::iced::core::layout::Limits as LayoutLimits;
        use cosmic::iced::core::renderer::Headless;
        use cosmic::iced::core::widget::Tree;

        if std::env::var_os("APSIS_LAYOUT_TEST").is_none_or(|v| v != "1") {
            eprintln!("layout test skipped; set APSIS_LAYOUT_TEST=1 to run it");
            return;
        }
        let Some(renderer) =
            cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
                cosmic::font::default(),
                14.0.into(),
                Some("tiny-skia"),
            ))
        else {
            eprintln!("no headless renderer here; skipped");
            return;
        };
        let mut app = listed(DEVICE_LIST);
        app.mode = Mode::Window;
        let Listing::Loaded(list) = &mut app.listing else {
            panic!()
        };
        list.usage = Some(DiskUsage {
            total: 1_000_203_837_440,
            used: 950_000_000_000,
            free: 50_203_837_440,
        });
        let mut surface = app.surface();
        let mut tree = Tree::new(&surface);
        let limits = LayoutLimits::new(Size::ZERO, WINDOW_MIN_SIZE);
        let node = surface
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        // Header, strip, panes, activity, prompt, hints.
        let parts = node.children();
        let (strip, panes) = (&parts[1], &parts[2]);
        eprintln!(
            "strip {:.0} px, panes {:.0} px of {:.0}",
            strip.bounds().height,
            panes.bounds().height,
            node.bounds().height
        );
        assert!(
            strip.bounds().y + strip.bounds().height <= panes.bounds().y + 0.5,
            "the strip overlaps the panes"
        );
        assert!(
            panes.bounds().height >= 5.0 * LINE_HEIGHT,
            "panes {} px high",
            panes.bounds().height
        );
        assert!(node.bounds().height <= WINDOW_MIN_SIZE.height + 0.5);
    }

    #[test]
    fn the_panels_overview_only_looks() {
        let mut app = listed(DEVICE_LIST);
        app.read_only = true;
        // Nothing that changes anything: create, delete, settings, help, prune, restore...
        for keys in ["c", "d", "s", "?", "p", "R", "J", " ", "j", "k"] {
            typed(&mut app, keys);
            assert_eq!(app.prompt, Prompt::Command, "{keys}");
            assert_eq!(app.overlay, Overlay::None, "{keys}");
            assert!(app.running.is_none() && app.popup.is_some(), "{keys}");
        }
        // Refresh lists again.
        typed(&mut app, "r");
        assert!(app.loading);
        app.loading = false;
        // `o` opens the window and closes the popup.
        typed(&mut app, "o");
        assert!(app.popup.is_none());
        // Esc closes it too.
        send(&mut app, Message::TogglePopup);
        assert!(app.popup.is_some());
        send(&mut app, Message::Escape);
        assert!(app.popup.is_none());
    }

    #[test]
    fn the_overview_menu_items_open_the_window_instead() {
        let mut app = listed(DEVICE_LIST);
        app.read_only = true;
        send(&mut app, Message::MenuSettings);
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.popup.is_none());
        send(&mut app, Message::TogglePopup);
        send(&mut app, Message::MenuAbout);
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.popup.is_none());
        // Not read-only (the window's own state machine): unchanged, the view opens in place.
        app.read_only = false;
        send(&mut app, Message::MenuAbout);
        assert_eq!(app.overlay, Overlay::About);
    }

    #[test]
    fn the_window_command_is_this_program_with_a_flag() {
        let command = |exe: Option<&str>, flag| {
            let command = window_command(exe.map(Into::into), flag);
            let program = command.get_program().to_string_lossy().into_owned();
            let args: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            (program, args)
        };
        assert_eq!(
            command(Some("/usr/bin/apsis"), None),
            ("/usr/bin/apsis".to_owned(), vec!["--window".to_owned()])
        );
        // A package upgrade while the panel runs.
        assert_eq!(
            command(Some("/usr/bin/apsis (deleted)"), Some("--settings")),
            ("/usr/bin/apsis".to_owned(), vec!["--settings".to_owned()])
        );
        assert_eq!(
            command(None, Some("--about")),
            ("apsis".to_owned(), vec!["--about".to_owned()])
        );
    }

    #[test]
    fn the_window_opens_on_the_view_its_flag_names() {
        let args = |list: &[&str]| {
            list.iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        };
        assert_eq!(startup_overlay(args(&["--window"])), None);
        assert_eq!(
            startup_overlay(args(&["--settings"])),
            Some(Overlay::Settings)
        );
        assert_eq!(
            startup_overlay(args(&["--window", "--about"])),
            Some(Overlay::About)
        );
        assert_eq!(startup_overlay(args(&[])), None);
    }

    #[test]
    fn the_overview_builds_for_every_state_of_the_list() {
        let mut app = model();
        app.read_only = true;
        // Failed (no helper), not loaded, loading.
        let _ = app.overview();
        app.listing = Listing::NotLoaded;
        let _ = app.overview();
        app.loading = true;
        let _ = app.overview();
        app.loading = false;
        // No device, empty, a few, many.
        app.on_listed(Ok(SnapshotList::default()));
        let _ = app.overview();
        let mut list = manual_list();
        list.snapshots.clear();
        app.on_listed(Ok(list));
        let _ = app.overview();
        app.on_listed(Ok(manual_list()));
        let _ = app.overview();
        assert_eq!(
            fmt::older_count(app.snapshots().len(), OVERVIEW_ROWS),
            Some(1)
        );
    }

    /// The overview fits its width, with the longest row it can have (all tags, a long
    /// comment), and stays short. `APSIS_LAYOUT_TEST=1`, as the other layout tests.
    #[test]
    fn the_overview_fits_its_card() {
        use cosmic::iced::core::layout::Limits as LayoutLimits;
        use cosmic::iced::core::renderer::Headless;
        use cosmic::iced::core::widget::Tree;

        if std::env::var_os("APSIS_LAYOUT_TEST").is_none_or(|v| v != "1") {
            eprintln!("layout test skipped; set APSIS_LAYOUT_TEST=1 to run it");
            return;
        }
        let Some(renderer) =
            cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
                cosmic::font::default(),
                14.0.into(),
                Some("tiny-skia"),
            ))
        else {
            eprintln!("no headless renderer here; skipped");
            return;
        };
        let mut app = listed(DEVICE_LIST);
        app.read_only = true;
        let Listing::Loaded(list) = &mut app.listing else {
            panic!()
        };
        list.usage = Some(DiskUsage {
            total: 1_000_203_837_440,
            used: 950_000_000_000,
            free: 50_203_837_440,
        });
        list.snapshots = manual_list().snapshots;
        for snapshot in &mut list.snapshots {
            snapshot.tags = vec![
                apsis_core::Tag::OnDemand,
                apsis_core::Tag::Boot,
                apsis_core::Tag::Daily,
            ];
            snapshot.comment = Some("a comment that is much too long for the row".to_owned());
        }
        let mut overview = app.overview();
        let mut tree = Tree::new(&overview);
        let limits = LayoutLimits::new(Size::ZERO, Size::new(OVERVIEW_WIDTH, 1000.0));
        let node = overview
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        eprintln!(
            "overview {:.0} x {:.0} px",
            node.bounds().width,
            node.bounds().height
        );
        assert!(node.bounds().width <= OVERVIEW_WIDTH + 0.5);
        assert!(node.bounds().height < 520.0, "{}", node.bounds().height);
    }

    /// A window on a list with a device, as the rooms run in it.
    fn window() -> AppModel {
        let mut app = listed(DEVICE_LIST);
        app.mode = Mode::Window;
        app
    }

    #[test]
    fn number_keys_jump_between_rooms_and_esc_walks_back() {
        let mut app = window();
        assert_eq!(app.room, Room::Snapshots);
        typed(&mut app, "3");
        assert_eq!(app.room, Room::Schedule);
        typed(&mut app, "4");
        assert_eq!(app.room, Room::Log);
        typed(&mut app, "1");
        assert_eq!(app.room, Room::Snapshots);
        // Create starts the comment prompt, like `c`, and the list stays.
        typed(&mut app, "2");
        assert_eq!(app.room, Room::Create);
        assert_eq!(app.prompt, Prompt::Comment(String::new()));
        // Digits typed at the comment prompt are text, not room keys.
        typed(&mut app, "7 3");
        assert_eq!(app.room, Room::Create);
        assert_eq!(app.prompt, Prompt::Comment("7 3".to_owned()));
        // Esc: the prompt, then the room, then (window) the window.
        send(&mut app, Message::Escape);
        assert_eq!((app.room, &app.prompt), (Room::Create, &Prompt::Command));
        send(&mut app, Message::Escape);
        assert_eq!(app.room, Room::Snapshots);
        assert!(app.popup.is_some());
        send(&mut app, Message::Escape);
        assert!(app.popup.is_none());
    }

    #[test]
    fn rooms_are_the_windows_and_wait_for_a_running_job() {
        // The panel's overview, and a popup that is not the window: no rooms.
        let mut app = listed(DEVICE_LIST);
        typed(&mut app, "3");
        assert_eq!(app.room, Room::Snapshots);
        app.read_only = true;
        typed(&mut app, "3");
        assert_eq!(app.room, Room::Snapshots);
        // A running create keeps the room it started in.
        let mut app = window();
        app.running = Some(Operation::Create(String::new()));
        send(&mut app, Message::Room(Room::Log));
        assert_eq!(app.room, Room::Snapshots);
        assert!(app.dock().is_some());
        // The settings view and the browser have their own footers.
        let mut app = window();
        app.overlay = Overlay::Settings;
        assert!(app.dock().is_none());
        app.overlay = Overlay::Browse;
        assert!(app.dock().is_none());
        app.overlay = Overlay::Details;
        assert!(app.dock().is_some());
        // Not in the popup.
        assert!(listed(DEVICE_LIST).dock().is_none());
    }

    #[test]
    fn the_schedule_room_takes_up_and_down_for_itself() {
        let mut app = window();
        typed(&mut app, "3");
        assert_eq!(app.schedule_row, 0);
        typed(&mut app, "j");
        assert_eq!(app.schedule_row, 1);
        assert_eq!(app.selected, 0, "the snapshot selection stays put");
        typed(&mut app, "k");
        assert_eq!(app.schedule_row, 0);
        // Other keys act as everywhere: `1` goes home, `?` shows help.
        typed(&mut app, "?");
        assert_eq!(app.overlay, Overlay::Help);
    }

    #[test]
    fn the_log_keeps_what_the_status_line_said() {
        let mut app = window();
        assert!(app.log.is_empty());
        send(
            &mut app,
            Message::Finished(Operation::Create(String::new()), Ok(())),
        );
        assert_eq!(
            app.log.last().map(|e| (e.text.as_str(), e.error)),
            Some(("snapshot created", false))
        );
        send(
            &mut app,
            Message::Finished(Operation::Create(String::new()), failed()),
        );
        let last = app.log.last().unwrap();
        assert!(
            last.error && last.text.starts_with("create failed"),
            "{last:?}"
        );
        assert_eq!(app.log.len(), 2);
        // A message that changes nothing adds nothing.
        send(&mut app, Message::Tick);
        assert_eq!(app.log.len(), 2);
        // A list that fails is logged once, not on every message after it.
        send(&mut app, Message::Listed(Err(CliError::NoHelper)));
        assert_eq!(app.log.len(), 3);
        assert!(app.log[2].error);
        send(&mut app, Message::Tick);
        assert_eq!(app.log.len(), 3);
        // Times are `HH:MM:SS`, and only the newest lines are kept.
        assert_eq!(app.log[0].time.len(), 8);
        for _ in 0..LOG_LINES + 20 {
            app.status = None;
            send(
                &mut app,
                Message::Finished(Operation::Create(String::new()), Ok(())),
            );
        }
        assert_eq!(app.log.len(), LOG_LINES);
    }

    #[test]
    fn every_room_builds() {
        let mut app = window();
        for room in Room::ALL {
            app.room = room;
            let _ = app.surface();
            assert!(app.dock().is_some());
        }
        app.log.push(LogEntry {
            time: "12:00:00".to_owned(),
            text: "create failed".to_owned(),
            error: true,
        });
        app.room = Room::Log;
        let _ = app.surface();
        app.prompt = Prompt::Comment("before upgrade".to_owned());
        app.room = Room::Create;
        let _ = app.surface();
    }

    /// Each room at the smallest window: the dock is there, the footer fits (with `[1-4]rooms`),
    /// and the panes keep room for a few rows. `APSIS_LAYOUT_TEST=1`.
    #[test]
    fn the_dock_and_every_room_fit_the_smallest_window() {
        use cosmic::iced::core::layout::Limits as LayoutLimits;
        use cosmic::iced::core::renderer::Headless;
        use cosmic::iced::core::widget::Tree;

        if std::env::var_os("APSIS_LAYOUT_TEST").is_none_or(|v| v != "1") {
            eprintln!("layout test skipped; set APSIS_LAYOUT_TEST=1 to run it");
            return;
        }
        let Some(renderer) =
            cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
                cosmic::font::default(),
                14.0.into(),
                Some("tiny-skia"),
            ))
        else {
            eprintln!("no headless renderer here; skipped");
            return;
        };
        for room in Room::ALL {
            let mut app = window();
            app.room = room;
            app.log.push(LogEntry {
                time: "12:00:00".to_owned(),
                text: "create failed: rsync exited with code 23".to_owned(),
                error: true,
            });
            let mut surface = app.surface();
            let mut tree = Tree::new(&surface);
            let limits = LayoutLimits::new(Size::ZERO, WINDOW_MIN_SIZE);
            let node = surface
                .as_widget_mut()
                .layout(&mut tree, &renderer, &limits);
            // Header, strip, panes, activity, prompt, dock, hints.
            let parts = node.children();
            assert_eq!(parts.len(), 7, "{room:?}");
            let (panes, dock, hints) = (&parts[2], &parts[5], &parts[6]);
            eprintln!(
                "{room:?}: panes {:.0} px, dock {:.0} px, footer ends {:.0} of {:.0}",
                panes.bounds().height,
                dock.bounds().height,
                hints
                    .children()
                    .last()
                    .map_or(0.0, |c| c.bounds().x + c.bounds().width),
                hints.bounds().x + hints.bounds().width
            );
            assert!(
                dock.bounds().height >= 20.0,
                "{room:?}: the dock is squeezed to {} px",
                dock.bounds().height
            );
            assert!(
                panes.bounds().height >= 5.0 * LINE_HEIGHT,
                "{room:?}: panes {} px high",
                panes.bounds().height
            );
            assert!(
                node.bounds().height <= WINDOW_MIN_SIZE.height + 0.5,
                "{room:?}"
            );
            // Nothing in the footer runs past its row.
            let right = hints.bounds().x + hints.bounds().width;
            for cell in hints.children() {
                assert!(
                    cell.bounds().x + cell.bounds().width <= right + 0.5,
                    "{room:?}: the footer is cut"
                );
            }
            // Four cells side by side in the dock.
            assert_eq!(dock.children().len(), 4);
        }
    }

    #[test]
    fn the_strip_is_the_windows_and_not_the_settings_views() {
        let mut app = listed(DEVICE_LIST);
        assert!(app.strip().is_none(), "the popup has the disk line instead");
        app.mode = Mode::Window;
        assert!(app.strip().is_some());
        app.overlay = Overlay::Settings;
        assert!(app.strip().is_none(), "settings notes are sized without it");
        // Even with no list or no disk figures there is a strip to say so.
        let mut app = model();
        app.mode = Mode::Window;
        assert!(app.strip().is_some());
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
        let mut list = fixture(DEVICE_LIST);
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

    const TIMESHIFT: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");

    /// A saved config (the real Timeshift file's device and filters), one user: root, whose
    /// home is "everything" there.
    fn settings_info() -> ConfigInfo {
        let config = apsis_core::config::import_timeshift(TIMESHIFT, &[], &[])
            .unwrap()
            .config;
        let users = vec![("root".to_owned(), "/root".to_owned(), false)];
        apsis_core::helper::config_info_from_wire((
            config.to_text(),
            apsis_core::helper::config_to_wire(&config),
            LSBLK.to_owned(),
            users,
            Vec::new(),
        ))
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

    /// Whether anything in `node` sticks out of the node holding it: text that doesn't wrap, or
    /// a scrollable whose content is taller than it (the reader has to scroll).
    fn overflows(node: &cosmic::iced::core::layout::Node) -> Option<String> {
        let size = node.size();
        node.children().iter().find_map(|child| {
            let b = child.bounds();
            if b.x + b.width > size.width + 0.5 || b.y + b.height > size.height + 0.5 {
                Some(format!("{b:?} in {size:?}"))
            } else {
                overflows(child)
            }
        })
    }

    /// Measures real text, so the result depends on the fonts installed. Runs only with
    /// `APSIS_LAYOUT_TEST=1` (`APSIS_LAYOUT_TEST=1 cargo test -p apsis settings_details_fit`),
    /// so CI can't fail over another machine's font widths.
    #[test]
    fn settings_details_fit_without_scrolling_in_the_popup_and_a_small_window() {
        use cosmic::iced::core::layout::Limits as LayoutLimits;
        use cosmic::iced::core::renderer::Headless;
        use cosmic::iced::core::widget::Tree;

        if std::env::var_os("APSIS_LAYOUT_TEST").is_none_or(|v| v != "1") {
            eprintln!("layout test skipped; set APSIS_LAYOUT_TEST=1 to run it");
            return;
        }
        // Headless tiny-skia: real text layout, no window.
        let Some(renderer) =
            cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
                cosmic::font::default(),
                14.0.into(),
                Some("tiny-skia"),
            ))
        else {
            eprintln!("no headless renderer here; skipped");
            return;
        };
        // The popup grows up to its limit; the window is its smallest size.
        for (mode, size) in [
            (Mode::Applet, Size::new(POPUP_WIDTH, 1000.0)),
            (Mode::Window, WINDOW_MIN_SIZE),
        ] {
            let mut app = in_settings();
            app.mode = mode;
            let rows = view(&app).rows();
            let mut heights = Vec::new();
            for (index, row) in rows.into_iter().enumerate() {
                if let SettingsLoad::Ready(view) = &mut app.settings {
                    view.select(index);
                }
                let mut surface = app.surface();
                let mut tree = Tree::new(&surface);
                let limits = LayoutLimits::new(Size::ZERO, size);
                let node = surface
                    .as_widget_mut()
                    .layout(&mut tree, &renderer, &limits);
                // Header, then the panes; the details pane is the second.
                let details = &node.children()[1].children()[1];
                assert_eq!(
                    overflows(details),
                    None,
                    "{mode:?} {row:?}: the details don't fit"
                );
                let panes: Vec<f32> = node.children()[1]
                    .children()
                    .iter()
                    .map(|pane| pane.size().height)
                    .collect();
                assert!(
                    (panes[0] - panes[1]).abs() < 0.5,
                    "{mode:?} {row:?}: panes {panes:?}"
                );
                heights.push(panes[1]);
            }
            // One height for every row: the popup doesn't jump while moving.
            assert!(
                heights.iter().all(|h| (h - heights[0]).abs() < 0.5),
                "{mode:?}: pane heights {heights:?}"
            );
        }
    }

    /// Like the settings layout test: real fonts, only with `APSIS_LAYOUT_TEST=1`. The disk
    /// bar's cells fit the space left for them (so [`MONO_CELL_WIDTH`] isn't too small for the
    /// mono font here), and the line fits the popup and the smallest window.
    #[test]
    fn disk_line_fits_its_row_in_the_popup_and_a_small_window() {
        use cosmic::iced::core::layout::Limits as LayoutLimits;
        use cosmic::iced::core::renderer::Headless;
        use cosmic::iced::core::widget::Tree;

        if std::env::var_os("APSIS_LAYOUT_TEST").is_none_or(|v| v != "1") {
            eprintln!("layout test skipped; set APSIS_LAYOUT_TEST=1 to run it");
            return;
        }
        let Some(renderer) =
            cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
                cosmic::font::default(),
                14.0.into(),
                Some("tiny-skia"),
            ))
        else {
            eprintln!("no headless renderer here; skipped");
            return;
        };
        // The window has the strip instead (`the_strip_fits_the_smallest_window`).
        let (mode, size) = (Mode::Applet, Size::new(POPUP_WIDTH, 1000.0));
        {
            let mut app = listed(DEVICE_LIST);
            app.mode = mode;
            let Listing::Loaded(list) = &mut app.listing else {
                panic!()
            };
            // 1 TB disk, 448G used: the longest text the line usually has.
            list.usage = Some(DiskUsage {
                total: 1_000_203_837_440,
                used: 481_036_337_152,
                free: 518_617_202_688,
            });
            let mut surface = app.surface();
            let mut tree = Tree::new(&surface);
            let limits = LayoutLimits::new(Size::ZERO, size);
            let node = surface
                .as_widget_mut()
                .layout(&mut tree, &renderer, &limits);
            // Header, panes, then the disk line: label, device, bar, text.
            let line = &node.children()[2];
            let parts = line.children();
            assert_eq!(parts.len(), 4, "{mode:?}");
            let text = &parts[3];
            let right = text.bounds().x + text.bounds().width;
            assert!(
                right <= line.bounds().width + 0.5,
                "{mode:?}: text ends at {right}"
            );
            // The bar's container, the `responsive` in it, and the row of cells in that.
            let bar = &parts[2];
            let responsive = &bar.children()[0];
            let cells = &responsive.children()[0];
            let used = cells.bounds().width;
            eprintln!(
                "{mode:?}: bar {:.1} px, cells {used:.1} px",
                bar.bounds().width
            );
            assert!(
                bar.bounds().width > 10.0 * MONO_CELL_WIDTH,
                "{mode:?}: bar too short"
            );
            assert!(
                used <= bar.bounds().width + 0.5,
                "{mode:?}: {used} px of cells in {} px",
                bar.bounds().width
            );
            // And not much shorter either: at most two cells to spare.
            assert!(
                used >= bar.bounds().width - 2.0 * MONO_CELL_WIDTH - 0.5,
                "{mode:?}: only {used} px of cells in {} px",
                bar.bounds().width
            );
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
        go_to(&mut app, Row::Home(0));
        typed(&mut app, " ");
        assert_eq!(view(&app).home_state(0), HomeState::Excluded);
        assert_eq!(app.body_title(), fl!("pane-settings-unsaved"));
        send(&mut app, Message::Submit);
        assert_eq!(view(&app).home_state(0), HomeState::Hidden);
        send(&mut app, Message::SettingsActivate(1));
        assert_eq!(view(&app).home_state(0), HomeState::All);
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
        assert_eq!(view(&app).edited.filters.last().unwrap(), "*.mp3");
        assert_eq!(view(&app).current(), Row::Filter(4));
        typed(&mut app, "x");
        assert_eq!(view(&app).edited.filters.len(), 4);
    }

    #[test]
    fn counts_take_digits_at_the_prompt() {
        let mut app = in_settings();
        go_to(&mut app, Row::KeepManual);
        typed(&mut app, "e");
        assert_eq!(
            app.prompt,
            Prompt::Count {
                counted: Counted::KeepManual,
                typed: "0".to_owned()
            }
        );
        typed(&mut app, "12x");
        send(&mut app, Message::Submit);
        assert_eq!(view(&app).backend.keep_manual, 12);
        typed(&mut app, "+");
        typed(&mut app, "+");
        typed(&mut app, "-");
        assert_eq!(view(&app).backend.keep_manual, 13);
    }

    #[test]
    fn esc_with_unsaved_changes_asks_first() {
        let mut app = in_settings();
        go_to(&mut app, Row::Home(0));
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
        go_to(&mut app, Row::Home(0));
        typed(&mut app, " ");
        typed(&mut app, "w");
        assert!(app.saving_settings);
        // Keys wait while it writes.
        typed(&mut app, " ");
        assert_eq!(view(&app).home_state(0), HomeState::Excluded);

        send(&mut app, Message::SettingsWritten(Ok(String::new())));
        assert!(!app.saving_settings);
        assert!(matches!(app.status, Some(Status::Info(_))));
        assert!(matches!(app.settings, SettingsLoad::Loading { .. }));
        assert!(app.loading);
    }

    #[test]
    fn a_refused_write_keeps_the_edits() {
        let mut app = in_settings();
        go_to(&mut app, Row::Home(0));
        typed(&mut app, " ");
        typed(&mut app, "w");
        send(
            &mut app,
            Message::SettingsWritten(Err(CliError::from(apsis_core::Error::ConfigChanged))),
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
