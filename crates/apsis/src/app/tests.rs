// SPDX-License-Identifier: GPL-3.0-only

//! The window's and the panel's behaviour, message by message. `update`'s tasks are never run
//! here, so nothing reaches the helper or runs as root.

use apsis_core::DiskUsage;
use apsis_core::config::System;
use cosmic::iced::keyboard::Modifiers;

use super::*;
use crate::status::DiskStrip;

/// A fresh, empty folder for one `/dev/disk/by-uuid` stand-in. Tests run on parallel threads, so
/// each call gets its own (pid and a counter); the first call of a run removes earlier runs'.
fn by_uuid_dir() -> std::path::PathBuf {
    static CLEANED: std::sync::Once = std::sync::Once::new();
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tmp = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    let base = tmp.join("by-uuid");
    CLEANED.call_once(|| {
        for old in [&base, &tmp.join("by-uuid-in"), &tmp.join("by-uuid-out")] {
            match std::fs::remove_dir_all(old) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("{e}"),
                _ => {}
            }
        }
    });
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = base.join(format!("{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `/dev/disk/by-uuid` stand-in with the fixture's disk in it.
fn plugged_in() -> std::path::PathBuf {
    let dir = by_uuid_dir();
    std::os::unix::fs::symlink("/", dir.join(FIXTURE_UUID)).unwrap();
    dir
}

/// One with nothing in it: the disk is unplugged.
fn unplugged() -> std::path::PathBuf {
    by_uuid_dir()
}

const FIXTURE_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// An applet with a main window and nothing open or listed.
fn model() -> AppModel {
    let mut core = cosmic::Core::default();
    core.set_main_window_id(Some(Id::unique()));
    let mut app = AppModel::new(core, Mode::Applet, Config::default());
    app.disk_links = plugged_in();
    app
}

/// A window, listed with the fixture.
fn window() -> AppModel {
    let mut app = model();
    app.mode = Mode::Window;
    app.popup = app.core.main_window_id();
    app.on_listed(Ok(fixture()));
    app
}

fn send(app: &mut AppModel, message: Message) {
    let _ = cosmic::Application::update(app, message);
}

/// Five snapshots on a configured device, the newest with a comment (as the helper lists
/// them, oldest first), and one leftover.
fn fixture() -> SnapshotList {
    let snapshot = |name: &str, comment: Option<&str>| Snapshot {
        name: name.to_owned(),
        created: apsis_core::parse_snapshot_name(name).unwrap(),
        tags: vec![apsis_core::Tag::OnDemand],
        comment: comment.map(str::to_owned),
        rsync_flags: None,
    };
    SnapshotList {
        device: Some("/dev/sdX1".to_owned()),
        uuid: Some(FIXTURE_UUID.to_owned()),
        mode: Some(apsis_core::Mode::Rsync),
        snapshots: vec![
            snapshot("2026-09-19_09-29-57", None),
            snapshot("2026-09-19_09-58-41", None),
            snapshot("2026-09-22_13-28-36", None),
            snapshot("2026-09-23_08-33-55", None),
            snapshot("2026-09-25_11-28-53", Some("before driver update")),
        ],
        warnings: Vec::new(),
        leftovers: vec!["2026-09-26_14-02-11".to_owned()],
        usage: None,
    }
}

/// The names in list order: newest first, then the leftover.
const NEWEST: &str = "2026-09-25_11-28-53";
const SECOND: &str = "2026-09-23_08-33-55";
const THIRD: &str = "2026-09-22_13-28-36";
const LEFTOVER: &str = "2026-09-26_14-02-11";

fn click(app: &mut AppModel, index: usize, modifiers: Modifiers) {
    send(app, Message::ModifiersChanged(modifiers));
    send(app, Message::RowClicked(index));
    send(app, Message::ModifiersChanged(Modifiers::empty()));
}

fn selected(app: &AppModel) -> Vec<String> {
    app.selected_names()
}

fn job(kind: JobKind, state: JobState, snapshot: &str) -> Job {
    Job {
        kind,
        state,
        snapshot: snapshot.to_owned(),
        started: 0,
        percent: None,
        eta_seconds: None,
    }
}

#[test]
fn right_click_swaps_the_popup_for_the_menu() {
    let mut app = model();
    send(&mut app, Message::TogglePopup);
    assert!(app.popup.is_some());
    send(&mut app, Message::ToggleMenu);
    assert!(app.popup.is_none());
    assert!(app.menu.is_some());
    send(&mut app, Message::ToggleMenu);
    assert!(app.menu.is_none());
}

#[test]
fn left_click_swaps_the_menu_for_the_popup_and_esc_closes_what_is_open() {
    let mut app = model();
    send(&mut app, Message::ToggleMenu);
    send(&mut app, Message::TogglePopup);
    assert!(app.menu.is_none() && app.popup.is_some());
    send(&mut app, Message::Escape);
    assert!(app.popup.is_none());
    send(&mut app, Message::ToggleMenu);
    send(&mut app, Message::Escape);
    assert!(app.menu.is_none());
}

#[test]
fn menu_refresh_opens_the_popup_and_lists() {
    let mut app = model();
    send(&mut app, Message::ToggleMenu);
    send(&mut app, Message::MenuRefresh);
    assert!(app.menu.is_none() && app.popup.is_some());
    assert!(app.loading);
}

#[test]
fn menu_close_closes_only_the_menu() {
    let mut app = model();
    send(&mut app, Message::ToggleMenu);
    send(&mut app, Message::MenuClose);
    assert!(app.menu.is_none() && app.popup.is_none());
}

#[test]
fn the_window_lists_at_once_and_becomes_resizable_once_shown() {
    let mut core = cosmic::Core::default();
    core.set_main_window_id(Some(Id::unique()));
    let mut app = AppModel::new(core, Mode::Window, Config::default());
    let _ = app.open_window();
    assert!(app.loading, "connecting, then listing");
    assert_eq!(app.popup, app.core.main_window_id());
    assert!(!app.window_resizable);
    send(&mut app, Message::WindowShown);
    assert!(app.window_resizable);
}

#[test]
fn the_window_command_is_this_program_with_a_flag() {
    let command = |exe: Option<&str>, flag| {
        let command = window_command(exe.map(Into::into), flag, None);
        let program = command.get_program().to_string_lossy().into_owned();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        (program, args)
    };
    assert_eq!(
        command(Some("/usr/bin/apsis"), "--window"),
        ("/usr/bin/apsis".to_owned(), vec!["--window".to_owned()])
    );
    // A package upgrade while the panel runs.
    assert_eq!(
        command(Some("/usr/bin/apsis (deleted)"), "--settings"),
        ("/usr/bin/apsis".to_owned(), vec!["--settings".to_owned()])
    );
    assert_eq!(
        command(None, "--about"),
        ("apsis".to_owned(), vec!["--about".to_owned()])
    );
    let command = window_command(
        Some("/usr/bin/apsis".into()),
        "--window",
        Some("t0k".into()),
    );
    let token = command
        .get_envs()
        .find(|(key, _)| *key == "XDG_ACTIVATION_TOKEN")
        .and_then(|(_, value)| value);
    assert_eq!(token, Some(std::ffi::OsStr::new("t0k")));
}

#[test]
fn a_second_start_switches_the_page_only_when_idle() {
    let mut app = window();
    let _ = app.on_activation(Some("about"));
    assert_eq!(app.page, Page::About);
    app.page = Page::List;
    let _ = app.on_activation(None);
    assert_eq!(app.page, Page::List);
    // A dialog open: left alone.
    app.dialog = Some(Dialog::Unsaved);
    let _ = app.on_activation(Some("settings"));
    assert_eq!(app.page, Page::List);
    app.dialog = None;
    // A job running: left alone.
    app.running = Some(Operation::Create(String::new()));
    let _ = app.on_activation(Some("settings"));
    assert_eq!(app.page, Page::List);
    app.running = None;
    let _ = app.on_activation(Some("settings"));
    assert_eq!(app.page, Page::Settings);
    assert_eq!(StartView::from_action("nonsense"), None);
    let args = |list: &[&str]| {
        list.iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
    };
    assert_eq!(StartView::from_args(args(&["--window"])), None);
    assert_eq!(
        StartView::from_args(args(&["--window", "--about"])),
        Some(StartView::About)
    );
}

#[test]
fn the_list_is_newest_first_then_the_leftovers() {
    let app = window();
    let rows = app.rows();
    let names: Vec<&str> = rows.iter().map(RowItem::name).collect();
    assert_eq!(names[0], NEWEST);
    assert_eq!(names.last(), Some(&LEFTOVER));
    assert_eq!(names.len(), 6);
    assert_eq!(
        app.snapshot_label(LEFTOVER),
        fl!(
            "leftover-label",
            when = fmt::when(apsis_core::parse_snapshot_name(LEFTOVER).unwrap())
        )
    );
}

#[test]
fn toolbar_buttons_follow_the_list_selection_and_jobs() {
    let mut app = model();
    assert!(!app.can_create(), "nothing listed");
    app.on_listed(Ok(SnapshotList::default()));
    assert!(!app.can_create(), "no backup device");
    let mut app = window();
    assert!(app.can_create());
    assert!(!app.can_delete(), "nothing selected");
    click(&mut app, 0, Modifiers::empty());
    assert!(app.can_delete());
    app.running = Some(Operation::Create(String::new()));
    assert!(!app.can_create() && !app.can_delete());
    app.running = None;
    // Another window's create.
    app.job = Some(job(JobKind::Create, JobState::Running, NEWEST));
    assert!(!app.can_create());
    // A config write elsewhere doesn't hold the toolbar up (a list is never a job).
    app.job = Some(job(JobKind::Configure, JobState::Running, ""));
    assert!(app.can_create());
}

#[test]
fn clicks_select_one_ctrl_adds_and_shift_takes_a_range() {
    let mut app = window();
    click(&mut app, 1, Modifiers::empty());
    assert_eq!(selected(&app), [SECOND]);
    click(&mut app, 0, Modifiers::CTRL);
    assert_eq!(selected(&app), [NEWEST, SECOND]);
    click(&mut app, 0, Modifiers::CTRL);
    assert_eq!(selected(&app), [SECOND]);
    click(&mut app, 0, Modifiers::empty());
    click(&mut app, 2, Modifiers::SHIFT);
    assert_eq!(selected(&app), [NEWEST, SECOND, THIRD]);
    click(&mut app, 3, Modifiers::empty());
    assert_eq!(selected(&app).len(), 1);
    // The keyboard: Up and Down move a single selection, Ctrl+A takes every row.
    send(&mut app, Message::Shortcut(Shortcut::Up));
    assert_eq!(selected(&app), [THIRD]);
    send(&mut app, Message::Shortcut(Shortcut::SelectAll));
    assert_eq!(selected(&app).len(), 6);
    send(&mut app, Message::Shortcut(Shortcut::Escape));
    assert!(selected(&app).is_empty());
}

#[test]
fn a_new_list_keeps_the_selection_of_what_is_still_there() {
    let mut app = window();
    click(&mut app, 0, Modifiers::empty());
    click(&mut app, 1, Modifiers::CTRL);
    let mut list = fixture();
    list.snapshots.retain(|s| s.name != SECOND);
    app.on_listed(Ok(list));
    assert_eq!(selected(&app), [NEWEST]);
    app.on_listed(Err(CliError::NoHelper));
    assert!(selected(&app).is_empty());
}

#[test]
fn delete_asks_about_the_selection_in_list_order_then_deletes_several_as_one_job() {
    let mut app = window();
    click(&mut app, 5, Modifiers::empty());
    click(&mut app, 0, Modifiers::CTRL);
    send(&mut app, Message::DeleteClicked);
    let Some(Dialog::Delete { names }) = app.dialog.clone() else {
        panic!("no delete dialog")
    };
    assert_eq!(names, [NEWEST, LEFTOVER]);
    send(&mut app, Message::DialogCancel);
    assert!(app.dialog.is_none() && app.running.is_none());
    send(&mut app, Message::Shortcut(Shortcut::Delete));
    send(&mut app, Message::DialogConfirm);
    // One `DeleteMany` call for both, not one `Delete` after the other.
    let many = Operation::DeleteMany(names.clone());
    assert_eq!(app.running, Some(many.clone()));
    // The progress line follows the helper's `JobChanged`: which snapshot it's on.
    let mut step = job(JobKind::DeleteMany, JobState::Running, LEFTOVER);
    step.percent = Some(50.0);
    send(&mut app, Message::Job(JobEvent::Changed(step)));
    assert_eq!(app.running, Some(many.clone()), "still this window's job");
    let line = app.delete_progress();
    assert!(line.starts_with("Deleting 2 of 2: "), "{line}");
    send(&mut app, Message::Finished(many, Ok(())));
    assert!(app.running.is_none());
    assert_eq!(
        app.status,
        Some(Status::Info(fl!("deleted-many", count = "2")))
    );
    assert!(app.loading, "lists again");
    assert!(selected(&app).is_empty());
}

#[test]
fn one_selected_snapshot_is_a_plain_delete() {
    let mut app = window();
    click(&mut app, 1, Modifiers::empty());
    send(&mut app, Message::Shortcut(Shortcut::Delete));
    send(&mut app, Message::DialogConfirm);
    let one = Operation::Delete(SECOND.to_owned());
    assert_eq!(app.running, Some(one.clone()));
    let line = app.delete_progress();
    assert!(
        line.starts_with("Deleting ") && !line.contains(" of "),
        "{line}"
    );
    send(&mut app, Message::Finished(one, Ok(())));
    assert!(selected(&app).is_empty() && app.loading);
}

#[test]
fn a_delete_of_several_stops_at_the_first_failure_and_says_what_went() {
    let mut app = window();
    click(&mut app, 0, Modifiers::empty());
    click(&mut app, 2, Modifiers::SHIFT);
    send(&mut app, Message::DeleteClicked);
    send(&mut app, Message::DialogConfirm);
    let names = vec![NEWEST.to_owned(), SECOND.to_owned(), THIRD.to_owned()];
    assert_eq!(app.running, Some(Operation::DeleteMany(names.clone())));
    // The helper's account: the first went, the second failed, the third wasn't touched.
    send(
        &mut app,
        Message::Finished(
            Operation::DeleteMany(names),
            Err(CliError::DeleteManyStopped {
                deleted: vec![NEWEST.to_owned()],
                failed: SECOND.to_owned(),
                left: vec![THIRD.to_owned()],
                reason: Box::new(CliError::Other("no space".to_owned())),
            }),
        ),
    );
    let Some(Status::Error(line, Some(details))) = &app.status else {
        panic!("{:?}", app.status)
    };
    assert!(line.starts_with("Delete stopped at "), "{line}");
    assert!(
        line.contains(&app.snapshot_label(SECOND)) && line.contains("no space"),
        "{line}"
    );
    assert!(
        details.contains("(1)") && details.contains("(2)"),
        "{details}"
    );
    // The ones not deleted stay selected; one was deleted, so it lists again.
    assert_eq!(selected(&app), [SECOND, THIRD]);
    assert!(app.loading);

    // Stopped on the first because the disk went: nothing deleted, no list (it would only
    // fail), and the disk is known to be gone.
    let mut app = window();
    let names = vec![NEWEST.to_owned(), SECOND.to_owned()];
    send(
        &mut app,
        Message::Finished(
            Operation::DeleteMany(names),
            Err(CliError::DeleteManyStopped {
                deleted: Vec::new(),
                failed: NEWEST.to_owned(),
                left: vec![SECOND.to_owned()],
                reason: Box::new(CliError::DiskRemoved {
                    reason: "I/O error".to_owned(),
                }),
            }),
        ),
    );
    assert!(matches!(app.status, Some(Status::Error(_, Some(_)))));
    assert!(!app.loading);
    assert_eq!(app.disk_seen, Some(false));
}

/// The helper frees the lock, announces the end (through its own task) and sends `Finished`
/// to the caller: on the bus the two can come in either order. This window's own job must be
/// known as its own whichever comes first; the real bug (apsis-test, check 13): `Finished`
/// first, then the end, shown as "A delete started elsewhere failed".
fn own_job_in_both_orders(
    operation: Operation,
    result: Result<(), CliError>,
    end: JobState,
    expected: &dyn Fn(&Status) -> bool,
) {
    let kind = operation.kind();
    let name = match &operation {
        Operation::Create(_) => NEWEST,
        Operation::Delete(name) => name.as_str(),
        Operation::DeleteMany(names) => names.last().unwrap().as_str(),
        Operation::Restore { snapshot, .. } => snapshot.as_str(),
    };
    let mut started = job(kind, JobState::Running, name);
    started.started = 1_790_000_000;
    let mut ended = job(kind, end, name);
    ended.started = 1_790_000_000;
    for (order, finished_first, seen_running) in [
        (
            "Finished first (the real order), the job seen running",
            true,
            true,
        ),
        ("Finished first, the job never seen running", true, false),
        ("the end first (the reverse)", false, true),
    ] {
        let mut app = window();
        app.running = Some(operation.clone());
        if seen_running {
            send(&mut app, Message::Job(JobEvent::Changed(started.clone())));
            assert_eq!(app.running, Some(operation.clone()), "{order}");
        }
        if finished_first {
            send(
                &mut app,
                Message::Finished(operation.clone(), result.clone()),
            );
            let after_finished = app.status.clone();
            send(&mut app, Message::Job(JobEvent::Changed(ended.clone())));
            assert_eq!(
                app.status, after_finished,
                "{order}: the late end changed the line"
            );
        } else {
            send(&mut app, Message::Job(JobEvent::Changed(ended.clone())));
            assert_eq!(app.running, Some(operation.clone()), "{order}: still ours");
            send(
                &mut app,
                Message::Finished(operation.clone(), result.clone()),
            );
        }
        let status = app.status.clone().expect(order);
        assert!(expected(&status), "{order}: {status:?}");
        assert!(app.running.is_none() && app.job.is_none(), "{order}");
        assert!(app.own_end.is_none(), "{order}: nothing left to wait for");
        assert!(app.loading, "{order}: listed once");
        // The next end of the same kind is another window's again.
        app.loading = false;
        let mut other = job(kind, JobState::Done, SECOND);
        other.started = 1_790_000_900;
        send(&mut app, Message::Job(JobEvent::Changed(other)));
        let elsewhere = match kind {
            JobKind::Create => fl!("created"),
            _ => fl!("deleted-elsewhere"),
        };
        assert_eq!(app.status, Some(Status::Info(elsewhere)), "{order}");
        assert!(app.loading, "{order}: another window's end lists");
    }
}

#[test]
fn a_failed_delete_of_several_is_this_windows_whichever_comes_first() {
    let names = vec![NEWEST.to_owned(), SECOND.to_owned()];
    let stopped = CliError::DeleteManyStopped {
        deleted: vec![NEWEST.to_owned()],
        failed: SECOND.to_owned(),
        left: Vec::new(),
        reason: Box::new(CliError::Other(
            "no snapshot called \"x\" on the backup device".to_owned(),
        )),
    };
    own_job_in_both_orders(
        Operation::DeleteMany(names),
        Err(stopped),
        JobState::Failed,
        &|status| {
            matches!(status, Status::Error(line, Some(details))
                if line.starts_with("Delete stopped at ") && details.contains("(1)"))
        },
    );
    // Not "A delete started elsewhere failed", and the selection keeps what wasn't deleted.
    let mut app = window();
    click(&mut app, 0, Modifiers::empty());
    click(&mut app, 1, Modifiers::CTRL);
    send(&mut app, Message::DeleteClicked);
    send(&mut app, Message::DialogConfirm);
    let names = vec![NEWEST.to_owned(), SECOND.to_owned()];
    let mut ended = job(JobKind::DeleteMany, JobState::Failed, SECOND);
    ended.started = 7;
    send(
        &mut app,
        Message::Finished(
            Operation::DeleteMany(names),
            Err(CliError::DeleteManyStopped {
                deleted: vec![NEWEST.to_owned()],
                failed: SECOND.to_owned(),
                left: Vec::new(),
                reason: Box::new(CliError::Other("gone".to_owned())),
            }),
        ),
    );
    send(&mut app, Message::Job(JobEvent::Changed(ended)));
    let Some(Status::Error(line, _)) = &app.status else {
        panic!("{:?}", app.status)
    };
    assert_ne!(line, &fl!("delete-failed-elsewhere"));
    assert!(line.starts_with("Delete stopped at "), "{line}");
    assert_eq!(selected(&app), [SECOND]);
}

#[test]
fn a_successful_delete_of_several_is_this_windows_whichever_comes_first() {
    let names = vec![NEWEST.to_owned(), SECOND.to_owned(), THIRD.to_owned()];
    own_job_in_both_orders(
        Operation::DeleteMany(names),
        Ok(()),
        JobState::Done,
        &|status| *status == Status::Info(fl!("deleted-many", count = "3")),
    );
}

#[test]
fn a_failed_single_delete_is_this_windows_whichever_comes_first() {
    own_job_in_both_orders(
        Operation::Delete(SECOND.to_owned()),
        Err(CliError::Other("no space".to_owned())),
        JobState::Failed,
        &|status| {
            matches!(status, Status::Error(line, None)
                if line.starts_with("Delete failed") && line.contains("no space"))
        },
    );
}

#[test]
fn a_failed_create_is_this_windows_whichever_comes_first() {
    own_job_in_both_orders(
        Operation::Create("before update".to_owned()),
        Err(CliError::Other("rsync exited with code 11".to_owned())),
        JobState::Failed,
        &|status| {
            matches!(status, Status::Error(line, None)
                if line.starts_with("Create failed") && line.contains("code 11"))
        },
    );
    own_job_in_both_orders(
        Operation::Create(String::new()),
        Err(CliError::Stopped),
        JobState::Stopped,
        &|status| *status == Status::Info(fl!("create-stopped")),
    );
}

/// Refused before the lock (polkit, busy, no helper): no job began, so no end is waited
/// for, and the next end of that kind is another window's at once.
#[test]
fn a_refusal_before_the_job_began_waits_for_no_end() {
    for refused in [CliError::NotAuthorized, CliError::Busy, CliError::NoHelper] {
        let mut app = window();
        app.running = Some(Operation::Delete(SECOND.to_owned()));
        send(
            &mut app,
            Message::Finished(Operation::Delete(SECOND.to_owned()), Err(refused.clone())),
        );
        assert!(app.own_end.is_none(), "{refused:?}");
        app.loading = false;
        send(
            &mut app,
            Message::Job(JobEvent::Changed(job(
                JobKind::Delete,
                JobState::Done,
                THIRD,
            ))),
        );
        assert_eq!(
            app.status,
            Some(Status::Info(fl!("deleted-elsewhere"))),
            "{refused:?}"
        );
        assert!(app.loading, "{refused:?}");
    }
}

/// The helper leaving the bus clears what was waited for; an end of another kind too.
#[test]
fn a_waited_for_end_is_dropped_when_something_else_comes() {
    let mut app = window();
    app.running = Some(Operation::Delete(SECOND.to_owned()));
    send(
        &mut app,
        Message::Finished(Operation::Delete(SECOND.to_owned()), Ok(())),
    );
    assert!(app.own_end.is_some());
    send(&mut app, Message::Job(JobEvent::HelperGone));
    assert!(app.own_end.is_none());

    let mut app = window();
    app.running = Some(Operation::Delete(SECOND.to_owned()));
    send(
        &mut app,
        Message::Finished(Operation::Delete(SECOND.to_owned()), Ok(())),
    );
    app.loading = false;
    // A create elsewhere ending: not ours, shown and listed; nothing waited for any more.
    send(
        &mut app,
        Message::Job(JobEvent::Changed(job(
            JobKind::Create,
            JobState::Done,
            THIRD,
        ))),
    );
    assert_eq!(app.status, Some(Status::Info(fl!("created"))));
    assert!(app.loading && app.own_end.is_none());
}

/// What the helper said reaches the status line, the tooltip and the dialogs as it was
/// said: `Error::Helper`'s own `apsis-helper: ` prefix (the journal's) is dropped on the way
/// into [`CliError`], the one place every shown reason goes through.
#[test]
fn a_helper_reason_is_shown_without_the_crates_prefix() {
    let said = "no snapshot called \"2026-10-01_20-21-45\" on the backup device";
    let error = apsis_core::Error::Helper(said.to_owned());
    assert!(
        error.to_string().starts_with("apsis-helper: "),
        "the journal's form"
    );
    assert_eq!(CliError::from(error), CliError::Other(said.to_owned()));
    assert_eq!(error_summary(&CliError::Other(said.to_owned()), None), said);
    // Inside a stopped delete of several: the reason is the helper's words too.
    let stopped = CliError::from(apsis_core::Error::DeleteManyStopped {
        deleted: vec![NEWEST.to_owned()],
        failed: SECOND.to_owned(),
        left: Vec::new(),
        reason: Box::new(apsis_core::Error::Helper(said.to_owned())),
    });
    let line = error_summary(&stopped, None);
    assert!(
        line.contains(said) && !line.contains("apsis-helper"),
        "{line}"
    );
    let mut app = window();
    app.running = Some(Operation::DeleteMany(vec![
        NEWEST.to_owned(),
        SECOND.to_owned(),
    ]));
    send(
        &mut app,
        Message::Finished(
            Operation::DeleteMany(vec![NEWEST.to_owned(), SECOND.to_owned()]),
            Err(stopped),
        ),
    );
    let Some(Status::Error(line, Some(details))) = &app.status else {
        panic!("{:?}", app.status)
    };
    assert!(
        !line.contains("apsis-helper") && line.contains(said),
        "{line}"
    );
    assert!(!details.contains("apsis-helper"), "{details}");
    // A stop that the helper refused, the same way.
    send(
        &mut app,
        Message::StopAnswered(Some(CliError::Other(said.to_owned()))),
    );
    let Some(Status::Error(line, None)) = &app.status else {
        panic!("{:?}", app.status)
    };
    assert!(
        !line.contains("apsis-helper") && line.contains(said),
        "{line}"
    );
    // Other kinds keep their own text.
    assert_eq!(
        CliError::from(apsis_core::Error::Native(
            "rsync exited with code 11".to_owned()
        )),
        CliError::Other("rsync exited with code 11".to_owned())
    );
}

#[test]
fn create_asks_for_an_optional_comment_and_checks_it_first() {
    let mut app = window();
    send(&mut app, Message::Shortcut(Shortcut::Create));
    assert!(matches!(app.dialog, Some(Dialog::Create { .. })));
    // The config is read for the "what it includes" line.
    assert!(matches!(app.settings, SettingsLoad::Loading));
    send(&mut app, Message::DialogText("-bad".to_owned()));
    send(&mut app, Message::DialogConfirm);
    assert!(
        matches!(&app.dialog, Some(Dialog::Create { error: Some(_), .. })),
        "{:?}",
        app.dialog
    );
    assert!(app.running.is_none());
    send(&mut app, Message::DialogText("x".repeat(500)));
    let Some(Dialog::Create { comment, .. }) = &app.dialog else {
        panic!()
    };
    assert_eq!(comment.chars().count(), MAX_COMMENT_CHARS);
    send(
        &mut app,
        Message::DialogText("before driver update".to_owned()),
    );
    send(&mut app, Message::DialogConfirm);
    assert_eq!(
        app.running,
        Some(Operation::Create("before driver update".to_owned()))
    );
    send(
        &mut app,
        Message::Finished(Operation::Create("before driver update".to_owned()), Ok(())),
    );
    assert_eq!(app.status, Some(Status::Info(fl!("created"))));
    assert!(app.loading);
}

#[test]
fn stop_needs_a_named_running_create_and_a_confirm() {
    let mut app = window();
    send(&mut app, Message::StopClicked);
    assert!(app.dialog.is_none(), "nothing to stop");
    app.running = Some(Operation::Create(String::new()));
    // Not named yet.
    app.job = Some(job(JobKind::Create, JobState::Running, ""));
    send(&mut app, Message::StopClicked);
    assert!(app.dialog.is_none());
    send(
        &mut app,
        Message::Job(JobEvent::Changed(job(
            JobKind::Create,
            JobState::Running,
            NEWEST,
        ))),
    );
    send(&mut app, Message::StopClicked);
    assert_eq!(
        app.dialog,
        Some(Dialog::Stop {
            snapshot: NEWEST.to_owned()
        })
    );
    send(&mut app, Message::DialogConfirm);
    assert_eq!(app.status, Some(Status::Info(fl!("stopping"))));
    // The helper refused (too late): the create goes on, the line says why.
    send(
        &mut app,
        Message::StopAnswered(Some(CliError::Other("being put in place".to_owned()))),
    );
    assert!(matches!(&app.status, Some(Status::Error(line, None)) if line.contains("being put")));
    assert!(app.running.is_some());
    // Or it stopped: the create's own call ends so.
    send(
        &mut app,
        Message::Finished(Operation::Create(String::new()), Err(CliError::Stopped)),
    );
    assert_eq!(app.status, Some(Status::Info(fl!("create-stopped"))));
    assert!(app.loading, "lists again");
}

#[test]
fn a_disk_removed_mid_job_says_so_with_the_reason_aside() {
    let mut app = window();
    app.running = Some(Operation::Create(String::new()));
    send(
        &mut app,
        Message::Finished(
            Operation::Create(String::new()),
            Err(CliError::from(apsis_core::Error::DeviceRemoved {
                device: FIXTURE_UUID.to_owned(),
                reason: "Input/output error (os error 5)".to_owned(),
            })),
        ),
    );
    assert_eq!(
        app.status,
        Some(Status::Error(
            fl!("create-failed-disk-removed"),
            Some("Input/output error (os error 5)".to_owned())
        ))
    );
    assert_eq!(app.disk_seen, Some(false));
    assert!(!app.loading, "a list would only fail");
}

#[test]
fn another_windows_job_shows_here_and_its_end_lists() {
    let mut app = window();
    send(
        &mut app,
        Message::JobPolled(Some(job(JobKind::Create, JobState::Running, NEWEST))),
    );
    assert!(app.active_job().is_some());
    let mut progress = job(JobKind::Create, JobState::Running, NEWEST);
    progress.percent = Some(42.0);
    progress.eta_seconds = Some(90);
    send(&mut app, Message::Job(JobEvent::Changed(progress)));
    let (text, fraction) = app.create_progress_at(Instant::now());
    assert!(text.contains("42%") && text.contains("1m 30s"), "{text}");
    assert!((fraction.unwrap() - 0.42).abs() < 1e-6);
    send(
        &mut app,
        Message::Job(JobEvent::Changed(job(
            JobKind::Create,
            JobState::Done,
            NEWEST,
        ))),
    );
    assert!(app.active_job().is_none());
    assert_eq!(app.status, Some(Status::Info(fl!("created"))));
    assert!(app.loading, "lists again");
}

#[test]
fn a_config_write_ending_elsewhere_doesnt_list_here_even_when_told_busy() {
    let mut app = window();
    send(
        &mut app,
        Message::Job(JobEvent::Changed(job(
            JobKind::Configure,
            JobState::Done,
            "",
        ))),
    );
    assert!(!app.loading, "the list hasn't changed");
    // Told busy (a write was running or waiting): still one list per job end, for a
    // create or delete only; the retry timer covers the rest.
    app.on_listed(Err(CliError::Busy));
    assert!(app.helper_busy);
    send(
        &mut app,
        Message::Job(JobEvent::Changed(job(
            JobKind::Configure,
            JobState::Done,
            "",
        ))),
    );
    assert!(!app.loading);
    send(
        &mut app,
        Message::Job(JobEvent::Changed(job(
            JobKind::DeleteMany,
            JobState::Done,
            "",
        ))),
    );
    assert!(app.loading, "a delete of several changes the list");
    assert_eq!(app.status, Some(Status::Info(fl!("deleted-elsewhere"))));
}

#[test]
fn one_connection_per_process_made_first_then_the_list() {
    // The window: connects first, with `loading` holding other lists off meanwhile.
    let mut core = cosmic::Core::default();
    core.set_main_window_id(Some(Id::unique()));
    let mut app = AppModel::new(core, Mode::Window, Config::default());
    app.disk_links = plugged_in();
    let _ = app.open_window();
    assert!(app.loading && app.helper.is_none());
    assert_eq!(app.bus_generation, 0);
    send(&mut app, Message::Refresh);
    // No helper: nothing to list with, said once, and no reconnect loop.
    send(&mut app, Message::Connected(None));
    assert!(!app.loading && app.helper.is_none());
    assert!(matches!(app.listing, Listing::Failed(CliError::NoHelper)));
    assert_eq!(app.bus_generation, 1);
    // The bus dropping asks for a new connection (a new generation for the subscription).
    send(&mut app, Message::BusLost);
    assert_eq!(app.bus_generation, 2);
    assert!(app.helper.is_none());

    // The applet: the same, ending in the reminder's background list.
    let mut app = model();
    assert!(!app.helper_found);
    app.loading = true;
    send(&mut app, Message::Connected(None));
    assert!(!app.loading && !app.helper_found);
}

#[test]
fn the_helper_leaving_mid_job_ends_it_and_lists() {
    let mut app = window();
    send(
        &mut app,
        Message::JobPolled(Some(job(JobKind::Delete, JobState::Running, NEWEST))),
    );
    send(&mut app, Message::Job(JobEvent::HelperGone));
    assert!(app.job.is_none());
    assert!(app.loading);
}

#[test]
fn an_unplugged_disk_reads_not_connected_and_plugging_in_lists() {
    let mut app = window();
    assert!(app.can_create());
    app.disk_links = unplugged();
    send(&mut app, Message::DiskCheck);
    assert_eq!(app.disk_seen, Some(false));
    let view = app.status_view();
    assert_eq!(
        view.strip().disk,
        DiskStrip::Text(fl!("strip-disk-not-connected"))
    );
    assert_ne!(view.strip().last, "-", "the time side stays");
    assert!(!app.can_create());
    app.disk_links = plugged_in();
    send(&mut app, Message::DiskCheck);
    assert!(app.loading);
}

#[test]
fn the_reminder_needs_a_device_and_an_old_newest_snapshot() {
    use apsis_core::Due;
    let now = jiff::civil::date(2026, 10, 3).at(10, 0, 0, 0);
    let due = |app: &AppModel| match app.status_view_at(now) {
        StatusView::Loaded(status) => status.due(),
        other => panic!("{other:?}"),
    };
    let mut app = window();
    // Newest 09-25 11:28: over 7 days.
    app.config.remind_days = 7;
    assert_eq!(due(&app), Due::Overdue { days: 7 });
    app.config.remind_days = 0;
    assert_eq!(due(&app), Due::Off);
    app.config.remind_days = 7;
    let mut empty = fixture();
    empty.snapshots.clear();
    app.on_listed(Ok(empty.clone()));
    assert_eq!(due(&app), Due::Never { days: 7 });
    empty.device = None;
    app.on_listed(Ok(empty));
    assert_eq!(due(&app), Due::Off);
}

/// A saved config (the real Timeshift settings converted, root known), in the settings.
fn in_settings() -> AppModel {
    const TIMESHIFT: &str = include_str!("../../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../../apsis-core/tests/fixtures/lsblk.json");
    let import = apsis_core::config::import_timeshift(TIMESHIFT, &[]).unwrap();
    let system = System {
        users: vec![apsis_core::settings::User {
            name: "root".to_owned(),
            home: "/root".to_owned(),
            encrypted_home: false,
        }],
        home_entries: Vec::new(),
    };
    let (config, _) = apsis_core::config::convert(&import.legacy, &system);
    let info = apsis_core::helper::config_info_from_wire((
        config.to_text(),
        apsis_core::helper::config_to_wire(&config),
        LSBLK.to_owned(),
        Vec::new(),
    ))
    .unwrap();
    let mut app = window();
    send(&mut app, Message::SettingsClicked);
    assert_eq!(app.page, Page::Settings);
    assert!(matches!(app.settings, SettingsLoad::Loading));
    send(&mut app, Message::SettingsRead(Ok(info)));
    app
}

fn view(app: &AppModel) -> &SettingsView {
    match &app.settings {
        SettingsLoad::Ready(view) => view,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_tabs_are_location_include_filters_and_misc() {
    let mut app = in_settings();
    assert_eq!(app.active_section(), Section::Location);
    let entities: Vec<_> = app.tabs.iter().collect();
    assert_eq!(entities.len(), 4);
    send(&mut app, Message::Tab(entities[2]));
    assert_eq!(app.active_section(), Section::Filters);
}

#[test]
fn includes_and_filter_signs_change_and_unsaved_changes_are_asked_about() {
    let mut app = in_settings();
    assert!(!view(&app).dirty());
    send(&mut app, Message::Include(Row::IncludeHome, true));
    assert!(view(&app).edited.include_home);
    send(&mut app, Message::FilterSign(0, false));
    assert!(view(&app).edited.filters[0].starts_with("- "));
    // Leaving asks.
    send(&mut app, Message::Back);
    assert_eq!(app.dialog, Some(Dialog::Unsaved));
    assert_eq!(app.page, Page::Settings);
    send(&mut app, Message::DialogDiscard);
    assert_eq!(app.page, Page::List);
    assert!(!view(&app).dirty());
}

#[test]
fn picked_paths_and_typed_patterns_go_on_top() {
    let mut app = in_settings();
    let before = view(&app).edited.filters.len();
    send(
        &mut app,
        Message::Picked(Some(("/home/user1/Projects".to_owned(), true))),
    );
    assert_eq!(view(&app).edited.filters[0], "+ /home/user1/Projects/***");
    assert_eq!(view(&app).current(), Row::Filter(0));
    send(&mut app, Message::Picked(None));
    assert_eq!(
        view(&app).edited.filters.len(),
        before + 1,
        "cancelled: nothing"
    );
    send(&mut app, Message::AddPattern);
    send(&mut app, Message::DialogText("   ".to_owned()));
    send(&mut app, Message::DialogConfirm);
    assert!(
        matches!(&app.dialog, Some(Dialog::AddPattern { error: Some(_), .. })),
        "a blank one stays, with why"
    );
    send(&mut app, Message::DialogText("*.iso".to_owned()));
    send(&mut app, Message::DialogConfirm);
    assert!(app.dialog.is_none());
    assert_eq!(view(&app).edited.filters[0], "- *.iso");
    send(&mut app, Message::MoveFilter(false));
    assert_eq!(view(&app).edited.filters[1], "- *.iso");
    send(&mut app, Message::RemoveFilter);
    assert_eq!(view(&app).edited.filters.len(), before + 1);
}

#[test]
fn save_checks_first_then_writes_then_reads_back_and_lists() {
    let mut app = in_settings();
    send(&mut app, Message::Save);
    assert!(!app.saving_settings, "nothing to save");
    send(&mut app, Message::Include(Row::IncludeHome, true));
    send(&mut app, Message::Save);
    assert!(app.saving_settings);
    send(&mut app, Message::SettingsWritten(Ok(String::new())));
    assert!(!app.saving_settings);
    assert_eq!(app.status, Some(Status::Info(fl!("settings-saved"))));
    assert!(matches!(app.settings, SettingsLoad::Loading));
    assert!(app.loading);
}

#[test]
fn a_refused_write_keeps_the_edits() {
    let mut app = in_settings();
    send(&mut app, Message::Include(Row::IncludeHome, true));
    send(&mut app, Message::Save);
    send(
        &mut app,
        Message::SettingsWritten(Err(CliError::from(apsis_core::Error::ConfigChanged))),
    );
    assert!(matches!(app.status, Some(Status::Error(..))));
    assert!(view(&app).dirty());
}

#[test]
fn shortcuts_map_keys_and_leave_typing_alone() {
    let key = |c: &str| Key::Character(c.into());
    assert_eq!(
        shortcut_for(&key("n"), Modifiers::CTRL, false),
        Some(Shortcut::Create)
    );
    assert_eq!(
        shortcut_for(&key(","), Modifiers::CTRL, false),
        Some(Shortcut::Settings)
    );
    assert_eq!(
        shortcut_for(&Key::Named(Named::F5), Modifiers::empty(), false),
        Some(Shortcut::Refresh)
    );
    assert_eq!(
        shortcut_for(&Key::Named(Named::Delete), Modifiers::empty(), false),
        Some(Shortcut::Delete)
    );
    // Plain letters do nothing: there's no command line any more.
    assert_eq!(shortcut_for(&key("c"), Modifiers::empty(), false), None);
    // A key a text field took is left to it, but Esc always counts.
    assert_eq!(shortcut_for(&key("n"), Modifiers::CTRL, true), None);
    assert_eq!(
        shortcut_for(&Key::Named(Named::Escape), Modifiers::empty(), true),
        Some(Shortcut::Escape)
    );
    // Shortcuts wait while a dialog is open.
    let mut app = window();
    app.dialog = Some(Dialog::Unsaved);
    send(&mut app, Message::Shortcut(Shortcut::Refresh));
    assert!(!app.loading);
    send(&mut app, Message::Shortcut(Shortcut::Escape));
    assert!(app.dialog.is_none());
}

#[test]
fn the_misc_tab_saves_apsiss_own_settings_at_once() {
    let mut app = in_settings();
    send(&mut app, Message::ShowLabel(true));
    assert!(view(&app).backend.show_label);
    send(&mut app, Message::RemindDays(3));
    assert_eq!(view(&app).backend.remind_days, 3);
    // Not part of the config: nothing to Save.
    assert!(!view(&app).dirty());
}

#[test]
fn creating_shows_the_elapsed_time_until_there_is_a_percent() {
    let mut app = window();
    let started = Instant::now();
    app.running = Some(Operation::Create(String::new()));
    app.run_started = Some(started);
    let (text, fraction) = app.create_progress_at(started + Duration::from_secs(68));
    assert!(text.contains("1m 08s"), "{text}");
    assert_eq!(fraction, None);
    // 0% is still no number.
    send(
        &mut app,
        Message::Progress(Progress {
            percent: Some(0.0),
            eta_seconds: None,
            text: String::new(),
        }),
    );
    assert_eq!(app.create_progress_at(started).1, None);
}

/// Lays `element` out at `size` with a headless renderer; `None` if there's none here. Runs
/// only with `APSIS_LAYOUT_TEST=1`: it measures real text, so it depends on the fonts
/// installed.
fn layout(
    mut element: Element<'_, Message>,
    size: Size,
) -> Option<cosmic::iced::core::layout::Node> {
    use cosmic::iced::core::layout::Limits as LayoutLimits;
    use cosmic::iced::core::renderer::Headless;
    use cosmic::iced::core::widget::Tree;
    if std::env::var_os("APSIS_LAYOUT_TEST").is_none_or(|v| v != "1") {
        eprintln!("layout test skipped; set APSIS_LAYOUT_TEST=1 to run it");
        return None;
    }
    let renderer = cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
        cosmic::font::default(),
        14.0.into(),
        Some("tiny-skia"),
    ))?;
    let mut tree = Tree::new(&element);
    let limits = LayoutLimits::new(Size::ZERO, size);
    Some(
        element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits),
    )
}

#[test]
fn the_window_and_every_settings_tab_fit_the_smallest_window() {
    let mut app = window();
    let Listing::Loaded(list) = &mut app.listing else {
        panic!()
    };
    list.usage = Some(DiskUsage {
        total: 1_000_203_837_440,
        used: 950_000_000_000,
        free: 50_203_837_440,
    });
    app.running = Some(Operation::Create(String::new()));
    app.job = Some(job(JobKind::Create, JobState::Running, NEWEST));
    let Some(node) = layout(app.window_view(), WINDOW_MIN_SIZE) else {
        return;
    };
    assert!(node.bounds().height <= WINDOW_MIN_SIZE.height + 0.5);
    // Toolbar, list, divider, status area: the list keeps some rows.
    let parts = node.children()[0].children();
    assert!(
        parts[1].bounds().height >= 80.0,
        "list {:?}",
        parts[1].bounds()
    );
    let mut app = in_settings();
    for entity in app.tabs.iter().collect::<Vec<_>>() {
        app.tabs.activate(entity);
        let node = layout(app.window_view(), WINDOW_MIN_SIZE).unwrap();
        assert!(
            node.bounds().width <= WINDOW_MIN_SIZE.width + 0.5,
            "{:?}: {:?}",
            app.active_section(),
            node.bounds()
        );
    }
}

#[test]
fn the_popup_fits_its_width() {
    let mut app = model();
    app.on_listed(Ok(fixture()));
    let Some(node) = layout(app.popup_view(), Size::new(POPUP_WIDTH, 1000.0)) else {
        return;
    };
    assert!(node.bounds().width <= POPUP_WIDTH + 0.5);
    assert!(node.bounds().height < 520.0, "{}", node.bounds().height);
}

/// Screenshots of the real views, for looking at them without a COSMIC session: set
/// `APSIS_SCREENSHOTS` to a folder and each view is written there as `<name>.rgba` (width and
/// height as two little-endian `u32`s, then RGBA bytes). Skipped otherwise.
#[test]
fn screenshots() {
    use cosmic::iced::core::layout::{Layout, Limits as LayoutLimits};
    use cosmic::iced::core::renderer::{Headless, Style};
    use cosmic::iced::core::widget::Tree;
    use cosmic::iced::core::{Rectangle, mouse};

    let Some(dir) = std::env::var_os("APSIS_SCREENSHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    // `APSIS_SCREENSHOTS_MONO=1`: the monospace face, as a theme with a monospace interface
    // font would show it (wider text).
    let font = if std::env::var_os("APSIS_SCREENSHOTS_MONO").is_some_and(|v| v == "1") {
        cosmic::font::mono()
    } else {
        cosmic::font::default()
    };
    let new_renderer = || {
        cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
            font,
            14.0.into(),
            Some("tiny-skia"),
        ))
    };
    if new_renderer().is_none() {
        eprintln!("no headless renderer here; skipped");
        return;
    }
    // A fresh renderer each time: a headless one keeps what was drawn before.
    let shoot = |name: &str, dark: bool, mut element: Element<'_, Message>, size: Size| {
        let mut renderer = new_renderer().unwrap();
        let theme = if dark {
            cosmic::Theme::dark()
        } else {
            cosmic::Theme::light()
        };
        let container = theme.cosmic().background(false);
        let background: cosmic::iced::Color = container.base.into();
        let text_color: cosmic::iced::Color = container.on.into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &LayoutLimits::new(Size::ZERO, size),
        );
        let bounds = Rectangle::with_size(size);
        element.as_widget().draw(
            &tree,
            &mut renderer,
            &theme,
            &Style {
                text_color,
                icon_color: text_color,
                scale_factor: 1.0,
            },
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &bounds,
        );
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a window"
        )]
        let (width, height) = (size.width as u32, size.height as u32);
        let pixels = renderer.screenshot(Size::new(width, height), 1.0, background);
        let mut out = Vec::with_capacity(8 + pixels.len());
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.extend(pixels);
        let theme_name = if dark { "dark" } else { "light" };
        std::fs::write(dir.join(format!("{name}-{theme_name}.rgba")), out).unwrap();
    };
    let window_size = Size::new(720.0, 520.0);

    let mut app = window();
    let Listing::Loaded(list) = &mut app.listing else {
        panic!()
    };
    list.usage = Some(DiskUsage {
        total: 1_000_203_837_440,
        used: 400_000_000_000,
        free: 600_203_837_440,
    });
    click(&mut app, 0, Modifiers::empty());
    for dark in [true, false] {
        shoot("window", dark, app.window_view(), window_size);
    }

    let mut running = window();
    let Listing::Loaded(list) = &mut running.listing else {
        panic!()
    };
    list.usage = Some(DiskUsage {
        total: 1_000_203_837_440,
        used: 950_000_000_000,
        free: 50_203_837_440,
    });
    running.running = Some(Operation::Create("before driver update".to_owned()));
    let mut create = job(JobKind::Create, JobState::Running, "2026-09-30_14-02-11");
    create.percent = Some(58.0);
    create.eta_seconds = Some(192);
    running.job = Some(create);
    running.progress = Some(Progress {
        percent: Some(58.0),
        eta_seconds: Some(192),
        text: String::new(),
    });
    shoot("window-creating", true, running.window_view(), window_size);

    let mut settings = in_settings();
    send(
        &mut settings,
        Message::Picked(Some(("/home/user1/Projects".to_owned(), true))),
    );
    for (index, entity) in settings
        .tabs
        .iter()
        .collect::<Vec<_>>()
        .into_iter()
        .enumerate()
    {
        settings.tabs.activate(entity);
        let name = format!("settings-{index}-{:?}", settings.active_section()).to_lowercase();
        shoot(&name, true, settings.window_view(), window_size);
    }

    // As a converted 0.3 config shows it: the notes card above one filter.
    let mut converted = in_settings();
    if let SettingsLoad::Ready(view) = &mut converted.settings {
        view.info.notes = vec![
            "settings from Apsis 0.3 (not saved yet; Save keeps them)".to_owned(),
            "  /root    included".to_owned(),
            "  /home    included (every home was \"everything\")".to_owned(),
            "  filters  1".to_owned(),
        ];
        view.edited.filters = vec!["- /var/lib/libvirt/**".to_owned()];
    }
    let filters = converted.tabs.iter().nth(2).unwrap();
    converted.tabs.activate(filters);
    shoot(
        "settings-filters-converted",
        true,
        converted.window_view(),
        window_size,
    );

    let mut dialog = window();
    click(&mut dialog, 0, Modifiers::empty());
    click(&mut dialog, 1, Modifiers::CTRL);
    send(&mut dialog, Message::DeleteClicked);
    shoot(
        "dialog-delete",
        true,
        dialog.dialog_view().unwrap(),
        Size::new(480.0, 320.0),
    );

    let mut popup = model();
    popup.on_listed(Ok(fixture()));
    let Listing::Loaded(list) = &mut popup.listing else {
        panic!()
    };
    list.usage = Some(DiskUsage {
        total: 1_000_203_837_440,
        used: 400_000_000_000,
        free: 600_203_837_440,
    });
    for dark in [true, false] {
        shoot(
            "popup",
            dark,
            popup.popup_view(),
            Size::new(POPUP_WIDTH, 460.0),
        );
    }
}

// ---- Restore (0.5.0, PLAN 6b.8 and 6b.12) ----

mod restore {
    use apsis_core::restore::apsis::InSnapshot;
    use apsis_core::restore::dialog::Dialog as Check;
    use apsis_core::restore::esp::CheckFailure;
    use apsis_core::restore::filter::Home;
    use apsis_core::restore::refusal::Refusal;
    use apsis_core::restore::state::{Outcome, RestoreResult};

    use super::*;

    fn check() -> Check {
        Check {
            refusal: None,
            has_home: true,
            has_root: true,
            old_format: false,
            apsis: InSnapshot::Current,
        }
    }

    /// A window with the newest snapshot selected and its restore dialog open.
    fn with_dialog() -> AppModel {
        let mut app = window();
        click(&mut app, 0, Modifiers::empty());
        send(&mut app, Message::RestoreClicked);
        assert_eq!(app.checking.as_deref(), Some(NEWEST));
        send(&mut app, Message::Checked(NEWEST.to_owned(), Ok(check())));
        app
    }

    fn restore_op() -> Operation {
        Operation::Restore {
            snapshot: NEWEST.to_owned(),
            restore_home: false,
            safety_snapshot: true,
        }
    }

    /// A window whose restore is ready at the prompt.
    fn ready() -> AppModel {
        let mut app = with_dialog();
        send(&mut app, Message::DialogConfirm);
        send(&mut app, Message::Finished(restore_op(), Ok(())));
        app
    }

    #[test]
    fn restore_is_enabled_only_with_one_real_snapshot_and_no_job() {
        let mut app = window();
        assert!(!app.can_restore(), "nothing selected");
        click(&mut app, 0, Modifiers::empty());
        assert!(app.can_restore());
        click(&mut app, 1, Modifiers::CTRL);
        assert!(!app.can_restore(), "two selected");
        click(&mut app, 5, Modifiers::empty());
        assert!(!app.can_restore(), "a leftover");
        click(&mut app, 0, Modifiers::empty());
        app.job = Some(job(JobKind::Create, JobState::Running, NEWEST));
        assert!(!app.can_restore(), "a job runs");
        app.job = None;
        app.disk_seen = Some(false);
        assert!(!app.can_restore(), "the disk is gone");
    }

    /// No key opens the dialog (owner, 2026-09-30); Enter isn't a shortcut at all.
    #[test]
    fn no_key_opens_restore() {
        let mut app = window();
        click(&mut app, 0, Modifiers::empty());
        for shortcut in [
            Shortcut::Create,
            Shortcut::Delete,
            Shortcut::Refresh,
            Shortcut::Settings,
            Shortcut::SelectAll,
            Shortcut::Up,
            Shortcut::Down,
            Shortcut::Escape,
        ] {
            send(&mut app, Message::Shortcut(shortcut));
            assert!(
                !matches!(app.dialog, Some(Dialog::Restore { .. })),
                "{shortcut:?} opened the restore dialog"
            );
            app.dialog = None;
        }
        use cosmic::iced::keyboard::Key;
        use cosmic::iced::keyboard::key::Named;
        assert_eq!(
            shortcut_for(&Key::Named(Named::Enter), Modifiers::empty(), false),
            None
        );
    }

    #[test]
    fn restore_asks_the_helper_first_and_opens_with_the_defaults() {
        let app = with_dialog();
        assert_eq!(
            app.dialog,
            Some(Dialog::Restore {
                snapshot: NEWEST.to_owned(),
                check: check(),
                restore_home: false,
                safety_snapshot: true,
            })
        );
        assert!(app.checking.is_none());
        // While the check runs the toolbar waits.
        let mut app = window();
        click(&mut app, 0, Modifiers::empty());
        send(&mut app, Message::RestoreClicked);
        assert!(!app.can_restore() && !app.can_create());
        // A refusal opens the other dialog, not dropped.
        send(
            &mut app,
            Message::Checked(
                NEWEST.to_owned(),
                Ok(Check {
                    refusal: Some(Refusal::OtherInstallation),
                    ..check()
                }),
            ),
        );
        assert_eq!(
            app.dialog,
            Some(Dialog::Refused {
                snapshot: NEWEST.to_owned(),
                refusal: Refusal::OtherInstallation,
                dropped: false,
            })
        );
        // The helper couldn't be asked: the status line says why, no dialog.
        let mut app = window();
        click(&mut app, 0, Modifiers::empty());
        send(&mut app, Message::RestoreClicked);
        send(
            &mut app,
            Message::Checked(NEWEST.to_owned(), Err(CliError::NotAuthorized)),
        );
        assert!(app.dialog.is_none());
        assert!(matches!(app.status, Some(Status::Error(..))));
    }

    #[test]
    fn home_radios_and_the_lost_for_good_line_follow_the_choices() {
        let mut app = with_dialog();
        let Some(Dialog::Restore { check: found, .. }) = &app.dialog else {
            panic!()
        };
        assert!(found.has_home, "the radios show");
        assert!(!app.dialog.as_ref().unwrap().lost_for_good());
        send(&mut app, Message::RestoreHome(1));
        assert!(
            !app.dialog.as_ref().unwrap().lost_for_good(),
            "safety is on"
        );
        send(&mut app, Message::RestoreSafety(false));
        assert!(app.dialog.as_ref().unwrap().lost_for_good());
        send(&mut app, Message::RestoreHome(0));
        assert!(!app.dialog.as_ref().unwrap().lost_for_good());
        // Without home in the snapshot the choice is hidden and stays "keep".
        let mut app = window();
        click(&mut app, 0, Modifiers::empty());
        send(&mut app, Message::RestoreClicked);
        send(
            &mut app,
            Message::Checked(
                NEWEST.to_owned(),
                Ok(Check {
                    has_home: false,
                    ..check()
                }),
            ),
        );
        send(&mut app, Message::RestoreHome(1));
        assert!(matches!(
            app.dialog,
            Some(Dialog::Restore {
                restore_home: false,
                ..
            })
        ));
    }

    #[test]
    fn the_dialogs_extra_line_names_the_format_or_the_apsis_in_the_snapshot() {
        let line = |check: Check| {
            Dialog::Restore {
                snapshot: NEWEST.to_owned(),
                check,
                restore_home: false,
                safety_snapshot: true,
            }
            .extra_lines()
        };
        assert!(line(check()).is_empty());
        let old = line(Check {
            old_format: true,
            ..check()
        });
        assert_eq!(old, [fl!("restore-old-format")]);
        assert_eq!(
            line(Check {
                apsis: InSnapshot::NotInstalled,
                ..check()
            }),
            [fl!("restore-no-apsis")]
        );
        assert_eq!(
            line(Check {
                apsis: InSnapshot::NoRestore {
                    version: "0.4.2".to_owned()
                },
                ..check()
            }),
            [fl!("restore-apsis-no-restore", version = "0.4.2")]
        );
        assert_eq!(
            line(Check {
                apsis: InSnapshot::OldSettings {
                    version: "0.3.1".to_owned()
                },
                ..check()
            }),
            [fl!("restore-apsis-old-settings", version = "0.3.1")]
        );
        // Both at once: both lines, the format first.
        let both = line(Check {
            old_format: true,
            apsis: InSnapshot::NotInstalled,
            has_home: false,
            ..check()
        });
        assert_eq!(both.len(), 3, "{both:?}");
        assert_eq!(both[0], fl!("restore-no-home"));
    }

    #[test]
    fn confirming_runs_the_restore_and_ready_opens_the_prompt() {
        let mut app = with_dialog();
        send(&mut app, Message::DialogConfirm);
        assert_eq!(app.running, Some(restore_op()));
        assert!(app.dialog.is_none());
        // Preparing: the status line before any progress, then with the safety snapshot's.
        assert_eq!(app.restore_progress().0, fl!("preparing-checking"));
        send(
            &mut app,
            Message::Progress(Progress {
                percent: Some(42.0),
                eta_seconds: Some(180),
                text: String::new(),
            }),
        );
        let (text, fraction) = app.restore_progress();
        assert!(
            text.starts_with("Preparing restore · safety snapshot · 42%"),
            "{text}"
        );
        assert!((fraction.unwrap() - 0.42).abs() < 1e-6);
        send(&mut app, Message::Finished(restore_op(), Ok(())));
        assert!(app.running.is_none());
        assert_eq!(app.ready.as_deref(), Some(NEWEST));
        assert_eq!(
            app.dialog,
            Some(Dialog::Ready {
                snapshot: NEWEST.to_owned()
            })
        );
        assert_eq!(app.restore_progress(), (fl!("preparing-ready"), Some(1.0)));
        assert!(app.loading, "lists again: the safety snapshot is new");
        assert!(
            !app.can_create() && !app.can_restore(),
            "the plan holds writes off"
        );
    }

    #[test]
    fn a_refused_or_failed_or_stopped_preparation_says_so() {
        let mut app = with_dialog();
        send(&mut app, Message::DialogConfirm);
        send(
            &mut app,
            Message::Finished(
                restore_op(),
                Err(CliError::RestoreRefused(Refusal::SystemSpace {
                    needs: 6_000_000_000,
                    free: 3_000_000_000,
                })),
            ),
        );
        assert!(matches!(
            app.dialog,
            Some(Dialog::Refused { dropped: false, .. })
        ));
        assert!(app.ready.is_none());
        let mut app = with_dialog();
        send(&mut app, Message::DialogConfirm);
        send(
            &mut app,
            Message::Finished(restore_op(), Err(CliError::Stopped)),
        );
        assert_eq!(app.status, Some(Status::Info(fl!("restore-stopped"))));
        assert!(app.loading, "a finished safety snapshot may be there");
        let mut app = with_dialog();
        send(&mut app, Message::DialogConfirm);
        send(
            &mut app,
            Message::Finished(restore_op(), Err(CliError::Other("rsync died".to_owned()))),
        );
        assert_eq!(
            app.status,
            Some(Status::Error(
                fl!("restore-failed", reason = "rsync died"),
                None
            ))
        );
    }

    #[test]
    fn the_ready_prompt_has_two_answers_and_esc_cancels() {
        let mut app = ready();
        send(&mut app, Message::Shortcut(Shortcut::Escape));
        assert!(app.dialog.is_none());
        assert!(app.cancelling, "Esc is Cancel restore");
        send(&mut app, Message::CancelAnswered(Ok(())));
        assert!(app.ready.is_none() && !app.cancelling);
        assert_eq!(app.status, Some(Status::Info(fl!("restore-cancelled"))));
        assert!(app.loading, "lists again: the safety snapshot stays");
        app.on_listed(Ok(fixture()));
        assert!(app.can_create(), "writes are free again");
        // Restart now: the helper arms and restarts; nothing more to show but the line.
        let mut app = ready();
        send(&mut app, Message::RestartNow);
        assert!(app.dialog.is_none() && app.restarting);
        send(&mut app, Message::RestartAnswered(Ok(())));
        assert_eq!(app.status, Some(Status::Info(fl!("restarting"))));
        assert!(app.ready.is_none());
        // Too old, or gone: one line, the plan is over.
        let mut app = ready();
        send(&mut app, Message::RestartNow);
        send(
            &mut app,
            Message::RestartAnswered(Err(CliError::Other(
                apsis_core::restore::plan::TOO_OLD.to_owned(),
            ))),
        );
        assert_eq!(
            app.status,
            Some(Status::Error(fl!("restore-too-old"), None))
        );
        assert!(app.ready.is_none());
        let mut app = ready();
        send(&mut app, Message::CancelRestore);
        send(
            &mut app,
            Message::CancelAnswered(Err(CliError::Other(
                apsis_core::restore::plan::GONE.to_owned(),
            ))),
        );
        assert_eq!(app.status, Some(Status::Error(fl!("restore-gone"), None)));
        assert!(app.ready.is_none());
    }

    /// A refused "Restart now" drops the plan (owner, 2026-10-02): the dialog says so, and
    /// the status line stops saying `ready` (check 4, 2026-10-03).
    #[test]
    fn a_refused_restart_now_says_the_plan_was_dropped() {
        let mut app = ready();
        assert_eq!(app.status, Some(Status::Info(fl!("preparing-ready"))));
        send(&mut app, Message::RestartNow);
        send(
            &mut app,
            Message::RestartAnswered(Err(CliError::RestoreRefused(Refusal::BootFiles(
                CheckFailure::NoEntry,
            )))),
        );
        assert_eq!(
            app.dialog,
            Some(Dialog::Refused {
                snapshot: NEWEST.to_owned(),
                refusal: Refusal::BootFiles(CheckFailure::NoEntry),
                dropped: true,
            })
        );
        assert!(app.ready.is_none());
        let lines = app.dialog.as_ref().unwrap().refusal_lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[2], fl!("refused-dropped"));
        // Under the dialog and after Close: the plan is over, and the line says so.
        assert_eq!(app.status, Some(Status::Info(fl!("restore-stopped"))));
        assert!(!app.restarting);
        send(&mut app, Message::DialogCancel);
        assert!(app.dialog.is_none());
        assert_eq!(app.status, Some(Status::Info(fl!("restore-stopped"))));
        assert!(app.ready.is_none() && app.running.is_none() && app.job.is_none());
    }

    #[test]
    fn every_refusal_has_its_two_lines() {
        let all = [
            Refusal::NotUefi,
            Refusal::NotKernelstub,
            Refusal::BootPartition,
            Refusal::RootFilesystem {
                fstype: "btrfs".to_owned(),
            },
            Refusal::RootDevice,
            Refusal::SplitSystem,
            Refusal::OtherInstallation,
            Refusal::Unreadable(apsis_core::restore::refusal::Unreadable::NoInfo),
            Refusal::KernelIncomplete,
            Refusal::PendingUpdate,
            Refusal::PopUpgradePending,
            Refusal::CrypttabDiffers,
            Refusal::BackupSpace {
                needs: 14_000_000_000,
                free: 9_000_000_000,
            },
            Refusal::SystemSpace { needs: 6, free: 3 },
            Refusal::BootSpace { needs: 6, free: 3 },
            Refusal::SizeUnknown,
            Refusal::BootFiles(CheckFailure::NoEntry),
        ];
        for refusal in all {
            let dialog = Dialog::Refused {
                snapshot: NEWEST.to_owned(),
                refusal: refusal.clone(),
                dropped: false,
            };
            let lines = dialog.refusal_lines();
            assert_eq!(lines.len(), 2, "{refusal:?}: {lines:?}");
            assert!(
                lines.iter().all(|l| !l.is_empty() && !l.contains('{')),
                "{lines:?}"
            );
        }
        let space = Dialog::Refused {
            snapshot: NEWEST.to_owned(),
            refusal: Refusal::SystemSpace {
                needs: 6_000_000_000,
                free: 3_000_000_000,
            },
            dropped: false,
        }
        .refusal_lines();
        let (needs, free) = (
            apsis_core::status::size(6_000_000_000),
            apsis_core::status::size(3_000_000_000),
        );
        assert!(
            space[0].contains(&needs) && space[0].contains(&free),
            "{space:?}"
        );
    }

    #[test]
    fn stop_during_the_preparation_asks_and_the_stopped_restore_says_so() {
        let mut app = with_dialog();
        send(&mut app, Message::DialogConfirm);
        send(
            &mut app,
            Message::Job(JobEvent::Changed(job(
                JobKind::Restore,
                JobState::Running,
                NEWEST,
            ))),
        );
        assert_eq!(app.stoppable().as_deref(), Some(NEWEST));
        send(&mut app, Message::StopClicked);
        assert_eq!(
            app.dialog,
            Some(Dialog::StopRestore {
                snapshot: NEWEST.to_owned()
            })
        );
        send(&mut app, Message::DialogConfirm);
        assert_eq!(app.status, Some(Status::Info(fl!("stopping"))));
        send(
            &mut app,
            Message::Finished(restore_op(), Err(CliError::Stopped)),
        );
        assert_eq!(app.status, Some(Status::Info(fl!("restore-stopped"))));
    }

    #[test]
    fn another_windows_restore_holds_the_toolbar_and_its_end_lists() {
        let mut app = window();
        let mut running = job(JobKind::Restore, JobState::Running, NEWEST);
        running.percent = Some(100.0);
        send(&mut app, Message::Job(JobEvent::Changed(running)));
        assert!(app.active_job().is_some());
        assert!(!app.can_create() && !app.can_restore());
        assert_eq!(app.restore_progress().0, fl!("restore-ready-elsewhere"));
        send(
            &mut app,
            Message::Job(JobEvent::Changed(job(
                JobKind::Restore,
                JobState::Stopped,
                NEWEST,
            ))),
        );
        assert!(app.active_job().is_none());
        assert!(app.loading, "a safety snapshot may have appeared");
        assert_eq!(app.status, None, "the other window says what happened");
    }

    #[test]
    fn restore_results_show_on_the_status_line_and_restore_again_reopens() {
        let mut app = window();
        let done = RestoreResult {
            state: apsis_core::restore::state::ResultState::Ended(Outcome::Done),
            snapshot: Some(NEWEST.to_owned()),
            message: String::new(),
            when: Some(1_790_000_000),
            home: Some(Home::Keep),
            safety_snapshot: None,
        };
        send(&mut app, Message::RestoreResultRead(Ok(done)));
        let shown = app.result_text().expect("a result line");
        assert_eq!(shown.0, fl!("result-done", date = "2026-09-25 11:28"));
        assert!(!app.restore_again_available());
        let failed = RestoreResult {
            state: apsis_core::restore::state::ResultState::Ended(Outcome::Failed),
            snapshot: Some(NEWEST.to_owned()),
            message: "the backup disk was disconnected".to_owned(),
            when: None,
            home: Some(Home::Keep),
            safety_snapshot: None,
        };
        send(&mut app, Message::RestoreResultRead(Ok(failed)));
        let (line, tooltip) = app.result_text().unwrap();
        assert!(line.starts_with("Restore incomplete"), "{line}");
        assert!(line.ends_with(fl!("result-what-disk").as_str()), "{line}");
        assert!(tooltip.contains("disconnected"));
        assert!(app.restore_again_available());
        send(&mut app, Message::RestoreAgain);
        assert_eq!(selected(&app), [NEWEST]);
        assert_eq!(app.checking.as_deref(), Some(NEWEST));
        // Nothing to show: no line. A ready plan elsewhere: no result line either.
        let mut app = window();
        send(
            &mut app,
            Message::RestoreResultRead(Ok(RestoreResult::none())),
        );
        assert!(app.result_text().is_none());
        send(
            &mut app,
            Message::RestoreResultRead(Ok(RestoreResult::ready(NEWEST))),
        );
        assert!(app.result_text().is_none());
        // Not started: what to do from the message.
        let not_started = RestoreResult {
            state: apsis_core::restore::state::ResultState::Ended(Outcome::NotStarted),
            snapshot: Some(NEWEST.to_owned()),
            message: "the backup disk 0000 wasn't found within 60 seconds".to_owned(),
            when: Some(1_790_000_000),
            home: Some(Home::Keep),
            safety_snapshot: None,
        };
        send(&mut app, Message::RestoreResultRead(Ok(not_started)));
        let (line, _) = app.result_text().unwrap();
        assert_eq!(
            line,
            fl!("result-not-started", what = fl!("result-what-disk"))
        );
    }

    /// A window that read this result when it opened.
    fn with_result(outcome: Outcome, message: &str, home: Home, safety: Option<&str>) -> AppModel {
        let mut app = window();
        send(
            &mut app,
            Message::RestoreResultRead(Ok(RestoreResult {
                state: apsis_core::restore::state::ResultState::Ended(outcome),
                snapshot: Some(NEWEST.to_owned()),
                message: message.to_owned(),
                when: Some(1_790_000_000),
                home: Some(home),
                safety_snapshot: safety.map(str::to_owned),
            })),
        );
        app
    }

    /// The line and the tooltip of that result.
    fn shown(
        outcome: Outcome,
        message: &str,
        home: Home,
        safety: Option<&str>,
    ) -> (String, String) {
        with_result(outcome, message, home, safety)
            .result_text()
            .expect("a result line")
    }

    /// The colour of that result's phrase, if its line has one.
    fn tone(outcome: Outcome, message: &str) -> Option<Tone> {
        with_result(outcome, message, Home::Keep, None)
            .result_phrase()
            .map(|phrase| phrase.tone)
    }

    /// The result's tooltip says what happened to home and names the safety snapshot, from
    /// `result.json` through `RestoreResult` (fix 6, 2026-10-04): the four combinations.
    #[test]
    fn the_result_tooltip_has_home_and_the_safety_snapshot_from_the_result() {
        for (home, safety, tooltip) in [
            (
                Home::Keep,
                Some(SECOND),
                "Restored from snapshot 2026-09-25_11-28-53. Home folders were kept. Safety \
                 snapshot: 2026-09-23_08-33-55.",
            ),
            (
                Home::Keep,
                None,
                "Restored from snapshot 2026-09-25_11-28-53. Home folders were kept. Safety \
                 snapshot: none.",
            ),
            (
                Home::Restore,
                Some(SECOND),
                "Restored from snapshot 2026-09-25_11-28-53. Home folders were restored too. \
                 Safety snapshot: 2026-09-23_08-33-55.",
            ),
            (
                Home::Restore,
                None,
                "Restored from snapshot 2026-09-25_11-28-53. Home folders were restored too. \
                 Safety snapshot: none.",
            ),
        ] {
            let (line, shown) = shown(Outcome::Done, "", home, safety);
            assert_eq!(line, "System restored to 2026-09-25 11:28");
            assert_eq!(shown, tooltip);
        }
        assert_eq!(tone(Outcome::Done, ""), None);
    }

    /// Fix 5 (check 6): `problems` isn't shown as `done`. Its line says that some files
    /// weren't restored when the helper names the log, and that a cleanup step failed for
    /// anything else; the tooltip has the helper's own words, which name the log.
    #[test]
    fn problems_has_a_line_and_a_tooltip_of_its_own() {
        const FILES: &str =
            "some files couldn't be written or deleted; see /var/lib/apsis/restore/rsync-log";
        const KERNEL: &str =
            "the kernel from before the restore couldn't be removed (Read-only file system)";
        let (line, tooltip) = shown(Outcome::Problems, FILES, Home::Keep, Some(SECOND));
        assert_eq!(
            line,
            "Restore finished · some files were not restored · point here for details"
        );
        assert_eq!(
            tooltip,
            "Restored from snapshot 2026-09-25_11-28-53. Home folders were kept. Safety \
             snapshot: 2026-09-23_08-33-55. What went wrong: some files couldn't be written or \
             deleted; see /var/lib/apsis/restore/rsync-log"
        );
        let (line, tooltip) = shown(Outcome::Problems, KERNEL, Home::Restore, None);
        assert_eq!(
            line,
            "Restore finished · a cleanup step failed · point here for details"
        );
        assert_eq!(
            tooltip,
            "Restored from snapshot 2026-09-25_11-28-53. Home folders were restored too. Safety \
             snapshot: none. What went wrong: the kernel from before the restore couldn't be \
             removed (Read-only file system)"
        );
        // Both at once: the files come first.
        let (line, _) = shown(
            Outcome::Problems,
            &format!("{FILES}; {KERNEL}"),
            Home::Keep,
            None,
        );
        assert!(line.contains("some files were not restored"), "{line}");
        assert_eq!(tone(Outcome::Problems, FILES), Some(Tone::Warning));
        assert_eq!(tone(Outcome::Problems, KERNEL), Some(Tone::Warning));
    }

    /// `boot-broken` isn't shown as `boot-kept` (triage, 2026-10-03): nothing was put back,
    /// so the line says the computer may not start next time, in the error colour, and both
    /// the line and the tooltip send the reader to the README before a restart.
    #[test]
    fn boot_broken_has_a_line_and_a_tooltip_of_its_own() {
        const WHY: &str = "the boot refresh failed (kernelstub exited 1); the boot files from \
                           before couldn't be put back (initrd.img doesn't match)";
        let (line, tooltip) = shown(Outcome::BootBroken, WHY, Home::Keep, Some(SECOND));
        assert_eq!(
            line,
            "Restore finished · the computer may not start next time · read the README before \
             restarting"
        );
        assert_eq!(
            tooltip,
            "Restored to 2026-09-25 11:28, but the boot files are not in order and the \
             computer may not start the next time you restart. Before you restart, read \"If \
             a restore goes wrong\" in Apsis's README. What went wrong: the boot refresh \
             failed (kernelstub exited 1); the boot files from before couldn't be put back \
             (initrd.img doesn't match)"
        );
        assert!(!line.contains("previous kernel"), "{line}");
        assert!(!tooltip.contains("kernel update"), "{tooltip}");
        assert_eq!(tone(Outcome::BootBroken, WHY), Some(Tone::Error));
    }

    /// The `boot-kept` tooltip (check 7): one wording for a refresh that failed, a check
    /// that failed and boot files that were never touched. It doesn't say "didn't check
    /// out" or "put back".
    #[test]
    fn the_boot_kept_tooltip_covers_every_way_the_boot_files_were_kept() {
        for why in [
            "the boot refresh failed (kernelstub exited 1); the boot files from before were \
             put back",
            "the new boot files didn't check out (vmlinuz.efi isn't the kernel /boot/vmlinuz \
             links to); the boot files from before were put back",
            "the boot files couldn't be backed up (No space left on device), so they were \
             left as they are",
        ] {
            let (line, tooltip) = shown(Outcome::BootKept, why, Home::Keep, Some(SECOND));
            assert_eq!(
                line,
                "System restored · still boots the previous kernel · see README"
            );
            assert_eq!(
                tooltip,
                "Restored to 2026-09-25 11:28. The boot files couldn't be refreshed, so the \
                 computer keeps the ones it had and runs the kernel from before the restore. \
                 The next kernel update should set this right. See \"If a restore goes \
                 wrong\" in Apsis's README."
            );
            assert_eq!(tone(Outcome::BootKept, why), Some(Tone::Warning));
        }
    }

    /// The `failed` tooltip offers the safety snapshot only if one was taken (the window
    /// knows since fix 6).
    #[test]
    fn the_failed_tooltip_offers_the_safety_snapshot_only_if_there_is_one() {
        const WHY: &str = "the backup disk was disconnected";
        let (line, tooltip) = shown(Outcome::Failed, WHY, Home::Keep, Some(SECOND));
        assert_eq!(
            line,
            "Restore incomplete · system partly restored · reconnect the backup disk"
        );
        assert_eq!(
            tooltip,
            "Stopped after 3 tries: the backup disk was disconnected. Reconnect the backup \
             disk and click Restore again, or restore the safety snapshot."
        );
        let (same_line, tooltip) = shown(Outcome::Failed, WHY, Home::Keep, None);
        assert_eq!(same_line, line);
        assert_eq!(
            tooltip,
            "Stopped after 3 tries: the backup disk was disconnected. Reconnect the backup \
             disk and click Restore again. No safety snapshot was taken."
        );
        assert_eq!(tone(Outcome::Failed, WHY), Some(Tone::Error));
    }

    /// `not-started` keeps its line and the helper's words as its tooltip.
    #[test]
    fn not_started_keeps_its_line_and_the_helpers_words() {
        const WHY: &str = "there isn't enough free space on the system disk";
        let (line, tooltip) = shown(Outcome::NotStarted, WHY, Home::Keep, Some(SECOND));
        assert_eq!(
            line,
            "The restore didn't start · nothing was changed · free space on the system disk"
        );
        assert_eq!(tooltip, WHY);
        assert_eq!(tone(Outcome::NotStarted, WHY), None);
    }

    /// Every restore state the window can show, by name (PLAN 6b.8's preview states).
    fn states() -> Vec<(&'static str, AppModel)> {
        let checked = |check: Check| {
            let mut app = window();
            click(&mut app, 0, Modifiers::empty());
            send(&mut app, Message::RestoreClicked);
            send(&mut app, Message::Checked(NEWEST.to_owned(), Ok(check)));
            app
        };
        let result = |state: Outcome, message: &str| {
            with_result(state, message, Home::Restore, Some(SECOND))
        };
        let mut home_no_safety = with_dialog();
        send(&mut home_no_safety, Message::RestoreHome(1));
        send(&mut home_no_safety, Message::RestoreSafety(false));
        let mut dropped = ready();
        send(&mut dropped, Message::RestartNow);
        send(
            &mut dropped,
            Message::RestartAnswered(Err(CliError::RestoreRefused(Refusal::BootFiles(
                CheckFailure::NoEntry,
            )))),
        );
        let mut preparing = with_dialog();
        send(&mut preparing, Message::DialogConfirm);
        send(
            &mut preparing,
            Message::Job(JobEvent::Changed(job(
                JobKind::Restore,
                JobState::Running,
                NEWEST,
            ))),
        );
        send(
            &mut preparing,
            Message::Progress(Progress {
                percent: Some(42.0),
                eta_seconds: Some(180),
                text: String::new(),
            }),
        );
        let mut stop = with_dialog();
        send(&mut stop, Message::DialogConfirm);
        send(
            &mut stop,
            Message::Job(JobEvent::Changed(job(
                JobKind::Restore,
                JobState::Running,
                NEWEST,
            ))),
        );
        send(&mut stop, Message::StopClicked);
        vec![
            ("restore", with_dialog()),
            ("restore-home-no-safety", home_no_safety),
            (
                "restore-old-format",
                checked(Check {
                    old_format: true,
                    ..check()
                }),
            ),
            (
                "restore-no-home",
                checked(Check {
                    has_home: false,
                    ..check()
                }),
            ),
            (
                "restore-no-apsis",
                checked(Check {
                    apsis: InSnapshot::NotInstalled,
                    ..check()
                }),
            ),
            (
                "restore-apsis-0.4",
                checked(Check {
                    apsis: InSnapshot::NoRestore {
                        version: "0.4.2".to_owned(),
                    },
                    ..check()
                }),
            ),
            (
                "restore-apsis-0.3",
                checked(Check {
                    apsis: InSnapshot::OldSettings {
                        version: "0.3.1".to_owned(),
                    },
                    old_format: true,
                    has_home: false,
                    ..check()
                }),
            ),
            (
                "restore-refused",
                checked(Check {
                    refusal: Some(Refusal::OtherInstallation),
                    ..check()
                }),
            ),
            ("restore-refused-dropped", dropped),
            ("preparing", preparing),
            ("stop-restore", stop),
            ("ready", ready()),
            ("result-done", result(Outcome::Done, "")),
            (
                "result-problems",
                result(
                    Outcome::Problems,
                    "some files couldn't be written or deleted; see \
                     /var/lib/apsis/restore/rsync-log",
                ),
            ),
            (
                "result-problems-other",
                result(
                    Outcome::Problems,
                    "the kernel from before the restore couldn't be removed (Read-only file \
                     system)",
                ),
            ),
            ("result-boot-kept", result(Outcome::BootKept, "")),
            (
                "result-boot-broken",
                result(
                    Outcome::BootBroken,
                    "the boot refresh failed (kernelstub exited 1); the boot files from \
                     before couldn't be put back (initrd.img doesn't match)",
                ),
            ),
            (
                "result-failed",
                result(Outcome::Failed, "the backup disk was disconnected"),
            ),
            (
                "result-failed-no-safety",
                with_result(
                    Outcome::Failed,
                    "the backup disk was disconnected",
                    Home::Keep,
                    None,
                ),
            ),
            (
                "result-not-started",
                result(Outcome::NotStarted, "the backup disk wasn't found"),
            ),
        ]
    }

    /// Every restore state at 0.4.0's default and smallest window (PLAN 6b.12): each dialog,
    /// buttons included, is at most the window's height minus 16 px (it's centred over the
    /// whole window, header included), and the page under it fits. With `APSIS_SCREENSHOTS`
    /// set, each is also written as `restore-<state>-<w>x<h>.rgba` (as `screenshots` writes
    /// them): the page under a 48 px header, the dialog centred over it.
    #[test]
    fn every_restore_state_fits_both_window_sizes() {
        const HEADER: f32 = 48.0;
        let shots = std::env::var_os("APSIS_SCREENSHOTS").map(std::path::PathBuf::from);
        for (name, mut app) in states() {
            for size in [WINDOW_SIZE, WINDOW_MIN_SIZE] {
                app.window_height = size.height;
                let content = Size::new(size.width, size.height - HEADER);
                let Some(page) = layout(app.window_view(), content) else {
                    return;
                };
                assert!(
                    page.bounds().height <= content.height + 0.5,
                    "{name} at {size:?}: page {:?}",
                    page.bounds()
                );
                assert!(
                    page.bounds().width <= content.width + 0.5,
                    "{name} at {size:?}: page {:?}",
                    page.bounds()
                );
                if let Some(dialog) = app.dialog_view() {
                    let node = layout(dialog, size).unwrap();
                    assert!(
                        node.bounds().height <= size.height - 16.0,
                        "{name} at {size:?}: dialog {:?}",
                        node.bounds()
                    );
                    assert!(
                        node.bounds().width <= size.width - 16.0,
                        "{name} at {size:?}: dialog {:?}",
                        node.bounds()
                    );
                }
                if let Some(dir) = &shots {
                    let mut layers: Vec<Element<'_, Message>> = vec![
                        widget::column::with_children(vec![
                            widget::space::vertical()
                                .height(cosmic::iced::Length::Fixed(HEADER))
                                .into(),
                            app.window_view(),
                        ])
                        .into(),
                    ];
                    if let Some(dialog) = app.dialog_view() {
                        layers.push(
                            widget::container(dialog)
                                .center(cosmic::iced::Length::Fill)
                                .into(),
                        );
                    }
                    let stack: Element<'_, Message> =
                        cosmic::iced::widget::Stack::with_children(layers).into();
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "a window"
                    )]
                    let (w, h) = (size.width as u32, size.height as u32);
                    shoot_rgba(dir, &format!("restore-{name}-{w}x{h}"), stack, size);
                }
            }
        }
    }

    /// Draws `element` at `size` with the dark theme and writes it as `<name>.rgba` in `dir`
    /// (the `screenshots` test's format).
    fn shoot_rgba(
        dir: &std::path::Path,
        name: &str,
        mut element: Element<'_, Message>,
        size: Size,
    ) {
        use cosmic::iced::core::layout::{Layout, Limits as LayoutLimits};
        use cosmic::iced::core::renderer::{Headless, Style};
        use cosmic::iced::core::widget::Tree;
        use cosmic::iced::core::{Rectangle, mouse};
        let Some(mut renderer) =
            cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(
                cosmic::font::default(),
                14.0.into(),
                Some("tiny-skia"),
            ))
        else {
            return;
        };
        std::fs::create_dir_all(dir).unwrap();
        let theme = cosmic::Theme::dark();
        let container = theme.cosmic().background(false);
        let background: cosmic::iced::Color = container.base.into();
        let text_color: cosmic::iced::Color = container.on.into();
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &LayoutLimits::new(Size::ZERO, size),
        );
        element.as_widget().draw(
            &tree,
            &mut renderer,
            &theme,
            &Style {
                text_color,
                icon_color: text_color,
                scale_factor: 1.0,
            },
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &Rectangle::with_size(size),
        );
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a window"
        )]
        let (width, height) = (size.width as u32, size.height as u32);
        let pixels = renderer.screenshot(Size::new(width, height), 1.0, background);
        let mut out = Vec::with_capacity(8 + pixels.len());
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.extend(pixels);
        std::fs::write(dir.join(format!("{name}.rgba")), out).unwrap();
    }
}
