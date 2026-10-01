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
