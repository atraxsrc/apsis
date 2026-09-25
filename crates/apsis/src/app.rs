// SPDX-License-Identifier: GPL-3.0-only

use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use apsis_core::helper::HelperClient;
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
use cosmic::iced::{Alignment, Background, Border, Color, Length, Limits, Padding, Subscription};
use cosmic::iced::{event, mouse, time, window::Id};
use cosmic::prelude::*;
use cosmic::widget::text::{body, monotext};
use cosmic::widget::{self, container, icon};
use cosmic::{Theme, theme};

use crate::config::Config;
use crate::fl;
use crate::fmt;

/// Popup width in logical pixels (UI.md: ~720, room for the list and details side by side).
const POPUP_WIDTH: f32 = 720.0;
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
/// Longest comment shown in a row; the row also ellipsizes to the pane width, and the details
/// pane shows all of it.
const ROW_COMMENT_CHARS: usize = 28;
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
/// The `>` input line. Keeping it focused gives the popup a focused widget for key input.
static INPUT_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("prompt-input"));
/// `APSIS_DEBUG_KEYS=1` logs key and popup focus events to stderr, to see where keys get lost.
static DEBUG_KEYS: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("APSIS_DEBUG_KEYS").is_some_and(|v| v == "1"));

type Cli = TimeshiftCli<PkexecRunner>;

/// The applet: a panel button and a terminal-style popup listing Timeshift snapshots.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// The popup id.
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
    /// The symbolic Apsis icon, from the icon theme or embedded.
    icon: icon::Handle,
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
}

/// A create or delete, run as root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// With the comment as typed; the backend trims it.
    Create(String),
    Delete(String),
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
    Tick,
    Select(usize),
    OpenDetails(usize),
    ToggleHelp,
    Escape,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "io.github.atraxsrc.Apsis";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) | Err((_, config)) => config,
            })
            .unwrap_or_default();
        let app = AppModel {
            core,
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
            icon: symbolic_icon(),
        };
        (app, Task::none())
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// The panel button, with a tooltip saying how old the newest snapshot is. Left click opens
    /// the popup, right click the menu. The button itself only reacts to the left button.
    fn view(&self) -> Element<'_, Self::Message> {
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
        let content = widget::column::with_children(vec![
            self.header(),
            panes.into(),
            self.activity(),
            self.prompt(),
            self.hints(),
        ])
        .spacing(6)
        .padding([10, 12]);

        self.core
            .applet
            .popup_container(content)
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
        if self.popup.is_some() && (self.loading || self.running.is_some()) {
            subscriptions.push(time::every(Duration::from_millis(80)).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

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
            },
            Message::Submit => return self.submit(),
            Message::InputUnfocused if self.popup.is_some() => return focus_input(),
            Message::InputUnfocused => {}
            Message::Refresh => return self.start_list(),
            Message::StartCreate => return self.on_key(KeyAction::Create),
            Message::StartDelete => return self.on_key(KeyAction::Delete),
            Message::Finished(operation, result) => return self.on_finished(&operation, result),
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
            Message::OpenDetails(index) => {
                self.selected = index;
                self.overlay = Overlay::Details;
            }
            Message::ToggleHelp => self.toggle_overlay(Overlay::Help),
            Message::Escape => return self.escape(),
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

// Update helpers.
impl AppModel {
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

    /// The popup closed: drop overlays and any half-typed prompt. A running operation and its
    /// status line stay.
    fn reset_popup_state(&mut self) {
        self.overlay = Overlay::None;
        self.prompt = Prompt::Command;
    }

    /// Lists in the background, through `apsis-helper` or pkexec. Does nothing while a list,
    /// create or delete is running (Timeshift runs one at a time).
    fn start_list(&mut self) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() {
            return Task::none();
        }
        self.loading = true;
        self.spinner = 0;
        let pkexec = Arc::clone(&self.pkexec);
        cosmic::task::future(async move { Message::Listed(list_snapshots(pkexec).await) })
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
        let count = self.snapshots().len();
        let target = match action {
            KeyAction::Up => self.selected.checked_sub(1),
            KeyAction::Down => Some(self.selected + 1).filter(|&i| i < count),
            KeyAction::First => (count > 0).then_some(0),
            KeyAction::Last => count.checked_sub(1),
            KeyAction::Details => {
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
        };
        let Some(index) = target else {
            return Task::none();
        };
        self.selected = index;
        // Scrolling to selected / (count - 1) of the way down always keeps an equal-height row
        // in view, from the first row at the top to the last at the bottom.
        #[allow(clippy::cast_precision_loss, reason = "a handful of rows")]
        let y = if count > 1 {
            index as f32 / (count - 1) as f32
        } else {
            0.0
        };
        snap_to(
            LIST_ID.clone(),
            RelativeOffset {
                x: None,
                y: Some(y),
            },
        )
    }

    /// Enter: runs what the prompt asked for, or shows details.
    fn submit(&mut self) -> Task<cosmic::Action<Message>> {
        match std::mem::replace(&mut self.prompt, Prompt::Command) {
            Prompt::Command => {
                if !self.snapshots().is_empty() {
                    self.toggle_overlay(Overlay::Details);
                }
                Task::none()
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
        }
    }

    /// Creates or deletes in the background, through `apsis-helper` or pkexec.
    fn run(&mut self, operation: Operation) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() {
            return Task::none();
        }
        self.running = Some(operation.clone());
        self.status = None;
        self.spinner = 0;
        let pkexec = Arc::clone(&self.pkexec);
        cosmic::task::future(async move {
            let result = operate(pkexec, operation.clone()).await;
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

    /// Esc cancels a prompt, then closes the overlay, then the popup.
    fn escape(&mut self) -> Task<cosmic::Action<Message>> {
        if self.prompt != Prompt::Command {
            self.prompt = Prompt::Command;
            self.status = None;
            // Esc also unfocused the `>` line.
            return focus_input();
        }
        if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
            // Esc also unfocused the `>` line.
            return focus_input();
        }
        match self.popup.take() {
            Some(popup) => {
                self.reset_popup_state();
                destroy_popup(popup)
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

    /// The left pane's title: what it shows.
    fn body_title(&self) -> String {
        match self.overlay {
            Overlay::Help => fl!("pane-help"),
            Overlay::About => fl!("pane-about"),
            Overlay::None | Overlay::Details => fl!("pane-snapshots"),
        }
    }

    /// Height of the snapshots and details panes' contents, in rows: the list's length between
    /// [`MIN_ROWS`] and [`VISIBLE_ROWS`] (longer lists scroll), or enough for what's shown instead.
    fn pane_rows(&self) -> u16 {
        match (self.overlay, &self.listing) {
            (Overlay::Help, _) | (Overlay::None | Overlay::Details, Listing::Failed(_)) => {
                VISIBLE_ROWS
            }
            (Overlay::About, _) => MIN_ROWS + 1,
            (_, Listing::Loaded(list)) => u16::try_from(list.snapshots.len())
                .unwrap_or(u16::MAX)
                .clamp(MIN_ROWS, VISIBLE_ROWS),
            (_, Listing::NotLoaded) => MIN_ROWS,
        }
    }

    fn pane_height(&self) -> f32 {
        ROW_HEIGHT * f32::from(self.pane_rows())
    }

    /// The left pane: the list (or why there is none), help or About.
    fn body(&self) -> Element<'_, Message> {
        let content = match (self.overlay, &self.listing) {
            (Overlay::Help, _) => scroll(help()),
            (Overlay::About, _) => scroll(about()),
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
            .height(Length::Fixed(self.pane_height()))
            .into()
    }

    /// The right pane: everything about the selected snapshot.
    fn details(&self) -> Element<'_, Message> {
        let content = match self.snapshots().get(self.selected) {
            Some(snapshot) => scroll(details(snapshot)),
            None => widget::column::with_children(vec![
                monotext(fl!("details-none"))
                    .class(theme::Text::Custom(dim_text))
                    .into(),
            ])
            .padding([4, 6])
            .into(),
        };
        container(content)
            .width(Length::Fill)
            .height(Length::Fixed(self.pane_height()))
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
        pane(
            fl!("pane-activity"),
            self.running.is_some(),
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
            fmt::truncate(&fmt::quoted_comment(snapshot), ROW_COMMENT_CHARS),
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
            .on_double_click(Message::OpenDetails(index))
            .interaction(mouse::Interaction::Pointer)
            .into()
    }

    /// The footer: `[c]reate  [d]elete  [r]efresh  [?]help            [esc]`. Each hint is a
    /// button.
    fn hints(&self) -> Element<'_, Message> {
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
            hint("[?]help", Some(Message::ToggleHelp)),
            widget::space::horizontal().into(),
            hint("[esc]", Some(Message::Escape)),
        ])
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
/// else through pkexec (a password prompt each time).
async fn list_snapshots(pkexec: Arc<Cli>) -> Result<SnapshotList, CliError> {
    if let Some(helper) = HelperClient::connect().await {
        return helper.list().await.map_err(CliError::from);
    }
    blocking(move || pkexec.list()).await
}

/// Creates or deletes through `apsis-helper` when it's installed, else through pkexec. With
/// the helper this returns when its `Finished` signal arrives, however long Timeshift takes.
///
/// A helper that's installed but fails is reported, not replaced by pkexec, so a broken
/// install gets noticed.
async fn operate(pkexec: Arc<Cli>, operation: Operation) -> Result<(), CliError> {
    if let Some(helper) = HelperClient::connect().await {
        let done = match &operation {
            Operation::Create(comment) => helper.create(comment).await,
            Operation::Delete(name) => helper.delete(name).await,
        };
        return done.map_err(CliError::from);
    }
    blocking(move || match &operation {
        Operation::Create(comment) => pkexec.create(comment),
        Operation::Delete(name) => pkexec.delete(name),
    })
    .await
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
        ("Enter", fl!("help-details")),
        ("c", fl!("help-create")),
        ("d", fl!("help-delete")),
        ("r", fl!("help-refresh")),
        ("?", fl!("help-help")),
        ("Esc", fl!("help-escape")),
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
        assert_eq!(char_action('x'), None);
        assert_eq!(char_action('J'), None);
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
        assert_eq!(action(character("x"), ignored), None);
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
    fn enter_and_double_click_make_details_the_active_pane() {
        let mut app = listed(DEVICE_LIST);
        send(&mut app, Message::Submit);
        assert_eq!(app.overlay, Overlay::Details);
        send(&mut app, Message::Submit);
        assert_eq!(app.overlay, Overlay::None);

        send(&mut app, Message::OpenDetails(1));
        assert_eq!((app.overlay, app.selected), (Overlay::Details, 1));
        send(&mut app, Message::Select(2));
        assert_eq!((app.overlay, app.selected), (Overlay::None, 2));

        send(&mut app, Message::OpenDetails(0));
        send(&mut app, Message::Escape);
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.popup.is_some());
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

    #[test]
    fn modifier_combinations_are_left_alone() {
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            let event = press(character("r"), modifiers);
            assert!(key_action(event, event::Status::Ignored, Id::unique()).is_none());
        }
    }
}
