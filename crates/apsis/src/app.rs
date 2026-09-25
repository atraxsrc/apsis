// SPDX-License-Identifier: GPL-3.0-only

use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use apsis_core::{Backend, PkexecRunner, Snapshot, SnapshotList, TimeshiftCli};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::keyboard::{self, Key, key::Named};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::widget::scrollable::{Direction, RelativeOffset, Scrollbar, snap_to};
use cosmic::iced::widget::svg as iced_svg;
use cosmic::iced::widget::text as iced_text;
use cosmic::iced::{Alignment, Background, Border, Color, Length, Limits, Subscription};
use cosmic::iced::{event, mouse, time, window::Id};
use cosmic::prelude::*;
use cosmic::widget::text::monotext;
use cosmic::widget::{self, container, icon};
use cosmic::{Theme, theme};

use crate::config::Config;
use crate::fl;
use crate::fmt;

/// Popup width in logical pixels (UI.md: ~520).
const POPUP_WIDTH: f32 = 520.0;
/// Height of one snapshot row: monotext line height (20) plus vertical padding.
const ROW_HEIGHT: f32 = 24.0;
/// Rows shown before the list scrolls.
const VISIBLE_ROWS: u16 = 8;
/// Longest comment shown in a row; the details view shows all of it.
const ROW_COMMENT_CHARS: usize = 28;
/// Lines of stderr shown in the error state.
const STDERR_LINES: usize = 6;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Themed name of the panel icon; installed by `just install`.
const SYMBOLIC_ICON: &str = "io.github.atraxsrc.Apsis-symbolic";
/// The same SVG, for when it isn't installed yet (`just run`).
const SYMBOLIC_ICON_SVG: &[u8] = include_bytes!(
    "../../../resources/icons/hicolor/symbolic/apps/io.github.atraxsrc.Apsis-symbolic.svg"
);
/// Size of the icon in the popup header, matching the monotext line height.
const HEADER_ICON_SIZE: u16 = 16;

static LIST_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("snapshot-list"));

type Cli = TimeshiftCli<PkexecRunner>;

/// The applet: a panel button and a terminal-style popup listing Timeshift snapshots.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// The popup id.
    popup: Option<Id>,
    /// Configuration data that persists between application runs.
    config: Config,
    /// Shared with the background task that runs `pkexec timeshift --list`.
    backend: Arc<Cli>,
    listing: Listing,
    /// A list is running in the background.
    loading: bool,
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
    Failed(ListError),
}

/// A `Clone`able summary of [`apsis_core::Error`] for messages and the view.
#[derive(Debug, Clone)]
pub enum ListError {
    NotInstalled,
    Failed {
        code: Option<i32>,
        stderr: Vec<String>,
    },
    Other(String),
}

impl From<apsis_core::Error> for ListError {
    fn from(error: apsis_core::Error) -> Self {
        match error {
            apsis_core::Error::NotInstalled => Self::NotInstalled,
            apsis_core::Error::Failed { code, stderr } => Self::Failed {
                code,
                stderr: fmt::tail(&stderr, STDERR_LINES),
            },
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Details,
    Help,
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
    Escape,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
    UpdateConfig(Config),
    Key(Id, KeyAction),
    Refresh,
    Listed(Result<SnapshotList, ListError>),
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
            config,
            backend: Arc::new(TimeshiftCli::new(PkexecRunner)),
            listing: Listing::NotLoaded,
            loading: false,
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

    /// The panel button, with a tooltip saying how old the newest snapshot is.
    fn view(&self) -> Element<'_, Self::Message> {
        let button = self
            .core
            .applet
            .icon_button_from_handle(self.icon.clone())
            .on_press(Message::TogglePopup);
        self.core
            .applet
            .applet_tooltip(
                button,
                self.tooltip(),
                self.popup.is_some(),
                Message::Surface,
                None,
            )
            .into()
    }

    /// The terminal-style popup.
    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let content = widget::column::with_children(vec![
            self.header(),
            widget::divider::horizontal::default().into(),
            self.body(),
            widget::divider::horizontal::default().into(),
            self.hints(),
            self.prompt(),
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
        if self.popup.is_some() {
            subscriptions.push(event::listen_with(key_action));
            if self.loading {
                subscriptions.push(time::every(Duration::from_millis(80)).map(|_| Message::Tick));
            }
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
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.overlay = Overlay::None;
                }
            }
            Message::Key(id, action) if self.popup == Some(id) => return self.on_key(action),
            Message::Key(..) => {}
            Message::Refresh => return self.start_list(),
            Message::Listed(result) => self.on_listed(result),
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
            self.overlay = Overlay::None;
            return destroy_popup(popup);
        }
        let Some(parent) = self.core.main_window_id() else {
            return Task::none();
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
        let open = get_popup(settings);
        if matches!(self.listing, Listing::NotLoaded) {
            Task::batch([open, self.start_list()])
        } else {
            open
        }
    }

    /// Runs `pkexec timeshift --list` on a blocking thread. Does nothing if one is running.
    fn start_list(&mut self) -> Task<cosmic::Action<Message>> {
        if self.loading {
            return Task::none();
        }
        self.loading = true;
        self.spinner = 0;
        let backend = Arc::clone(&self.backend);
        cosmic::task::future(async move {
            let result = match tokio::task::spawn_blocking(move || backend.list()).await {
                Ok(listed) => listed.map_err(ListError::from),
                Err(join) => Err(ListError::Other(join.to_string())),
            };
            Message::Listed(result)
        })
    }

    fn on_listed(&mut self, result: Result<SnapshotList, ListError>) {
        self.loading = false;
        match result {
            Ok(mut list) => {
                // Keep the same snapshot selected across refreshes when it still exists.
                let selected_name = self.snapshots().get(self.selected).map(|s| s.name.clone());
                list.snapshots.sort_by_key(|s| std::cmp::Reverse(s.created));
                self.selected = selected_name
                    .and_then(|name| list.snapshots.iter().position(|s| s.name == name))
                    .unwrap_or(0);
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

    fn on_key(&mut self, action: KeyAction) -> Task<cosmic::Action<Message>> {
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

    /// Esc closes the overlay if one is open, otherwise the popup.
    fn escape(&mut self) -> Task<cosmic::Action<Message>> {
        if self.overlay != Overlay::None {
            self.overlay = Overlay::None;
            return Task::none();
        }
        match self.popup.take() {
            Some(popup) => destroy_popup(popup),
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

    fn body(&self) -> Element<'_, Message> {
        match self.overlay {
            Overlay::Help => return help(),
            Overlay::Details => {
                if let Some(snapshot) = self.snapshots().get(self.selected) {
                    return details(snapshot);
                }
            }
            Overlay::None => {}
        }
        match &self.listing {
            Listing::Loaded(list) if list.snapshots.is_empty() => {
                if list.device.is_none() {
                    lines([fl!("no-device"), fl!("no-device-hint")])
                } else {
                    lines([fl!("empty")])
                }
            }
            Listing::Loaded(list) => self.list(&list.snapshots),
            Listing::Failed(error) => error_view(error),
            Listing::NotLoaded if self.loading => lines([fl!("waiting")]),
            Listing::NotLoaded => lines([fl!("not-loaded")]),
        }
    }

    fn list<'a>(&'a self, snapshots: &'a [Snapshot]) -> Element<'a, Message> {
        let rows = snapshots
            .iter()
            .enumerate()
            .map(|(index, snapshot)| self.row(index, snapshot));
        let scroll = widget::scrollable(widget::column::with_children(rows))
            .id(LIST_ID.clone())
            .padding(0.0)
            .direction(Direction::Vertical(
                Scrollbar::new().width(4.0).scroller_width(4.0).spacing(4.0),
            ))
            .height(Length::Shrink);
        container(scroll)
            .max_height(ROW_HEIGHT * f32::from(VISIBLE_ROWS))
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
            monotext(text).into(),
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

    /// `[r]efresh  [?]help                          [esc]`. Each hint is a button.
    fn hints(&self) -> Element<'_, Message> {
        let refresh = if matches!(self.listing, Listing::Failed(_)) {
            "[r]etry"
        } else {
            "[r]efresh"
        };
        widget::row::with_children(vec![
            hint(refresh, (!self.loading).then_some(Message::Refresh)),
            hint("[?]help", Some(Message::ToggleHelp)),
            widget::space::horizontal().into(),
            hint("[esc]", Some(Message::Escape)),
        ])
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// `> _`, or `> timeshift --list ⠹` while listing.
    fn prompt(&self) -> Element<'_, Message> {
        let text = if self.loading {
            format!("timeshift --list {}", SPINNER[self.spinner])
        } else {
            "_".to_owned()
        };
        widget::row::with_children(vec![
            monotext(">").class(theme::Text::Accent).into(),
            monotext(text).into(),
        ])
        .spacing(8)
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

fn lines<'a>(lines: impl IntoIterator<Item = String>) -> Element<'a, Message> {
    widget::column::with_children(lines.into_iter().map(|l| monotext(l).into()))
        .spacing(2)
        .padding([4, 6])
        .into()
}

fn error_view(error: &ListError) -> Element<'_, Message> {
    let mut out = Vec::new();
    match error {
        ListError::NotInstalled => out.push(fl!("not-installed")),
        ListError::Failed { code, stderr } => {
            out.push(match code {
                Some(code) => fl!("failed-code", code = code.to_string()),
                None => fl!("failed-signal"),
            });
            // pkexec: 126 = not authorised or dialog dismissed, 127 = couldn't authenticate.
            if matches!(code, Some(126 | 127)) {
                out.push(fl!("failed-auth"));
            }
            out.extend(stderr.iter().cloned());
        }
        ListError::Other(message) => out.push(fl!("failed-other", message = message.clone())),
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

fn details(snapshot: &Snapshot) -> Element<'_, Message> {
    let comment = snapshot.comment.clone().unwrap_or_else(|| "-".to_owned());
    let fields = [
        (fl!("details-name"), snapshot.name.clone()),
        (
            fl!("details-created"),
            format!(
                "{} ({})",
                snapshot.created.strftime("%Y-%m-%d %H:%M:%S"),
                fmt::ago(snapshot.created, jiff::Zoned::now().datetime())
            ),
        ),
        (fl!("details-tags"), fmt::tag_names(&snapshot.tags)),
        (fl!("details-comment"), comment),
    ];
    key_value_rows(fields)
}

fn help() -> Element<'static, Message> {
    let keys = [
        ("↑ ↓  j k", fl!("help-move")),
        ("Home End", fl!("help-ends")),
        ("Enter", fl!("help-details")),
        ("r", fl!("help-refresh")),
        ("?", fl!("help-help")),
        ("Esc", fl!("help-escape")),
    ];
    key_value_rows(keys.map(|(k, v)| (k.to_owned(), v)))
}

fn key_value_rows<'a>(rows: impl IntoIterator<Item = (String, String)>) -> Element<'a, Message> {
    let rows = rows.into_iter().map(|(key, value)| {
        widget::row::with_children(vec![
            monotext(key)
                .class(theme::Text::Accent)
                .width(Length::Fixed(96.0))
                .into(),
            monotext(value).into(),
        ])
        .spacing(8)
        .into()
    });
    widget::column::with_children(rows)
        .spacing(2)
        .padding([4, 6])
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

/// Maps key presses to [`KeyAction`]s. Ctrl/Alt/Super combinations are left alone.
fn key_action(event: event::Event, _status: event::Status, window: Id) -> Option<Message> {
    let event::Event::Keyboard(keyboard::Event::KeyPressed {
        modified_key,
        modifiers,
        ..
    }) = event
    else {
        return None;
    };
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    let action = match modified_key.as_ref() {
        Key::Named(Named::ArrowUp) | Key::Character("k") => KeyAction::Up,
        Key::Named(Named::ArrowDown) | Key::Character("j") => KeyAction::Down,
        Key::Named(Named::Home) => KeyAction::First,
        Key::Named(Named::End) => KeyAction::Last,
        Key::Named(Named::Enter) => KeyAction::Details,
        Key::Character("r") => KeyAction::Refresh,
        Key::Character("?") => KeyAction::Help,
        Key::Named(Named::Escape) => KeyAction::Escape,
        _ => return None,
    };
    Some(Message::Key(window, action))
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
