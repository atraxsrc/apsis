// SPDX-License-Identifier: GPL-3.0-only

//! Apsis's UI: the panel button (icon, optional label, tooltip, right-click menu), its
//! read-only popup, and the window (`apsis --window`), laid out like Timeshift's: a toolbar,
//! the snapshot list and a status area, with the settings as tabs. Standard libcosmic widgets
//! and COSMIC theme colours only (`docs/APSIS-UI-PROMPT.md`).
//!
//! State and updates are here; drawing is in `view`.

mod view;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::future::Future;
use std::time::{Duration, Instant};

use apsis_core::config::{Config as ApsisConfig, ConfigInfo};
use apsis_core::helper::{HelperClient, JobEvent};
use apsis_core::job::{Job, JobKind, JobState};
use apsis_core::restore::apsis::InSnapshot;
use apsis_core::restore::dialog::Dialog as Check;
use apsis_core::restore::filter::Home;
use apsis_core::restore::plan;
use apsis_core::restore::refusal::{Refusal, Unreadable};
use apsis_core::restore::state::{Outcome, RestoreResult, ResultState};
use apsis_core::status::{BY_UUID, disk_connected};
use apsis_core::{
    ApsisStatus, MAX_COMMENT_CHARS, Progress, Snapshot, SnapshotList, validate_comment,
};
use cosmic::applet::token::subscription::{
    TokenRequest, TokenUpdate, activation_token_subscription,
};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::{Stream, StreamExt, stream};
use cosmic::iced::keyboard::{self, Key, Modifiers, key::Named};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{Limits, Size, Subscription, event, time, window, window::Id};
use cosmic::prelude::*;
use cosmic::widget::{self, icon, segmented_button};

use crate::config::Config;
use crate::fl;
use crate::fmt;
use crate::settings_view::{ApsisChoice, Row, Section, SettingsView, picked_filter, typed_filter};
use crate::status::StatusView;

/// Window mode: the size it opens at.
const WINDOW_SIZE: Size = Size::new(720.0, 520.0);
/// Window mode: the smallest it can be dragged to, with the toolbar, a few rows and the status
/// area still in view.
const WINDOW_MIN_SIZE: Size = Size::new(640.0, 440.0);
// The window opens no smaller than it may be dragged to.
const _: () = assert!(
    WINDOW_MIN_SIZE.width <= WINDOW_SIZE.width && WINDOW_MIN_SIZE.height <= WINDOW_SIZE.height
);
/// Width of the edge that resizes the window when dragged, in logical pixels.
const WINDOW_RESIZE_BORDER: f64 = 8.0;
/// If the window never reports focus, it becomes resizable this long after starting anyway.
const WINDOW_SHOWN_FALLBACK: Duration = Duration::from_millis(1500);
/// The panel popup's width.
const POPUP_WIDTH: f32 = 360.0;
/// Newest snapshots the popup lists.
const POPUP_ROWS: usize = 5;
/// Width of the right-click menu.
const MENU_WIDTH: f32 = 240.0;
/// While the helper is busy with a job this window can't see: how often to list again. A
/// fallback only: `JobChanged` normally says when the job ends.
const BUSY_RETRY_EVERY: Duration = Duration::from_secs(5);
/// How often the backup disk's `/dev/disk/by-uuid` link is looked at (no root, no mount).
const DISK_CHECK_EVERY: Duration = Duration::from_secs(5);
/// How often the panel lists in the background (through the helper only) for the reminder.
const BACKGROUND_LIST_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How often the elapsed time of a job without a percent is redrawn.
const TICK_EVERY: Duration = Duration::from_millis(500);

/// Themed name of the panel icon; installed by `just install`.
const SYMBOLIC_ICON: &str = "io.github.atraxsrc.Apsis-symbolic";
/// The same SVG, for when it isn't installed yet (`just run`).
const SYMBOLIC_ICON_SVG: &[u8] = include_bytes!(
    "../../../resources/icons/hicolor/symbolic/apps/io.github.atraxsrc.Apsis-symbolic.svg"
);

/// For the About page, from `Cargo.toml`.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// How Apsis was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Started by cosmic-panel: a panel button that opens the popup.
    Applet,
    /// `apsis --window`, or started outside the panel: the window.
    Window,
}

/// The page `apsis --settings` or `--about` opens the window on. A second start hands it to
/// the window that's already open (libcosmic's single instance, as the D-Bus action name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartView {
    Settings,
    About,
}

impl StartView {
    /// The first `--settings` or `--about` among the arguments.
    pub fn from_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<Self> {
        args.into_iter().find_map(|arg| match arg.to_str()? {
            "--settings" => Some(Self::Settings),
            "--about" => Some(Self::About),
            _ => None,
        })
    }

    /// The action name a second start sends.
    fn from_action(action: &str) -> Option<Self> {
        match action {
            "settings" => Some(Self::Settings),
            "about" => Some(Self::About),
            _ => None,
        }
    }
}

impl std::fmt::Display for StartView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Settings => "settings",
            Self::About => "about",
        })
    }
}

/// How Apsis was started, and for a window the page it opens on.
#[derive(Debug, Clone, Copy)]
pub struct Flags {
    pub mode: Mode,
    pub view: Option<StartView>,
}

impl cosmic::app::CosmicFlags for Flags {
    type SubCommand = StartView;
    type Args = Vec<String>;

    fn action(&self) -> Option<&StartView> {
        self.view.as_ref()
    }
}

/// Runs the window, resizable by its edges and corners.
///
/// It starts floating, even with tiling on: COSMIC (cosmic-comp `Shell::map_window`) decides
/// floating or tiled once, when a window first appears, and floats one whose minimum and
/// maximum size are equal. So it opens with both at [`WINDOW_SIZE`], and once it's on screen
/// ([`Message::WindowShown`]) the maximum goes and the minimum drops to [`WINDOW_MIN_SIZE`].
///
/// One window at a time: if one is open (it owns the app ID on the session bus), this start
/// asks it to come forward (with the launcher's or applet's activation token, and `view`) and
/// exits. `COSMIC_SINGLE_INSTANCE=0` turns that off.
pub fn run_window(view: Option<StartView>) -> cosmic::iced::Result {
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
    let flags = Flags {
        mode: Mode::Window,
        view,
    };
    cosmic::app::run_single_instance::<AppModel>(settings, flags)
}

/// Apsis: the panel button and its popup, or the window.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    mode: Mode,
    /// The popup's id; in window mode, the window's while it's open.
    popup: Option<Id>,
    /// The right-click menu's popup id. At most one of `popup` and `menu` is open.
    menu: Option<Id>,
    /// Apsis's own per-user settings (cosmic-config).
    config: Config,
    listing: Listing,
    /// A list is running in the background.
    loading: bool,
    /// UUID of the backup disk from the last good list, to name it when it goes missing.
    known_uuid: Option<String>,
    /// Whether the backup disk's UUID link was there at the last look (see
    /// [`AppModel::check_disk`]); `None` before the first.
    disk_seen: Option<bool>,
    /// Where to look for it: [`BY_UUID`] (the tests use a folder of their own).
    disk_links: std::path::PathBuf,
    /// A create or delete this window started, running in the helper.
    running: Option<Operation>,
    /// The helper's job, as `Job` and `JobChanged` say: this window's or anyone else's.
    job: Option<Job>,
    /// The running create's last progress. `None` until the first.
    progress: Option<Progress>,
    /// When this window's create started, for the elapsed time.
    run_started: Option<Instant>,
    /// How the last job went, or why something was refused; the status area's line.
    status: Option<Status>,
    /// Selected rows: snapshot and leftover names.
    selection: BTreeSet<String>,
    /// The row a Shift-click extends the selection from.
    anchor: Option<usize>,
    /// Ctrl and Shift, for clicks in the list.
    modifiers: Modifiers,
    page: Page,
    /// The settings' tabs.
    tabs: segmented_button::SingleSelectModel,
    /// Apsis's config, for the settings and the Create dialog.
    settings: SettingsLoad,
    /// A config write is running in the helper.
    saving_settings: bool,
    dialog: Option<Dialog>,
    about: widget::about::About,
    /// Window mode: the window is on screen and its size limits are relaxed (see
    /// [`run_window`]).
    window_resizable: bool,
    /// The symbolic Apsis icon, from the icon theme or embedded.
    icon: icon::Handle,
    /// A background list went through `apsis-helper` (so more can, without a password).
    helper_found: bool,
    /// The helper answered busy: a job this window didn't start runs. Cleared when a list
    /// goes through.
    helper_busy: bool,
    /// The applet: asks the compositor for an activation token before starting the window, so
    /// the window (or the one already open) may take focus. `None` until it's ready, or if the
    /// compositor has none to give; the window then starts without one.
    token_requests: Option<TokenSender>,
    /// The helper, on this process's one system-bus connection (made at start, re-made when
    /// the bus drops). `None` until it's made, or when there's no helper.
    helper: Option<HelperClient>,
    /// Counts the connections made, so the job subscription follows the current one.
    bus_generation: u32,
    /// This window's operation has ended (`Finished` came) but the helper's end announcement
    /// for its job hasn't arrived yet: the next end of that kind is this job's, not another
    /// window's (see [`AppModel::on_job`]).
    own_end: Option<OwnEnd>,
    /// The helper announced the end of this window's running operation before its `Finished`
    /// came (the other order): nothing is left to wait for then.
    own_job_ended: bool,
    /// `CheckRestore` is running for this snapshot (the dialog opens when it answers).
    checking: Option<String>,
    /// This window's restore plan waits at the ready prompt (PLAN 6b.5): the helper refuses
    /// writes until "Restart now" or "Cancel restore".
    ready: Option<String>,
    /// `CancelRestore` / `RestartToRestore` is on its way to the helper.
    cancelling: bool,
    restarting: bool,
    /// The last restore's outcome (`RestoreResult`), shown on the status line after login.
    restore_result: Option<RestoreResult>,
    /// The window's height, for the restore dialog's scrolling body (PLAN 6b.8).
    window_height: f32,
}

/// The end announcement this window still expects for its own job, after its `Finished`.
///
/// The helper frees the lock, announces the end (`JobChanged`, through a task of its own)
/// and sends `Finished` to the caller; on the bus the two can arrive in either order. An
/// end that comes after `Finished` must not be taken for another window's job.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnEnd {
    kind: JobKind,
    /// The job's start time as the helper announced it, when this window saw the job
    /// running; `None` when `Finished` came before any announcement (then the kind alone
    /// says, and only within [`OWN_END_GRACE`]).
    started: Option<i64>,
    finished_at: Instant,
}

/// How long after its `Finished` a window still takes an end of its job's kind, with no
/// announced start time to match, for its own. The announcement follows `Finished` by
/// milliseconds; a job elsewhere can't even begin before the own end is announced.
const OWN_END_GRACE: Duration = Duration::from_secs(5);

impl OwnEnd {
    /// Whether `job` is the job this is waiting for the end of.
    fn is(&self, job: &Job) -> bool {
        job.kind == self.kind
            && match self.started {
                Some(started) => job.started == started,
                None => self.finished_at.elapsed() < OWN_END_GRACE,
            }
    }
}

/// What a task reaches the helper through: the process's connection, or `None` before it's
/// made (the task then connects for itself, once).
type Link = Option<HelperClient>;

/// What the job subscription runs on: the connection, told apart by its generation (the
/// client itself isn't hashable, and a new connection is a new generation anyway).
struct JobSource {
    generation: u32,
    client: HelperClient,
}

impl std::hash::Hash for JobSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.generation.hash(state);
    }
}

/// Where the applet sends its activation token requests.
type TokenSender = cosmic::cctk::sctk::reexports::calloop::channel::Sender<TokenRequest>;

/// Where Apsis's config is.
#[derive(Debug, Clone)]
enum SettingsLoad {
    NotLoaded,
    Loading,
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The helper is running another job. Nothing ran.
    Busy,
    /// The backup disk left while a create or delete ran; `reason` is what failed because of
    /// it (the helper checks, see `apsis_core::Error::DeviceRemoved`).
    DiskRemoved {
        reason: String,
    },
    /// A create was stopped; what it had copied is gone.
    Stopped,
    /// A delete of several stopped at `failed` (`reason` says why): `deleted` are gone,
    /// `left` weren't touched. From the helper's own account, not from counting.
    DeleteManyStopped {
        deleted: Vec<String>,
        failed: String,
        left: Vec<String>,
        reason: Box<CliError>,
    },
    /// The helper refused a full-system restore (PLAN 6b.7). Nothing ran, or (from "Restart
    /// now") the plan was dropped.
    RestoreRefused(Refusal),
    /// The helper refused a delete because a restore is armed and waits for the restart.
    /// Nothing ran and no job began.
    RestoreArmed,
    Other(String),
}

impl From<apsis_core::Error> for CliError {
    fn from(error: apsis_core::Error) -> Self {
        match error {
            apsis_core::Error::RestoreRefused(word) => match Refusal::from_wire(&word) {
                Some(refusal) => Self::RestoreRefused(refusal),
                None => Self::Other(word),
            },
            apsis_core::Error::RestoreArmed => Self::RestoreArmed,
            apsis_core::Error::NotAuthorized => Self::NotAuthorized,
            apsis_core::Error::DeviceNotFound { device } => Self::DeviceNotFound { device },
            apsis_core::Error::Busy => Self::Busy,
            apsis_core::Error::DeviceRemoved { reason, .. } => Self::DiskRemoved { reason },
            apsis_core::Error::Stopped => Self::Stopped,
            // The helper's own words (or what the bus said), without the crate's
            // `apsis-helper: ` prefix: the status line, the tooltip and the dialogs show
            // them as they are. The journal keeps its prefix (that's the helper's `log`).
            apsis_core::Error::Helper(message) => Self::Other(message),
            apsis_core::Error::DeleteManyStopped {
                deleted,
                failed,
                left,
                reason,
            } => Self::DeleteManyStopped {
                deleted,
                failed,
                left,
                reason: Box::new(Self::from(*reason)),
            },
            other => Self::Other(other.to_string()),
        }
    }
}

/// A create or delete this window runs, as root, through the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// With the comment as typed; the helper trims it.
    Create(String),
    /// One snapshot (or leftover).
    Delete(String),
    /// Several, as one job in the helper (`DeleteMany`), in list order; it stops at the first
    /// failure and says what was deleted.
    DeleteMany(Vec<String>),
    /// A full-system restore's preparation (PLAN 6b.4): ends "ready" at the prompt.
    Restore {
        snapshot: String,
        restore_home: bool,
        safety_snapshot: bool,
    },
}

impl Operation {
    /// The job the helper runs for it, as `Job` and `JobChanged` name it.
    fn kind(&self) -> JobKind {
        match self {
            Self::Create(_) => JobKind::Create,
            Self::Delete(_) => JobKind::Delete,
            Self::DeleteMany(_) => JobKind::DeleteMany,
            Self::Restore { .. } => JobKind::Restore,
        }
    }
}

/// The status area's result line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Info(String),
    /// With details for the tooltip (the helper's own words), if any.
    Error(String, Option<String>),
}

/// What the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    List,
    Settings,
    About,
}

/// A dialog over the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// Create, with an optional comment; `error` says why it can't go yet.
    Create {
        comment: String,
        error: Option<String>,
    },
    /// Delete these snapshots (or leftovers), in list order.
    Delete { names: Vec<String> },
    /// Stop the running create that makes `snapshot`.
    Stop { snapshot: String },
    /// A typed filter; `error` says why it wasn't taken.
    AddPattern { text: String, error: Option<String> },
    /// Leaving the settings with unsaved changes.
    Unsaved,
    /// Restore `snapshot` (PLAN 6b.8): what `CheckRestore` said, and the two choices.
    Restore {
        snapshot: String,
        check: Check,
        restore_home: bool,
        safety_snapshot: bool,
    },
    /// "Can't restore this snapshot": the reason; `dropped` when a ready plan was dropped by
    /// a refused "Restart now" (owner, 2026-10-02).
    Refused {
        snapshot: String,
        refusal: Refusal,
        dropped: bool,
    },
    /// Stop the restore's preparation (the safety snapshot being made goes).
    StopRestore { snapshot: String },
    /// "Ready to restore": Restart now, or Cancel restore (also Esc).
    Ready { snapshot: String },
}

impl Dialog {
    /// The "lost for good" line shows only with "Restore them too" and no safety snapshot.
    fn lost_for_good(&self) -> bool {
        matches!(
            self,
            Self::Restore {
                restore_home: true,
                safety_snapshot: false,
                ..
            }
        )
    }

    /// The restore dialog's extra lines, in order: no home in the snapshot, the old format,
    /// which Apsis the snapshot holds (PLAN 6b.8).
    fn extra_lines(&self) -> Vec<String> {
        let Self::Restore { check, .. } = self else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        if !check.has_home {
            lines.push(fl!("restore-no-home"));
        }
        if check.old_format {
            lines.push(fl!("restore-old-format"));
        }
        match &check.apsis {
            InSnapshot::Current => {}
            InSnapshot::NotInstalled => lines.push(fl!("restore-no-apsis")),
            InSnapshot::NoRestore { version } => {
                lines.push(fl!("restore-apsis-no-restore", version = version.as_str()));
            }
            InSnapshot::OldSettings { version } => {
                lines.push(fl!(
                    "restore-apsis-old-settings",
                    version = version.as_str()
                ));
            }
        }
        lines
    }

    /// The "Can't restore" dialog's lines: the reason, what to do, and (a dropped plan) that
    /// a fresh Restore re-measures.
    fn refusal_lines(&self) -> Vec<String> {
        let Self::Refused {
            refusal, dropped, ..
        } = self
        else {
            return Vec::new();
        };
        let (why, what) = refusal_lines(refusal);
        let mut lines = vec![why, what];
        if *dropped {
            lines.push(fl!("refused-dropped"));
        }
        lines
    }
}

/// Each refusal's two lines (PLAN 6b.8's string table).
fn refusal_lines(refusal: &Refusal) -> (String, String) {
    let size = apsis_core::status::size;
    match refusal {
        Refusal::NotUefi => (fl!("refused-not-uefi"), fl!("refused-this-computer")),
        Refusal::NotKernelstub => (
            fl!("refused-not-kernelstub"),
            fl!("refused-this-computer-yet"),
        ),
        Refusal::BootPartition => (fl!("refused-boot-partition"), fl!("refused-this-computer")),
        Refusal::RootFilesystem { fstype } => (
            fl!("refused-root-filesystem", fstype = fstype.as_str()),
            fl!("refused-this-computer-yet"),
        ),
        Refusal::RootDevice => (fl!("refused-root-device"), fl!("refused-this-computer-yet")),
        Refusal::SplitSystem => (
            fl!("refused-split-system"),
            fl!("refused-this-computer-yet"),
        ),
        Refusal::OtherInstallation => (
            fl!("refused-other-installation"),
            fl!("refused-other-installation-do"),
        ),
        Refusal::Unreadable(why) => {
            let reason = match why {
                Unreadable::NoInfo => fl!("refused-unreadable-no-info"),
                Unreadable::NotRsync => fl!("refused-unreadable-not-rsync"),
                Unreadable::NoLocalhost => fl!("refused-unreadable-no-localhost"),
                Unreadable::NoExcludeList => fl!("refused-unreadable-no-exclude-list"),
            };
            (
                fl!("refused-unreadable", reason = reason),
                fl!("refused-pick-another"),
            )
        }
        Refusal::KernelIncomplete => (
            fl!("refused-kernel-incomplete"),
            fl!("refused-pick-another"),
        ),
        Refusal::PendingUpdate => (
            fl!("refused-pending-update"),
            fl!("refused-pending-update-do"),
        ),
        Refusal::RestoreArmed => (
            fl!("refused-restore-armed"),
            fl!("refused-restore-armed-do"),
        ),
        Refusal::PopUpgradePending => (fl!("refused-pop-upgrade"), fl!("refused-pop-upgrade-do")),
        Refusal::CrypttabDiffers => (fl!("refused-crypttab"), fl!("refused-crypttab-do")),
        Refusal::BackupSpace { needs, free } => (
            fl!(
                "refused-backup-space",
                needs = size(*needs),
                free = size(*free)
            ),
            fl!("refused-backup-space-do"),
        ),
        Refusal::SystemSpace { needs, free } => (
            fl!(
                "refused-system-space",
                needs = size(*needs),
                free = size(*free)
            ),
            fl!("refused-system-space-do"),
        ),
        Refusal::BootSpace { needs, free } => (
            fl!(
                "refused-boot-space",
                needs = size(*needs),
                free = size(*free)
            ),
            fl!("refused-boot-space-do"),
        ),
        Refusal::SizeUnknown => (fl!("refused-size-unknown"), fl!("refused-size-unknown-do")),
        Refusal::BootFiles(_) => (fl!("refused-boot-files"), fl!("refused-boot-files-do")),
    }
}

/// The colour of a result line's phrase: the theme's warning or destructive text colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// The system runs, and something needs attention.
    Warning,
    /// The restore is incomplete, or the computer may not start.
    Error,
}

/// A result line in three parts (PLAN 6b.8): `phrase` carries the colour, `before` and
/// `after` are plain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResultPhrase {
    before: String,
    phrase: String,
    after: String,
    tone: Tone,
}

/// `2026-09-25 11:28` for a snapshot name, or the name itself.
fn date_of(name: &str) -> String {
    apsis_core::parse_snapshot_name(name)
        .map(fmt::when)
        .unwrap_or_else(|| name.to_owned())
}

/// Keyboard shortcuts (the man page and README list them; the UI doesn't).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Ctrl+N
    Create,
    /// Delete
    Delete,
    /// Ctrl+R, F5
    Refresh,
    /// Ctrl+,
    Settings,
    /// Ctrl+A
    SelectAll,
    /// Up / Down in the list
    Up,
    Down,
    /// Esc
    Escape,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    /// Right-click on the panel button.
    ToggleMenu,
    /// Menu: open the popup and refresh.
    MenuRefresh,
    /// Menu: `cosmic-settings panel`, where the applet is removed or moved.
    MenuPanelSettings,
    /// Menu: closes it.
    MenuClose,
    /// Opens the window in its own process (`apsis --window`, or with `--settings` or
    /// `--about`), and closes the popup.
    OpenWindow(Option<&'static str>),
    OpenUrl(String),
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
    UpdateConfig(Config),
    Shortcut(Shortcut),
    ModifiersChanged(Modifiers),
    Refresh,
    Listed(Result<SnapshotList, CliError>),
    /// A click on a row (with the Ctrl and Shift state at the time).
    RowClicked(usize),
    /// Toolbar.
    CreateClicked,
    DeleteClicked,
    SettingsClicked,
    AboutClicked,
    /// Back from the settings or About page.
    Back,
    StopClicked,
    /// The helper answered a stop: `None` when it's stopping, else why not (the create goes on).
    StopAnswered(Option<CliError>),
    /// Esc in the popup or the menu.
    Escape,
    /// Dialog input and buttons.
    DialogText(String),
    DialogConfirm,
    DialogCancel,
    /// The Unsaved dialog's third choice: leave without saving.
    DialogDiscard,
    /// This window's create or delete (one step of it) finished.
    Finished(Operation, Result<(), CliError>),
    /// This window's create got this far.
    Progress(Progress),
    /// From the helper's `JobChanged`, or its leaving the bus.
    Job(JobEvent),
    /// What `Job` said when the window or popup opened.
    JobPolled(Option<Job>),
    Tick,
    SettingsRead(Result<ConfigInfo, CliError>),
    /// A settings write finished: a note from the helper (empty when all went well).
    SettingsWritten(Result<String, CliError>),
    Tab(segmented_button::Entity),
    PickDevice(usize),
    Include(Row, bool),
    FilterClicked(usize),
    FilterSign(usize, bool),
    AddFolder,
    AddFile,
    AddPattern,
    /// A file chooser answered: the path and whether it's a folder; `None` if cancelled.
    Picked(Option<(String, bool)>),
    RemoveFilter,
    MoveFilter(bool),
    Save,
    RemindDays(u32),
    ShowLabel(bool),
    /// Window mode: the window is on screen (first focus, or the fallback timer).
    WindowShown,
    /// Every [`BACKGROUND_LIST_EVERY`] while the popup is closed: list again for the reminder.
    BackgroundRefresh,
    /// A background list ended: `None` when there's no helper (nothing was run).
    BackgroundListed(Option<Result<SnapshotList, CliError>>),
    /// The applet's activation token channel, or a token for starting the window.
    Token(TokenUpdate),
    /// Every [`DISK_CHECK_EVERY`]: is the backup disk still (or again) connected?
    DiskCheck,
    /// Every [`BUSY_RETRY_EVERY`] while the helper is busy with a job this window can't see.
    BusyRetry,
    /// The process's connection to the helper is made (`None`: no helper, or no system bus).
    Connected(Link),
    /// The connection to the system bus ended: the job subscription's stream closed.
    BusLost,
    /// Toolbar: Restore (PLAN 6b.8). Asks the helper's `CheckRestore` first.
    RestoreClicked,
    /// `CheckRestore` answered for this snapshot.
    Checked(String, Result<Check, CliError>),
    /// The restore dialog's home radios (0 keep, 1 restore) and safety checkbox.
    RestoreHome(usize),
    RestoreSafety(bool),
    /// The ready prompt's two answers.
    RestartNow,
    CancelRestore,
    RestartAnswered(Result<(), CliError>),
    CancelAnswered(Result<(), CliError>),
    /// After a failed restore: the normal dialog for the same snapshot.
    RestoreAgain,
    /// `RestoreResult` answered (asked when the window opens).
    RestoreResultRead(Result<RestoreResult, CliError>),
    /// The window's height changed.
    WindowResized(f32),
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "io.github.atraxsrc.Apsis";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let config = cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
            .map(|context| match Config::get_entry(&context) {
                Ok(config) | Err((_, config)) => config,
            })
            .unwrap_or_default();
        let mut app = AppModel::new(core, flags.mode, config);
        let task = match flags.mode {
            // The connection first; [`Message::Connected`] then lists for the reminder, but
            // only through the helper (no password). `loading` keeps the popup from starting
            // a second list meanwhile.
            Mode::Applet => {
                app.loading = true;
                connect()
            }
            Mode::Window => {
                let open = app.open_window();
                let page = match flags.view {
                    Some(StartView::Settings) => app.open_settings(),
                    Some(StartView::About) => {
                        app.page = Page::About;
                        Task::none()
                    }
                    None => Task::none(),
                };
                Task::batch([open, page])
            }
        };
        (app, task)
    }

    /// A second `apsis --window` (or `--settings`, `--about`) started while this window is open.
    fn dbus_activation(
        &mut self,
        msg: cosmic::dbus_activation::Message,
    ) -> Task<cosmic::Action<Self::Message>> {
        match msg.msg {
            cosmic::dbus_activation::Details::ActivateAction { action, .. } => {
                self.on_activation(Some(&action))
            }
            _ => self.on_activation(None),
        }
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Self::Message> {
        match self.mode {
            Mode::Window => self.window_view(),
            Mode::Applet => self.panel_button(),
        }
    }

    /// The popup, or the right-click menu.
    fn view_window(&self, id: Id) -> Element<'_, Self::Message> {
        if self.menu == Some(id) {
            return self.menu_view();
        }
        self.core
            .applet
            .popup_container(self.popup_view())
            .limits(
                Limits::NONE
                    .min_width(POPUP_WIDTH)
                    .max_width(POPUP_WIDTH)
                    .min_height(1.0)
                    .max_height(1000.0),
            )
            .into()
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        (self.mode == Mode::Window)
            .then(|| self.dialog_view())
            .flatten()
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        if self.mode != Mode::Window {
            return Vec::new();
        }
        self.header_buttons()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ];
        // Jobs from any window, and the helper leaving the bus, on the process's connection.
        if let Some(client) = &self.helper {
            subscriptions.push(Subscription::run_with(
                JobSource {
                    generation: self.bus_generation,
                    client: client.clone(),
                },
                job_events,
            ));
        }
        if self.mode == Mode::Window {
            subscriptions.push(event::listen_with(shortcut));
            subscriptions.push(event::listen_with(window_resized));
        } else if self.popup.is_some() || self.menu.is_some() {
            subscriptions.push(event::listen_with(escape_only));
        }
        // A job this window can't see (an older helper, or signals that don't arrive).
        if self.helper_busy && self.popup.is_some() && self.running.is_none() {
            subscriptions.push(time::every(BUSY_RETRY_EVERY).map(|_| Message::BusyRetry));
        }
        // The backup disk coming and going.
        if self.watched_uuid().is_some() {
            subscriptions.push(time::every(DISK_CHECK_EVERY).map(|_| Message::DiskCheck));
        }
        // Activation tokens for starting the window.
        if self.mode == Mode::Applet {
            subscriptions.push(activation_token_subscription(0).map(Message::Token));
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
        // The elapsed time while a job has no percent yet.
        if self.popup.is_some() && self.active_job().is_some() {
            subscriptions.push(time::every(TICK_EVERY).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        self.handle(message)
    }

    /// The applet's transparent surfaces; a window keeps the default opaque background.
    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        match self.mode {
            Mode::Applet => Some(cosmic::applet::style()),
            Mode::Window => None,
        }
    }
}

/// The settings' tabs, in order, with their labels.
fn tabs_model() -> segmented_button::SingleSelectModel {
    let mut builder = segmented_button::Model::builder();
    for section in Section::ALL {
        builder = builder.insert(move |b| {
            let b = b.text(section_title(section)).data(section);
            if section == Section::Location {
                b.activate()
            } else {
                b
            }
        });
    }
    builder.build()
}

fn section_title(section: Section) -> String {
    match section {
        Section::Location => fl!("tab-location"),
        Section::Include => fl!("tab-include"),
        Section::Filters => fl!("tab-filters"),
        Section::Misc => fl!("tab-misc"),
    }
}

impl AppModel {
    fn new(core: cosmic::Core, mode: Mode, config: Config) -> Self {
        let icon = symbolic_icon();
        let about = widget::about::About::default()
            .name(fl!("app-title"))
            .icon(icon.clone())
            .version(VERSION)
            .license(LICENSE)
            .links([(fl!("about-source"), REPOSITORY)]);
        AppModel {
            core,
            mode,
            popup: None,
            menu: None,
            config,
            listing: Listing::NotLoaded,
            loading: false,
            known_uuid: None,
            disk_seen: None,
            disk_links: BY_UUID.into(),
            running: None,
            job: None,
            progress: None,
            run_started: None,
            status: None,
            selection: BTreeSet::new(),
            anchor: None,
            modifiers: Modifiers::empty(),
            page: Page::List,
            tabs: tabs_model(),
            settings: SettingsLoad::NotLoaded,
            saving_settings: false,
            dialog: None,
            about,
            window_resizable: false,
            icon,
            helper_found: false,
            helper_busy: false,
            token_requests: None,
            helper: None,
            bus_generation: 0,
            own_end: None,
            own_job_ended: false,
            checking: None,
            ready: None,
            cancelling: false,
            restarting: false,
            restore_result: None,
            window_height: WINDOW_SIZE.height,
        }
    }

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
                let open = self.open_popup();
                return Task::batch([open, self.start_list()]);
            }
            Message::MenuClose => return self.close_menu(),
            Message::MenuPanelSettings => {
                let close = self.close_menu();
                let mut settings = std::process::Command::new("cosmic-settings");
                settings.arg("panel");
                return Task::batch([close, spawn(settings)]);
            }
            Message::OpenWindow(flag) => return self.launch_window(flag),
            Message::OpenUrl(url) => {
                let mut open = std::process::Command::new("xdg-open");
                open.arg(url);
                return spawn(open);
            }
            Message::Token(update) => return self.on_token(update),
            Message::DiskCheck => return self.check_disk(),
            Message::BusyRetry => return self.start_list(),
            Message::WindowShown => return self.make_window_resizable(),
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
                if self.menu == Some(id) {
                    self.menu = None;
                }
            }
            Message::Connected(client) => return self.on_connected(client),
            Message::BusLost => {
                self.helper = None;
                self.bus_generation += 1;
                return connect();
            }
            Message::BackgroundRefresh => {
                if self.popup.is_none() && !self.loading && self.running.is_none() {
                    self.loading = true;
                    return background_list(self.helper.clone());
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
            Message::Shortcut(shortcut) => return self.on_shortcut(shortcut),
            Message::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            Message::Refresh => return self.start_list(),
            Message::Listed(result) => self.on_listed(result),
            Message::RowClicked(index) => self.click_row(index),
            Message::CreateClicked => return self.open_create(),
            Message::DeleteClicked => self.open_delete(),
            Message::RestoreClicked => return self.open_restore(),
            Message::Checked(snapshot, result) => self.on_checked(&snapshot, result),
            Message::RestoreHome(index) => {
                if let Some(Dialog::Restore {
                    check,
                    restore_home,
                    ..
                }) = &mut self.dialog
                {
                    *restore_home = check.has_home && index == 1;
                }
            }
            Message::RestoreSafety(on) => {
                if let Some(Dialog::Restore {
                    safety_snapshot, ..
                }) = &mut self.dialog
                {
                    *safety_snapshot = on;
                }
            }
            Message::RestartNow => return self.restart_now(),
            Message::CancelRestore => return self.cancel_restore(),
            Message::RestartAnswered(result) => self.on_restart_answered(result),
            Message::CancelAnswered(result) => return self.on_cancel_answered(result),
            Message::RestoreAgain => return self.restore_again(),
            Message::RestoreResultRead(result) => {
                if let Ok(result) = result {
                    self.restore_result =
                        matches!(result.state, ResultState::Ended(_)).then_some(result);
                }
            }
            Message::WindowResized(height) => self.window_height = height,
            Message::SettingsClicked => return self.open_settings(),
            Message::AboutClicked => self.page = Page::About,
            Message::Back => return self.leave_page(),
            Message::StopAnswered(Some(error)) => {
                let reason = self.error_text(&error);
                self.status = Some(Status::Error(fl!("stop-failed", reason = reason), None));
            }
            Message::StopAnswered(None) => {}
            Message::Escape if self.menu.is_some() => return self.close_menu(),
            Message::Escape => {
                if let Some(popup) = self.popup.take() {
                    return destroy_popup(popup);
                }
            }
            Message::StopClicked => {
                if let Some(snapshot) = self.stoppable() {
                    let restore = self
                        .job
                        .as_ref()
                        .is_some_and(|j| j.kind == JobKind::Restore);
                    self.dialog = Some(if restore {
                        Dialog::StopRestore { snapshot }
                    } else {
                        Dialog::Stop { snapshot }
                    });
                }
            }
            Message::DialogText(text) => match &mut self.dialog {
                Some(Dialog::Create { comment, error }) => {
                    *comment = text.chars().take(MAX_COMMENT_CHARS).collect();
                    *error = None;
                }
                Some(Dialog::AddPattern { text: typed, error }) => {
                    *typed = text;
                    *error = None;
                }
                _ => {}
            },
            Message::DialogConfirm => return self.confirm_dialog(),
            // Closing the ready prompt is Cancel restore (PLAN 6b.5).
            Message::DialogCancel if matches!(self.dialog, Some(Dialog::Ready { .. })) => {
                return self.cancel_restore();
            }
            Message::DialogCancel => self.dialog = None,
            Message::DialogDiscard => {
                self.dialog = None;
                if let SettingsLoad::Ready(view) = &mut self.settings {
                    view.discard();
                }
                self.page = Page::List;
            }
            Message::Finished(operation, result) => return self.on_finished(&operation, result),
            // A late one, after its operation ended, is dropped.
            Message::Progress(progress) => {
                if self.running.is_some() {
                    self.progress = Some(progress);
                }
            }
            Message::Job(event) => return self.on_job(event),
            Message::JobPolled(job) => {
                if self.running.is_none() {
                    self.job = job.filter(|j| !j.state.is_end());
                }
            }
            Message::Tick => {}
            Message::SettingsRead(result) => self.on_settings_read(result),
            Message::SettingsWritten(result) => return self.on_settings_written(result),
            Message::Tab(entity) => self.tabs.activate(entity),
            Message::PickDevice(index) => {
                if let SettingsLoad::Ready(view) = &mut self.settings
                    && let Some(uuid) = view.candidates().get(index).map(|d| d.uuid.clone())
                    && let Err(reason) = view.pick_device(&uuid)
                {
                    self.status = Some(Status::Error(reason, None));
                }
            }
            Message::Include(row, on) => {
                if let SettingsLoad::Ready(view) = &mut self.settings {
                    view.set_include(row, on);
                }
            }
            Message::FilterClicked(index) => {
                if let SettingsLoad::Ready(view) = &mut self.settings {
                    view.select_row(Row::Filter(index));
                }
            }
            Message::FilterSign(index, include) => {
                if let SettingsLoad::Ready(view) = &mut self.settings {
                    view.set_sign(index, include);
                    view.select_row(Row::Filter(index));
                }
            }
            Message::AddFolder => return pick(true),
            Message::AddFile => return pick(false),
            Message::Picked(picked) => self.on_picked(picked),
            Message::AddPattern => {
                self.dialog = Some(Dialog::AddPattern {
                    text: String::new(),
                    error: None,
                });
            }
            Message::RemoveFilter => self.edit_settings(SettingsView::remove_filter),
            Message::MoveFilter(up) => self.edit_settings(|view| view.move_filter(up)),
            Message::Save => return self.save_settings(),
            Message::RemindDays(days) => {
                let choice = ApsisChoice {
                    remind_days: days,
                    ..self.config.backend()
                };
                return self.set_backend(choice);
            }
            Message::ShowLabel(on) => {
                let choice = ApsisChoice {
                    show_label: on,
                    ..self.config.backend()
                };
                return self.set_backend(choice);
            }
        }
        Task::none()
    }

    /// Window mode: lists at once, and asks the helper what it's doing.
    fn open_window(&mut self) -> Task<cosmic::Action<Message>> {
        self.core.window.show_maximize = false;
        self.core.window.show_minimize = false;
        self.core.set_header_title(fl!("app-title"));
        self.popup = self.core.main_window_id();
        // The connection first; the list and the job poll follow once it's made
        // ([`AppModel::on_connected`]). `loading` holds other lists off meanwhile.
        self.loading = true;
        Task::batch([connect(), self.check_disk()])
    }

    /// The process's connection is made (or there's no helper): what was waiting for it
    /// runs. The window lists and asks what the helper is doing; the applet lists for the
    /// reminder.
    fn on_connected(&mut self, client: Link) -> Task<cosmic::Action<Message>> {
        self.bus_generation += 1;
        self.loading = false;
        let Some(client) = client else {
            self.helper = None;
            return match self.mode {
                Mode::Window => {
                    self.on_listed(Err(CliError::NoHelper));
                    Task::none()
                }
                Mode::Applet => self.handle(Message::BackgroundListed(None)),
            };
        };
        self.helper = Some(client.clone());
        match self.mode {
            Mode::Window => Task::batch([
                self.start_list(),
                poll_job(Some(client.clone())),
                read_restore_result(Some(client)),
            ]),
            Mode::Applet if self.popup.is_some() => {
                Task::batch([self.start_list(), poll_job(Some(client))])
            }
            Mode::Applet => {
                if self.loading || self.running.is_some() {
                    return Task::none();
                }
                self.loading = true;
                background_list(Some(client))
            }
        }
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

    /// Starts `apsis --window` (or `--settings`, `--about`) as its own process, and closes the
    /// popup and the menu: the work happens in the window. If a window is open already, the new
    /// process hands over to it and exits (see [`run_window`]).
    ///
    /// It first asks the compositor for an activation token; the process starts when the token
    /// comes ([`Message::Token`]), so the window may take focus. Without the token channel, at
    /// once.
    fn launch_window(&mut self, flag: Option<&'static str>) -> Task<cosmic::Action<Message>> {
        let flag = flag.unwrap_or("--window");
        let asked = self.token_requests.as_ref().is_some_and(|requests| {
            requests
                .send(TokenRequest {
                    app_id: <Self as cosmic::Application>::APP_ID.to_owned(),
                    exec: flag.to_owned(),
                })
                .is_ok()
        });
        let start = if asked {
            Task::none()
        } else {
            spawn(window_command(std::env::current_exe().ok(), flag, None))
        };
        let close_menu = self.close_menu();
        let close_popup = match self.popup.take() {
            Some(popup) if self.mode == Mode::Applet => destroy_popup(popup),
            _ => Task::none(),
        };
        Task::batch([close_menu, close_popup, start])
    }

    /// The token channel is ready or gone, or a token came for [`AppModel::launch_window`]:
    /// the window starts with it (or without, if the compositor gave none).
    fn on_token(&mut self, update: TokenUpdate) -> Task<cosmic::Action<Message>> {
        match update {
            TokenUpdate::Init(requests) => self.token_requests = Some(requests),
            TokenUpdate::Finished => self.token_requests = None,
            TokenUpdate::ActivationToken { token, exec } => {
                let command = window_command(std::env::current_exe().ok(), &exec, token);
                return spawn(command);
            }
        }
        Task::none()
    }

    /// Another `apsis --window` handed over to this window: libcosmic has already brought it
    /// forward (with the starter's activation token). `--settings` and `--about` switch to
    /// that page, but only when nothing is in progress here: no job this window started, no
    /// dialog, no unsaved settings.
    fn on_activation(&mut self, action: Option<&str>) -> Task<cosmic::Action<Message>> {
        let Some(view) = action.and_then(StartView::from_action) else {
            return Task::none();
        };
        let unsaved = matches!(&self.settings, SettingsLoad::Ready(v) if v.dirty())
            && self.page == Page::Settings;
        if self.running.is_some() || self.dialog.is_some() || unsaved {
            return Task::none();
        }
        match view {
            StartView::Settings => self.open_settings(),
            StartView::About => {
                self.page = Page::About;
                Task::none()
            }
        }
    }

    fn toggle_popup(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(popup) = self.popup.take() {
            return destroy_popup(popup);
        }
        self.open_popup()
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
            Some(popup) => destroy_popup(popup),
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

    /// Opens the popup, closing the menu first. Lists if nothing is listed yet, and asks the
    /// helper what it's doing (only if it runs).
    fn open_popup(&mut self) -> Task<cosmic::Action<Message>> {
        let close = self.close_menu();
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
        let open = close.chain(get_popup(settings));
        let open = Task::batch([open, self.check_disk(), poll_job(self.helper.clone())]);
        if matches!(self.listing, Listing::NotLoaded) {
            Task::batch([open, self.start_list()])
        } else {
            open
        }
    }

    /// Lists in the background, through `apsis-helper`. Does nothing while a list, create or
    /// delete this window started is running (the helper runs one at a time).
    fn start_list(&mut self) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        self.loading = true;
        let link = self.helper.clone();
        cosmic::task::future(async move { Message::Listed(list_snapshots(link).await) })
    }

    fn on_listed(&mut self, result: Result<SnapshotList, CliError>) {
        self.loading = false;
        if matches!(result, Err(CliError::Busy)) {
            // Not a failed list: what was shown stays, and it's tried again when the job ends.
            self.helper_busy = true;
            return;
        }
        self.helper_busy = false;
        match result {
            Ok(mut list) => {
                list.snapshots.sort_by_key(|s| std::cmp::Reverse(s.created));
                self.known_uuid.clone_from(&list.uuid);
                // It listed, so the disk is there (or none is set).
                self.disk_seen = list.uuid.is_some().then_some(true);
                // Rows that are gone leave the selection.
                self.selection.retain(|name| {
                    list.snapshots.iter().any(|s| &s.name == name) || list.leftovers.contains(name)
                });
                self.anchor = None;
                self.listing = Listing::Loaded(list);
            }
            Err(error) => {
                if matches!(error, CliError::DeviceNotFound { .. }) {
                    // Plugging it in lists again by itself.
                    self.disk_seen = Some(false);
                }
                self.listing = Listing::Failed(error);
                self.selection.clear();
                self.anchor = None;
            }
        }
    }

    /// Whether Create can start: a list showed a backup device, it's still connected, and no
    /// job runs.
    fn can_create(&self) -> bool {
        let has_device =
            matches!(&self.listing, Listing::Loaded(list) if list.snapshot_device().is_some());
        has_device
            && !self.disk_gone()
            && !self.loading
            && self.running.is_none()
            && self.active_job().is_none()
            && !self.saving_settings
            && self.checking.is_none()
            && self.ready.is_none()
            && !self.cancelling
            && !self.restarting
    }

    /// Whether Restore can start (PLAN 6b.8): as for create, with exactly one snapshot (not a
    /// leftover) selected.
    fn can_restore(&self) -> bool {
        let selected = self.selected_names();
        self.can_create()
            && selected.len() == 1
            && self.snapshots().iter().any(|s| s.name == selected[0])
    }

    /// Restore: asks the helper what it would say about the snapshot, then opens the dialog.
    fn open_restore(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.can_restore() {
            return Task::none();
        }
        let snapshot = self.selected_names().remove(0);
        self.checking = Some(snapshot.clone());
        self.status = None;
        let link = self.helper.clone();
        cosmic::task::future(async move {
            let result = check_restore(link, &snapshot).await;
            Message::Checked(snapshot, result)
        })
    }

    /// `CheckRestore` answered: the restore dialog with its defaults (home kept, the safety
    /// snapshot on), the "Can't restore" dialog, or why the helper couldn't be asked.
    fn on_checked(&mut self, snapshot: &str, result: Result<Check, CliError>) {
        if self.checking.as_deref() != Some(snapshot) {
            return;
        }
        self.checking = None;
        match result {
            Ok(check) => {
                self.dialog = Some(match check.refusal.clone() {
                    Some(refusal) => Dialog::Refused {
                        snapshot: snapshot.to_owned(),
                        refusal,
                        dropped: false,
                    },
                    None => Dialog::Restore {
                        snapshot: snapshot.to_owned(),
                        check,
                        restore_home: false,
                        safety_snapshot: true,
                    },
                });
            }
            Err(error) => {
                let reason = self.error_text(&error);
                self.status = Some(Status::Error(fl!("restore-failed", reason = reason), None));
            }
        }
    }

    /// "Restart now": the helper re-checks, arms and restarts (PLAN 6b.5).
    fn restart_now(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(Dialog::Ready { snapshot }) = self.dialog.take() else {
            return Task::none();
        };
        self.restarting = true;
        let link = self.helper.clone();
        cosmic::task::future(async move {
            Message::RestartAnswered(restart_to_restore(link, &snapshot).await)
        })
    }

    fn on_restart_answered(&mut self, result: Result<(), CliError>) {
        self.restarting = false;
        let snapshot = self.ready.take().unwrap_or_default();
        self.status = Some(match result {
            Ok(()) => Status::Info(fl!("restarting")),
            Err(CliError::RestoreRefused(refusal)) => {
                // The plan was dropped with the refusal (owner, 2026-10-02), and the helper
                // ended its job `stopped`: the line under the dialog says that, not `ready`
                // (check 4, 2026-10-03).
                self.dialog = Some(Dialog::Refused {
                    snapshot,
                    refusal,
                    dropped: true,
                });
                Status::Info(fl!("restore-stopped"))
            }
            Err(error) => {
                let fallback = self.error_text(&error);
                Status::Error(plan_error_text(&error, &fallback), None)
            }
        });
    }

    /// "Cancel restore", Esc or closing the prompt: the plan goes (a finished safety snapshot
    /// stays).
    fn cancel_restore(&mut self) -> Task<cosmic::Action<Message>> {
        if self.ready.is_none() || self.cancelling {
            self.dialog = None;
            return Task::none();
        }
        self.dialog = None;
        self.cancelling = true;
        let link = self.helper.clone();
        cosmic::task::future(async move { Message::CancelAnswered(cancel_restore(link).await) })
    }

    fn on_cancel_answered(
        &mut self,
        result: Result<(), CliError>,
    ) -> Task<cosmic::Action<Message>> {
        self.cancelling = false;
        self.ready = None;
        self.status = Some(match result {
            Ok(()) => Status::Info(fl!("restore-cancelled")),
            Err(error) => {
                let fallback = self.error_text(&error);
                Status::Error(plan_error_text(&error, &fallback), None)
            }
        });
        // The safety snapshot stays and is in the list.
        self.start_list()
    }

    /// After a failed restore: the normal dialog for the same snapshot (PLAN 6b.10).
    fn restore_again(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(name) = self
            .restore_result
            .as_ref()
            .and_then(|r| r.snapshot.clone())
        else {
            return Task::none();
        };
        if !self.snapshots().iter().any(|s| s.name == name) {
            return Task::none();
        }
        self.selection.clear();
        self.selection.insert(name);
        self.open_restore()
    }

    /// Whether the status line offers "Restore again" (a failed restore whose snapshot is
    /// still listed).
    fn restore_again_available(&self) -> bool {
        self.restore_result
            .as_ref()
            .filter(|r| r.state == ResultState::Ended(Outcome::Failed))
            .and_then(|r| r.snapshot.as_deref())
            .is_some_and(|name| self.snapshots().iter().any(|s| s.name == name))
    }

    /// What to do after a restore that stopped, from the helper's message.
    fn result_what(message: &str) -> String {
        if message.contains("backup disk") {
            fl!("result-what-disk")
        } else if message.contains("space") {
            fl!("result-what-space")
        } else {
            fl!("result-what-readme")
        }
    }

    /// The last restore's line as a coloured phrase between two plain parts; `None` for no
    /// result and for `done` and `not-started`, whose line is one plain text.
    fn result_phrase(&self) -> Option<ResultPhrase> {
        let result = self.restore_result.as_ref()?;
        let ResultState::Ended(outcome) = result.state else {
            return None;
        };
        let (before, phrase, after, tone) = match outcome {
            Outcome::Done | Outcome::NotStarted => return None,
            // The helper names the log when rsync couldn't write or delete files; any other
            // problem is a cleanup after a restore that worked (the old kernel's removal).
            Outcome::Problems if result.message.contains("rsync-log") => (
                fl!("result-problems-before"),
                fl!("result-problems-phrase"),
                fl!("result-problems-after"),
                Tone::Warning,
            ),
            Outcome::Problems => (
                fl!("result-problems-before"),
                fl!("result-problems-other-phrase"),
                fl!("result-problems-other-after"),
                Tone::Warning,
            ),
            Outcome::BootKept => (
                fl!("result-boot-kept-before"),
                fl!("result-boot-kept-phrase"),
                fl!("result-boot-kept-after"),
                Tone::Warning,
            ),
            Outcome::BootBroken => (
                fl!("result-boot-broken-before"),
                fl!("result-boot-broken-phrase"),
                fl!("result-boot-broken-after"),
                Tone::Error,
            ),
            Outcome::Failed => (
                fl!("result-failed-before"),
                fl!("result-failed-phrase"),
                fl!(
                    "result-failed-after",
                    what = Self::result_what(&result.message)
                ),
                Tone::Error,
            ),
        };
        Some(ResultPhrase {
            before,
            phrase,
            after,
            tone,
        })
    }

    /// The last restore's line for the status area (PLAN 6b.8, "After login") and its tooltip.
    fn result_text(&self) -> Option<(String, String)> {
        let result = self.restore_result.as_ref()?;
        let ResultState::Ended(outcome) = result.state else {
            return None;
        };
        let date = result.snapshot.as_deref().map(date_of).unwrap_or_default();
        let name = result.snapshot.clone().unwrap_or_default();
        // From `result.json` (fix 6, 2026-10-04). A result always says what happened to
        // home (`RestoreResult::from_wire`); without it the tooltip says nothing of home.
        let home = match result.home {
            Some(Home::Keep) => fl!("result-home-kept"),
            Some(Home::Restore) => fl!("result-home-restored"),
            None => String::new(),
        };
        let safety = result
            .safety_snapshot
            .clone()
            .unwrap_or_else(|| fl!("result-safety-none"));
        let reason = result.message.as_str();
        let line = match self.result_phrase() {
            Some(parts) => format!("{} {} {}", parts.before, parts.phrase, parts.after),
            None if outcome == Outcome::Done => fl!("result-done", date = date.as_str()),
            None => fl!(
                "result-not-started",
                what = Self::result_what(&result.message)
            ),
        };
        let tooltip = match outcome {
            Outcome::Done => fl!("result-done-tip", name = name, home = home, safety = safety),
            Outcome::Problems => fl!(
                "result-problems-tip",
                name = name,
                home = home,
                safety = safety,
                reason = reason
            ),
            Outcome::BootKept => fl!("result-boot-kept-tip", date = date.as_str()),
            Outcome::BootBroken => fl!(
                "result-boot-broken-tip",
                date = date.as_str(),
                reason = reason
            ),
            Outcome::Failed if result.safety_snapshot.is_some() => {
                fl!("result-failed-tip", reason = reason)
            }
            Outcome::Failed => fl!("result-failed-tip-no-safety", reason = reason),
            Outcome::NotStarted => result.message.clone(),
        };
        Some((line, tooltip))
    }

    /// `Preparing restore · …` and how far (PLAN 6b.8): this window's preparation, its ready
    /// plan, or another window's restore job.
    fn restore_progress(&self) -> (String, Option<f32>) {
        if self.ready.is_some() {
            return (fl!("preparing-ready"), Some(1.0));
        }
        if self.running.is_none() {
            // Another window's: at 100% it waits at its prompt.
            let at_prompt = self
                .job
                .as_ref()
                .is_some_and(|j| j.percent.is_some_and(|p| p >= 100.0));
            if at_prompt {
                return (fl!("restore-ready-elsewhere"), Some(1.0));
            }
        }
        match &self.progress {
            Some(progress) if progress.has_estimate() => {
                let percent = progress.percent.unwrap_or(0.0);
                let text = match progress.eta_seconds {
                    Some(eta) => fl!(
                        "preparing-safety",
                        percent = fmt::percent(percent),
                        time = fmt::duration(eta)
                    ),
                    None => fl!("preparing-safety-percent", percent = fmt::percent(percent)),
                };
                #[allow(clippy::cast_possible_truncation, reason = "0 to 1")]
                (text, Some((percent / 100.0) as f32))
            }
            _ => (fl!("preparing-checking"), None),
        }
    }

    /// Whether Delete can start: as for create, plus something is selected.
    fn can_delete(&self) -> bool {
        self.can_create() && !self.selection.is_empty()
    }

    /// The backup disk to watch: the last good list's, else the one a failed list missed.
    fn watched_uuid(&self) -> Option<&str> {
        match &self.listing {
            Listing::Failed(CliError::DeviceNotFound { device }) => Some(device),
            _ => self.known_uuid.as_deref(),
        }
    }

    /// The backup disk left since the last good list (its UUID link is gone).
    fn disk_gone(&self) -> bool {
        matches!(self.listing, Listing::Loaded(_)) && self.disk_seen == Some(false)
    }

    /// Looks for the backup disk's `/dev/disk/by-uuid` link: gone, the status area and tooltip
    /// say it's not connected (the snapshots stay as listed); back, it lists again. Only the
    /// change from gone to back lists, so a disk the helper can't use isn't mounted every few
    /// seconds.
    fn check_disk(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(present) = self
            .watched_uuid()
            .and_then(|uuid| disk_connected(&self.disk_links, uuid))
        else {
            return Task::none();
        };
        let was = self.disk_seen.replace(present);
        if !present || was != Some(false) {
            return Task::none();
        }
        match self.mode {
            Mode::Window => self.start_list(),
            Mode::Applet if self.popup.is_some() => self.start_list(),
            // The panel's own list, through the helper only (no password), as the reminder's.
            Mode::Applet if !self.loading && self.running.is_none() => {
                self.loading = true;
                background_list(self.helper.clone())
            }
            Mode::Applet => Task::none(),
        }
    }

    /// The rows the list shows, top to bottom: snapshots (newest first), then leftovers.
    fn rows(&self) -> Vec<RowItem<'_>> {
        let Listing::Loaded(list) = &self.listing else {
            return Vec::new();
        };
        list.snapshots
            .iter()
            .map(RowItem::Snapshot)
            .chain(list.leftovers.iter().map(|n| RowItem::Leftover(n)))
            .collect()
    }

    /// A click on row `index`: Ctrl adds or removes it, Shift selects the range from the last
    /// click, a plain click selects only it.
    fn click_row(&mut self, index: usize) {
        let rows = self.rows();
        let Some(name) = rows.get(index).map(RowItem::name) else {
            return;
        };
        let name = name.to_owned();
        if self.modifiers.shift()
            && let Some(anchor) = self.anchor
        {
            let (from, to) = (anchor.min(index), anchor.max(index));
            let names: Vec<String> = rows[from..=to.min(rows.len() - 1)]
                .iter()
                .map(|r| r.name().to_owned())
                .collect();
            if !self.modifiers.control() {
                self.selection.clear();
            }
            self.selection.extend(names);
            return;
        }
        if self.modifiers.control() {
            if !self.selection.remove(&name) {
                self.selection.insert(name);
            }
        } else {
            self.selection.clear();
            self.selection.insert(name);
        }
        self.anchor = Some(index);
    }

    /// Up or Down: moves a single selection.
    fn move_selection(&mut self, down: bool) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let current = self
            .anchor
            .or_else(|| rows.iter().position(|r| self.selection.contains(r.name())));
        let index = match (current, down) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(rows.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        let name = rows[index].name().to_owned();
        self.selection.clear();
        self.selection.insert(name);
        self.anchor = Some(index);
    }

    /// The selected names, in list order.
    fn selected_names(&self) -> Vec<String> {
        self.rows()
            .iter()
            .map(RowItem::name)
            .filter(|n| self.selection.contains(*n))
            .map(str::to_owned)
            .collect()
    }

    fn on_shortcut(&mut self, shortcut: Shortcut) -> Task<cosmic::Action<Message>> {
        match shortcut {
            Shortcut::Escape if matches!(self.dialog, Some(Dialog::Ready { .. })) => {
                self.cancel_restore()
            }
            Shortcut::Escape => {
                if self.dialog.take().is_some() {
                    return Task::none();
                }
                if self.page != Page::List {
                    return self.leave_page();
                }
                self.selection.clear();
                self.anchor = None;
                Task::none()
            }
            // The rest act on the list only, with no dialog open.
            _ if self.dialog.is_some() || self.page != Page::List => Task::none(),
            Shortcut::Create => self.open_create(),
            Shortcut::Delete => {
                self.open_delete();
                Task::none()
            }
            Shortcut::Refresh => self.start_list(),
            Shortcut::Settings => self.open_settings(),
            Shortcut::SelectAll => {
                self.selection = self.rows().iter().map(|r| r.name().to_owned()).collect();
                Task::none()
            }
            Shortcut::Up | Shortcut::Down => {
                self.move_selection(shortcut == Shortcut::Down);
                Task::none()
            }
        }
    }

    /// The Create dialog, with what a snapshot includes (the config is read for it).
    fn open_create(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.can_create() {
            return Task::none();
        }
        self.dialog = Some(Dialog::Create {
            comment: String::new(),
            error: None,
        });
        match &self.settings {
            SettingsLoad::Ready(view) if view.dirty() => Task::none(),
            _ => self.load_settings(),
        }
    }

    fn open_delete(&mut self) {
        if self.can_delete() {
            self.dialog = Some(Dialog::Delete {
                names: self.selected_names(),
            });
        }
    }

    /// The running create's snapshot, if it can be stopped now (its name is known).
    fn stoppable(&self) -> Option<String> {
        self.job
            .as_ref()
            .filter(|j| {
                matches!(j.kind, JobKind::Create | JobKind::Restore)
                    && j.state == JobState::Running
                    && !j.snapshot.is_empty()
                    && self.ready.is_none()
            })
            .map(|j| j.snapshot.clone())
    }

    /// The job to show in the status area: this window's, or another's create, delete or
    /// restore (a ready plan elsewhere holds the toolbar too, PLAN 6b.9).
    fn active_job(&self) -> Option<&Job> {
        self.job
            .as_ref()
            .filter(|j| j.kind != JobKind::Configure && !j.state.is_end())
    }

    /// OK in the dialog.
    fn confirm_dialog(&mut self) -> Task<cosmic::Action<Message>> {
        match self.dialog.take() {
            Some(Dialog::Create { comment, .. }) => {
                // Checked here too, so a bad comment can be fixed before the password prompt.
                if let Err(error) = validate_comment(&comment) {
                    self.dialog = Some(Dialog::Create {
                        comment,
                        error: Some(error.to_string()),
                    });
                    return Task::none();
                }
                self.run(Operation::Create(comment))
            }
            Some(Dialog::Delete { mut names }) => match names.len() {
                0 => Task::none(),
                1 => self.run(Operation::Delete(names.remove(0))),
                _ => self.run(Operation::DeleteMany(names)),
            },
            Some(Dialog::Stop { snapshot }) => {
                self.status = Some(Status::Info(fl!("stopping")));
                let link = self.helper.clone();
                cosmic::task::future(async move {
                    Message::StopAnswered(stop(link, &snapshot).await.err())
                })
            }
            Some(Dialog::AddPattern { text, .. }) => {
                let SettingsLoad::Ready(view) = &mut self.settings else {
                    return Task::none();
                };
                if let Err(reason) = view.add_filter(&typed_filter(&text)) {
                    self.dialog = Some(Dialog::AddPattern {
                        text,
                        error: Some(reason),
                    });
                }
                Task::none()
            }
            Some(Dialog::Unsaved) => {
                // Save, then leave.
                let save = self.save_settings();
                if self.saving_settings {
                    self.page = Page::List;
                }
                save
            }
            Some(Dialog::Restore {
                snapshot,
                restore_home,
                safety_snapshot,
                ..
            }) => self.run(Operation::Restore {
                snapshot,
                restore_home,
                safety_snapshot,
            }),
            Some(Dialog::Refused { .. }) => Task::none(),
            Some(Dialog::StopRestore { snapshot }) => {
                self.status = Some(Status::Info(fl!("stopping")));
                let link = self.helper.clone();
                cosmic::task::future(async move {
                    Message::StopAnswered(stop(link, &snapshot).await.err())
                })
            }
            Some(dialog @ Dialog::Ready { .. }) => {
                self.dialog = Some(dialog);
                self.restart_now()
            }
            None => Task::none(),
        }
    }

    /// Creates or deletes in the background, through `apsis-helper`.
    fn run(&mut self, operation: Operation) -> Task<cosmic::Action<Message>> {
        if self.loading || self.running.is_some() || self.saving_settings {
            return Task::none();
        }
        self.running = Some(operation.clone());
        self.status = None;
        self.progress = None;
        self.run_started = Some(Instant::now());
        self.own_job_ended = false;
        self.own_end = None;
        let link = self.helper.clone();
        with_progress(move |mut progress| async move {
            let result = operate(link, operation.clone(), &mut progress).await;
            Message::Finished(operation, result)
        })
    }

    /// Shows how this window's job went and lists again if anything may have changed (not
    /// when polkit refused, or the helper or the disk isn't there).
    fn on_finished(
        &mut self,
        operation: &Operation,
        result: Result<(), CliError>,
    ) -> Task<cosmic::Action<Message>> {
        self.running = None;
        self.progress = None;
        self.run_started = None;
        self.own_end = self.expect_own_end(operation, &result);
        if matches!(result, Err(CliError::Busy)) {
            self.helper_busy = true;
            self.status = Some(Status::Info(fl!("busy-background")));
            return Task::none();
        }
        if let Operation::Restore { snapshot, .. } = operation {
            return self.on_restore_finished(snapshot, result);
        }
        let ran = match &result {
            // A failed create or delete may have left something (a staging folder, a partly
            // deleted snapshot); a stopped one leaves nothing, but the disk has changed.
            Ok(()) | Err(CliError::Other(_) | CliError::Stopped) => true,
            // Whatever was deleted before it stopped changed the list.
            Err(CliError::DeleteManyStopped {
                deleted, reason, ..
            }) => !deleted.is_empty() || matches!(**reason, CliError::Other(_)),
            // Nothing ran, or (disk missing) a list would only fail again.
            Err(
                CliError::NoHelper
                | CliError::NotAuthorized
                | CliError::DeviceNotFound { .. }
                | CliError::DiskRemoved { .. }
                | CliError::Busy
                | CliError::RestoreRefused(_)
                | CliError::RestoreArmed,
            ) => false,
        };
        self.status = Some(match (operation, result) {
            (Operation::Create(_), Ok(())) => Status::Info(fl!("created")),
            (Operation::Delete(name), Ok(())) => {
                self.selection.remove(name);
                Status::Info(fl!("deleted", name = self.snapshot_label(name)))
            }
            (Operation::DeleteMany(names), Ok(())) => {
                for name in names {
                    self.selection.remove(name);
                }
                Status::Info(fl!("deleted-many", count = names.len().to_string()))
            }
            (Operation::Create(_), Err(CliError::Stopped)) => Status::Info(fl!("create-stopped")),
            (op, Err(CliError::DiskRemoved { reason })) => {
                // The helper saw the disk's link go; so will the next look here.
                self.disk_seen = Some(false);
                let line = match op {
                    Operation::Create(_) => fl!("create-failed-disk-removed"),
                    Operation::Delete(_) | Operation::DeleteMany(_) => {
                        fl!("delete-failed-disk-removed")
                    }
                    Operation::Restore { .. } => unreachable!("handled above"),
                };
                Status::Error(line, Some(reason))
            }
            (Operation::Create(_), Err(error)) => {
                Status::Error(fl!("create-failed", reason = self.error_text(&error)), None)
            }
            (
                Operation::DeleteMany(_),
                Err(CliError::DeleteManyStopped {
                    deleted,
                    failed,
                    left,
                    reason,
                }),
            ) => {
                // The helper's account of what went: those leave the selection, the rest stay.
                for name in &deleted {
                    self.selection.remove(name);
                }
                if matches!(*reason, CliError::DiskRemoved { .. }) {
                    self.disk_seen = Some(false);
                }
                let labels = |names: &[String]| -> Vec<String> {
                    names.iter().map(|n| self.snapshot_label(n)).collect()
                };
                let mut not_deleted = vec![failed.clone()];
                not_deleted.extend(left);
                Status::Error(
                    fl!(
                        "delete-many-stopped",
                        name = self.snapshot_label(&failed),
                        reason = self.error_text(&reason)
                    ),
                    Some(bulk_delete_details(
                        &labels(&deleted),
                        &labels(&not_deleted),
                    )),
                )
            }
            // Refused while a restore is armed: nothing was deleted, of one or of several.
            (Operation::Delete(_) | Operation::DeleteMany(_), Err(CliError::RestoreArmed)) => {
                Status::Error(
                    fl!("delete-refused-armed"),
                    Some(fl!("delete-refused-armed-tip")),
                )
            }
            // A delete refused before anything ran, or one that failed.
            (Operation::Delete(_) | Operation::DeleteMany(_), Err(error)) => {
                Status::Error(fl!("delete-failed", reason = self.error_text(&error)), None)
            }
            (Operation::Restore { .. }, _) => unreachable!("handled above"),
        });
        if ran {
            return self.start_list();
        }
        Task::none()
    }

    /// This window's restore preparation ended: ready at the prompt, refused, stopped or
    /// failed (PLAN 6b.5, 6b.8). The list is refreshed for the safety snapshot, which a
    /// ready plan doesn't block (reads go through).
    fn on_restore_finished(
        &mut self,
        snapshot: &str,
        result: Result<(), CliError>,
    ) -> Task<cosmic::Action<Message>> {
        match result {
            Ok(()) => {
                self.ready = Some(snapshot.to_owned());
                self.dialog = Some(Dialog::Ready {
                    snapshot: snapshot.to_owned(),
                });
                self.status = Some(Status::Info(fl!("preparing-ready")));
            }
            Err(CliError::RestoreRefused(refusal)) => {
                self.dialog = Some(Dialog::Refused {
                    snapshot: snapshot.to_owned(),
                    refusal,
                    dropped: false,
                });
                return Task::none();
            }
            Err(CliError::Stopped) => {
                self.status = Some(Status::Info(fl!("restore-stopped")));
            }
            Err(CliError::Busy) => {
                self.helper_busy = true;
                self.status = Some(Status::Info(fl!("busy-background")));
                return Task::none();
            }
            Err(error @ (CliError::NoHelper | CliError::NotAuthorized)) => {
                let reason = self.error_text(&error);
                self.status = Some(Status::Error(fl!("restore-failed", reason = reason), None));
                return Task::none();
            }
            Err(error) => {
                let reason = self.error_text(&error);
                self.status = Some(Status::Error(fl!("restore-failed", reason = reason), None));
            }
        }
        self.start_list()
    }

    /// What this window's finished `operation` still has coming from the helper: the end
    /// announcement of its job, unless that already arrived (then `job` is `None`) or no job
    /// began (refused before the lock: no helper, polkit, busy, a restore armed).
    fn expect_own_end(
        &mut self,
        operation: &Operation,
        result: &Result<(), CliError>,
    ) -> Option<OwnEnd> {
        let kind = operation.kind();
        let finished_at = Instant::now();
        if std::mem::take(&mut self.own_job_ended) {
            return None;
        }
        match self.job.take() {
            // Seen running and not yet ended: its end is still to come.
            Some(job) if job.kind == kind && !job.state.is_end() => Some(OwnEnd {
                kind,
                started: Some(job.started),
                finished_at,
            }),
            // Another job's (stale): keep it where it was.
            Some(job) => {
                self.job = Some(job);
                None
            }
            None => match result {
                Err(
                    CliError::NoHelper
                    | CliError::NotAuthorized
                    | CliError::Busy
                    | CliError::RestoreArmed,
                ) => None,
                // A restore refused as armed is refused before the lock. (The helper's guard
                // under the lock gives the same refusal with a job, in the seconds while
                // another window's arm is being made; if `Finished` beats that job's every
                // announcement, its end is taken for another window's: a list, no line.)
                Err(CliError::RestoreRefused(Refusal::RestoreArmed)) => None,
                // `Finished` beat even the job's first announcement: by kind, briefly.
                _ => Some(OwnEnd {
                    kind,
                    started: None,
                    finished_at,
                }),
            },
        }
    }

    /// A job changed in the helper (any window's), or the helper left the bus.
    fn on_job(&mut self, event: JobEvent) -> Task<cosmic::Action<Message>> {
        let job = match event {
            JobEvent::Changed(job) => job,
            JobEvent::HelperGone => {
                self.own_end = None;
                let had = self.job.take();
                // This window's own job ends with its `Finished` (or the helper-gone error).
                if self.running.is_none() && had.is_some_and(|j| j.kind.changes_the_list()) {
                    return self.start_list();
                }
                return Task::none();
            }
        };
        // This window's own job, announced after its `Finished` already came: its start and
        // progress are stale, and its end was shown by `on_finished`, which also listed.
        if let Some(own) = &self.own_end {
            if own.is(&job) {
                if job.state.is_end() {
                    self.own_end = None;
                    self.job = None;
                }
                return Task::none();
            }
            // Something else's: the own end was lost, or this isn't it.
            self.own_end = None;
        }
        if !job.state.is_end() {
            // Progress of another window's create or restore; this window's comes with its
            // own call too.
            if self.running.is_none()
                && matches!(job.kind, JobKind::Create | JobKind::Restore)
                && job.percent.is_some()
            {
                self.progress = Some(Progress {
                    percent: job.percent,
                    eta_seconds: job.eta_seconds,
                    text: String::new(),
                });
            }
            if job.state == JobState::Stopping {
                self.status = Some(Status::Info(fl!("stopping")));
            }
            self.job = Some(job);
            return Task::none();
        }
        // It ended.
        let changed = job.kind.changes_the_list();
        self.job = None;
        if self.running.is_some() {
            // This window's own: its `Finished` says how it went and lists.
            self.own_job_ended = true;
            return Task::none();
        }
        self.progress = None;
        // Another window's restore ended (cancelled, restarted, or failed): its safety
        // snapshot may be in the list; that window says how it went.
        if job.kind == JobKind::Restore {
            if self.ready.is_none() {
                return self.start_list();
            }
            return Task::none();
        }
        if changed {
            self.status = Some(match (job.kind, job.state) {
                (JobKind::Create, JobState::Done) => Status::Info(fl!("created")),
                (JobKind::Create, JobState::Stopped) => Status::Info(fl!("create-stopped")),
                (JobKind::Create, _) => Status::Error(fl!("create-failed-elsewhere"), None),
                (_, JobState::Done) => Status::Info(fl!("deleted-elsewhere")),
                _ => Status::Error(fl!("delete-failed-elsewhere"), None),
            });
        }
        // One list per process per job end, and only for a create or delete: a config write
        // doesn't change the list, and a list is never a job. A Busy from a write that was
        // running or waiting is retried by the [`BUSY_RETRY_EVERY`] timer.
        if changed {
            return self.start_list();
        }
        Task::none()
    }

    fn open_settings(&mut self) -> Task<cosmic::Action<Message>> {
        self.page = Page::Settings;
        match &self.settings {
            SettingsLoad::Ready(view) if view.dirty() => Task::none(),
            _ => self.load_settings(),
        }
    }

    /// Back from the settings (asking about unsaved changes first) or About.
    fn leave_page(&mut self) -> Task<cosmic::Action<Message>> {
        if self.page == Page::Settings
            && let SettingsLoad::Ready(view) = &self.settings
            && view.dirty()
            && !view.imported()
        {
            self.dialog = Some(Dialog::Unsaved);
            return Task::none();
        }
        self.page = Page::List;
        Task::none()
    }

    /// Reads Apsis's config through the helper, dropping unsaved changes.
    fn load_settings(&mut self) -> Task<cosmic::Action<Message>> {
        if self.saving_settings || matches!(self.settings, SettingsLoad::Loading) {
            return Task::none();
        }
        self.settings = SettingsLoad::Loading;
        let link = self.helper.clone();
        cosmic::task::future(async move { Message::SettingsRead(read_config(link).await) })
    }

    fn on_settings_read(&mut self, result: Result<ConfigInfo, CliError>) {
        self.settings = match result {
            Ok(info) => {
                SettingsLoad::Ready(Box::new(SettingsView::new(info, self.config.backend())))
            }
            Err(error) => SettingsLoad::Failed(self.error_text(&error)),
        };
    }

    /// Changes the settings with `edit`, showing why if it can't.
    fn edit_settings(&mut self, edit: impl FnOnce(&mut SettingsView) -> Result<(), String>) {
        if let SettingsLoad::Ready(view) = &mut self.settings
            && let Err(reason) = edit(view)
        {
            self.status = Some(Status::Error(reason, None));
        }
    }

    /// A path from the file chooser becomes a filter at the top.
    fn on_picked(&mut self, picked: Option<(String, bool)>) {
        let Some((path, folder)) = picked else {
            return;
        };
        self.edit_settings(|view| {
            let filter = picked_filter(&path, folder, &view.edited)?;
            view.add_filter(&filter)
        });
    }

    /// Save: checks the edits here first (so a mistake shows before the password dialog), then
    /// has the helper write them.
    fn save_settings(&mut self) -> Task<cosmic::Action<Message>> {
        if self.saving_settings || self.loading || self.running.is_some() {
            return Task::none();
        }
        let SettingsLoad::Ready(view) = &self.settings else {
            return Task::none();
        };
        if !view.dirty() {
            return Task::none();
        }
        if let Err(reason) = view.validate() {
            self.status = Some(Status::Error(reason, None));
            return Task::none();
        }
        let (expected, config) = (view.info.text.clone(), view.edited.clone());
        self.saving_settings = true;
        self.status = Some(Status::Info(fl!("settings-saving")));
        let link = self.helper.clone();
        cosmic::task::future(async move {
            Message::SettingsWritten(write_config(link, expected, config).await)
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
            Ok(note) => Status::Error(fl!("settings-saved-note", note = note), None),
            Err(error) => Status::Error(
                fl!("settings-failed", reason = self.error_text(&error)),
                None,
            ),
        });
        if written {
            self.settings = SettingsLoad::NotLoaded;
            return Task::batch([self.load_settings(), self.start_list()]);
        }
        Task::none()
    }

    /// Saves Apsis's own settings (the Misc tab) to cosmic-config, at once.
    fn set_backend(&mut self, choice: ApsisChoice) -> Task<cosmic::Action<Message>> {
        let saved =
            cosmic_config::Config::new(<Self as cosmic::Application>::APP_ID, Config::VERSION)
                .and_then(|context| {
                    self.config.set_remind_days(&context, choice.remind_days)?;
                    self.config.set_show_label(&context, choice.show_label)
                });
        if let Err(error) = saved {
            self.status = Some(Status::Error(
                fl!("backend-save-failed", reason = error.to_string()),
                None,
            ));
        }
        if let SettingsLoad::Ready(view) = &mut self.settings {
            view.backend = choice;
        }
        Task::none()
    }

    /// Displayed snapshots, newest first; empty unless a list succeeded.
    fn snapshots(&self) -> &[Snapshot] {
        match &self.listing {
            Listing::Loaded(list) => &list.snapshots,
            Listing::NotLoaded | Listing::Failed(_) => &[],
        }
    }

    /// `09-27 09:12 "comment"` for snapshot `name` in the last list (see [`fmt::label`]), an
    /// interrupted snapshot's words for a leftover, or `name` itself.
    fn snapshot_label(&self, name: &str) -> String {
        if let Some(snapshot) = self.snapshots().iter().find(|s| s.name == name) {
            return fmt::label(snapshot);
        }
        match apsis_core::parse_snapshot_name(name) {
            Some(when) if matches!(&self.listing, Listing::Loaded(l) if l.leftovers.iter().any(|n| n == name)) =>
            {
                fl!("leftover-label", when = fmt::when(when))
            }
            _ => name.to_owned(),
        }
    }

    /// Why something failed, as a line of text.
    fn error_text(&self, error: &CliError) -> String {
        error_summary(error, self.known_uuid.as_deref())
    }

    /// What the panel shows, from the last list and the reminder setting. The one place the
    /// panel label, tooltip and icon colour come from.
    fn status_view(&self) -> StatusView {
        self.status_view_at(jiff::Zoned::now().datetime())
    }

    fn status_view_at(&self, now: jiff::civil::DateTime) -> StatusView {
        match &self.listing {
            Listing::Loaded(list) => {
                let status = ApsisStatus::from_list(list, self.config.remind_days, now);
                if self.disk_gone() {
                    StatusView::Loaded(status.disk_gone())
                } else {
                    StatusView::Loaded(status)
                }
            }
            Listing::Failed(_) => StatusView::Failed,
            Listing::NotLoaded => StatusView::Unloaded,
        }
    }
}

/// One row of the list.
#[derive(Debug, Clone, Copy)]
enum RowItem<'a> {
    Snapshot(&'a Snapshot),
    /// An interrupted create's folder; its name is when it started.
    Leftover(&'a str),
}

impl RowItem<'_> {
    fn name(&self) -> &str {
        match self {
            Self::Snapshot(s) => &s.name,
            Self::Leftover(name) => name,
        }
    }
}

/// Makes the process's connection to `apsis-helper`, which does everything that needs root.
/// Apsis can't work without it.
fn connect() -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async { Message::Connected(HelperClient::connect().await) })
}

/// The helper for a task: the process's connection, or (none yet, between a bus drop and the
/// reconnect) one of its own for this call.
async fn helper(link: Link) -> Result<HelperClient, CliError> {
    match link {
        Some(client) => Ok(client),
        None => HelperClient::connect().await.ok_or(CliError::NoHelper),
    }
}

/// Lists through `apsis-helper` (no password for the active session).
async fn list_snapshots(link: Link) -> Result<SnapshotList, CliError> {
    helper(link).await?.list().await.map_err(CliError::from)
}

/// Lists through `apsis-helper` for the reminder; `None` without a helper.
fn background_list(link: Link) -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        let Ok(helper) = helper(link).await else {
            return Message::BackgroundListed(None);
        };
        Message::BackgroundListed(Some(helper.list().await.map_err(CliError::from)))
    })
}

/// What the helper is doing, without starting it just to ask.
fn poll_job(link: Link) -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        let job = match helper(link).await {
            Ok(helper) => helper.job().await.ok().flatten(),
            Err(_) => None,
        };
        Message::JobPolled(job)
    })
}

/// Every `JobChanged`, and the helper leaving the bus, on the process's connection, for as
/// long as it lasts; then [`Message::BusLost`], so a new one is made.
fn job_events(source: &JobSource) -> impl Stream<Item = Message> + Send + use<> {
    let client = source.client.clone();
    stream::once(async move { client.job_changes().await.ok() })
        .filter_map(|changes| async move { changes })
        .flat_map(|changes| {
            changes
                .map(Message::Job)
                .chain(stream::once(async { Message::BusLost }))
        })
}

/// Asks the helper to stop the create making `snapshot`.
async fn stop(link: Link, snapshot: &str) -> Result<(), CliError> {
    helper(link)
        .await?
        .stop(snapshot)
        .await
        .map_err(CliError::from)
}

/// Creates or deletes through `apsis-helper`; returns when its `Finished` signal arrives,
/// however long rsync takes. A delete of several is one call and one job; the password is
/// asked once.
async fn operate(
    link: Link,
    operation: Operation,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<(), CliError> {
    let helper = helper(link).await?;
    let done = match &operation {
        Operation::Create(comment) => helper.create_with_progress(comment, progress).await,
        Operation::Delete(name) => helper.delete(name).await,
        Operation::DeleteMany(names) => helper.delete_many(names, progress).await,
        Operation::Restore {
            snapshot,
            restore_home,
            safety_snapshot,
        } => {
            helper
                .restore(snapshot, *restore_home, *safety_snapshot, progress)
                .await
        }
    };
    done.map_err(CliError::from)
}

/// `CheckRestore` through `apsis-helper` (no password).
async fn check_restore(link: Link, snapshot: &str) -> Result<Check, CliError> {
    helper(link)
        .await?
        .check_restore(snapshot)
        .await
        .map_err(CliError::from)
}

/// "Restart now" through `apsis-helper`.
async fn restart_to_restore(link: Link, snapshot: &str) -> Result<(), CliError> {
    helper(link)
        .await?
        .restart_to_restore(snapshot)
        .await
        .map_err(CliError::from)
}

/// "Cancel restore" through `apsis-helper`.
async fn cancel_restore(link: Link) -> Result<(), CliError> {
    helper(link)
        .await?
        .cancel_restore()
        .await
        .map_err(CliError::from)
}

/// The last restore's result, when the window opens.
fn read_restore_result(link: Link) -> Task<cosmic::Action<Message>> {
    cosmic::task::future(async move {
        let result = match helper(link).await {
            Ok(helper) => helper.restore_result().await.map_err(CliError::from),
            Err(error) => Err(error),
        };
        Message::RestoreResultRead(result)
    })
}

/// The lines for a plan the helper no longer has, or has had too long (PLAN 6b.5, 6b.9);
/// anything else is `fallback`.
fn plan_error_text(error: &CliError, fallback: &str) -> String {
    match error {
        CliError::Other(text) if text == plan::TOO_OLD => fl!("restore-too-old"),
        CliError::Other(text) if text == plan::GONE => fl!("restore-gone"),
        _ => fallback.to_owned(),
    }
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

/// Apsis's config (converted or imported until it's saved) and the devices, through
/// `apsis-helper`.
async fn read_config(link: Link) -> Result<ConfigInfo, CliError> {
    helper(link)
        .await?
        .read_config()
        .await
        .map_err(CliError::from)
}

/// Writes `config` through `apsis-helper` if `config.toml` still reads `expected`.
async fn write_config(
    link: Link,
    expected: String,
    config: ApsisConfig,
) -> Result<String, CliError> {
    helper(link)
        .await?
        .write_config(&expected, &config)
        .await
        .map_err(CliError::from)
}

/// Opens the file chooser (the XDG portal, as the user) for a folder or a file.
fn pick(folder: bool) -> Task<cosmic::Action<Message>> {
    use cosmic::dialog::file_chooser;
    cosmic::task::future(async move {
        let dialog = file_chooser::open::Dialog::new().title(if folder {
            fl!("pick-folder")
        } else {
            fl!("pick-file")
        });
        let picked = if folder {
            dialog.open_folder().await
        } else {
            dialog.open_file().await
        };
        let path = picked
            .ok()
            .and_then(|response| response.url().to_file_path().ok())
            .map(|path| (path.to_string_lossy().into_owned(), folder));
        Message::Picked(path)
    })
}

/// Why something failed, as a line of text.
fn error_summary(error: &CliError, known_uuid: Option<&str>) -> String {
    match error {
        CliError::NoHelper => fl!("need-helper"),
        CliError::NotAuthorized => fl!("failed-auth"),
        CliError::DeviceNotFound { device } => disk_missing(device, known_uuid),
        CliError::Busy => fl!("busy-background"),
        CliError::DiskRemoved { reason } => format!("{}: {reason}", fl!("disk-removed")),
        CliError::Stopped => fl!("create-stopped"),
        CliError::DeleteManyStopped { failed, reason, .. } => fl!(
            "delete-many-stopped",
            name = failed.clone(),
            reason = error_summary(reason, known_uuid)
        ),
        CliError::RestoreRefused(refusal) => refusal_lines(refusal).0,
        CliError::RestoreArmed => fl!("refused-restore-armed"),
        CliError::Other(message) => message.clone(),
    }
}

/// A delete of several that stopped: what was deleted and what wasn't (including the one that
/// failed). Takes the snapshots' labels.
fn bulk_delete_details(deleted: &[String], left: &[String]) -> String {
    let list = |names: &[String]| {
        if names.is_empty() {
            fl!("delete-none")
        } else {
            names.join(", ")
        }
    };
    [
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

/// `Backup disk not connected (UUID 1a2b…). Plug it in.` Names the disk by the UUID from the
/// last good list.
fn disk_missing(device: &str, known_uuid: Option<&str>) -> String {
    let id = match known_uuid {
        Some(uuid) => format!("UUID {}", fmt::short_uuid(uuid)),
        None if !device.starts_with('/') => format!("UUID {}", fmt::short_uuid(device)),
        None => device.to_owned(),
    };
    fl!("disk-missing", id = id)
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
/// the `PATH`, with `flag` (`--window`, `--settings` or `--about`), and the activation token if
/// the compositor gave one.
fn window_command(
    exe: Option<std::path::PathBuf>,
    flag: &str,
    token: Option<String>,
) -> std::process::Command {
    let program = exe
        .map(|path| {
            let text = path.to_string_lossy();
            std::path::PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(&text))
        })
        .unwrap_or_else(|| "apsis".into());
    let mut command = std::process::Command::new(program);
    command.arg(flag);
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
    command
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

/// The window's keyboard shortcuts, and the modifier state for clicks in the list. A key a
/// widget took (typing in a text field) is left alone, except Esc.
fn shortcut(event: event::Event, status: event::Status, _window: Id) -> Option<Message> {
    let (key, modifiers) = match event {
        event::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            return Some(Message::ModifiersChanged(modifiers));
        }
        event::Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key,
            modifiers,
            ..
        }) => (modified_key, modifiers),
        _ => return None,
    };
    shortcut_for(&key, modifiers, status == event::Status::Captured).map(Message::Shortcut)
}

/// Which shortcut `key` with `modifiers` is; `captured` when a widget took it.
fn shortcut_for(key: &Key, modifiers: Modifiers, captured: bool) -> Option<Shortcut> {
    if let Key::Named(Named::Escape) = key {
        return Some(Shortcut::Escape);
    }
    if captured || modifiers.alt() || modifiers.logo() {
        return None;
    }
    let ctrl = modifiers.control();
    match key.as_ref() {
        Key::Named(Named::F5) if !ctrl => Some(Shortcut::Refresh),
        Key::Named(Named::Delete) if !ctrl => Some(Shortcut::Delete),
        Key::Named(Named::ArrowUp) if !ctrl => Some(Shortcut::Up),
        Key::Named(Named::ArrowDown) if !ctrl => Some(Shortcut::Down),
        Key::Character(c) if ctrl => match c.to_lowercase().as_str() {
            "n" => Some(Shortcut::Create),
            "r" => Some(Shortcut::Refresh),
            "," => Some(Shortcut::Settings),
            "a" => Some(Shortcut::SelectAll),
            _ => None,
        },
        _ => None,
    }
}

/// The window's height, for the restore dialog's scrolling body.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the signature event::listen_with takes"
)]
fn window_resized(event: event::Event, _status: event::Status, _window: Id) -> Option<Message> {
    match event {
        event::Event::Window(window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.height))
        }
        _ => None,
    }
}

/// The popup and the menu: Esc closes them.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the signature event::listen_with takes"
)]
fn escape_only(event: event::Event, _status: event::Status, _window: Id) -> Option<Message> {
    match event {
        event::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        }) => Some(Message::Escape),
        _ => None,
    }
}

/// Window mode: the main window got focus, so it's on screen.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the signature event::listen_with takes"
)]
fn window_focused(event: event::Event, _status: event::Status, _window: Id) -> Option<Message> {
    matches!(event, event::Event::Window(window::Event::Focused)).then_some(Message::WindowShown)
}
