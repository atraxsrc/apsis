// SPDX-License-Identifier: GPL-3.0-only

//! Command-building tests for `TimeshiftCli`. `FakeRunner` stands in for running `timeshift` as
//! root: it records each argv and replies with canned output.

#![allow(
    clippy::unnecessary_wraps,
    reason = "reply helpers return io::Result because that's what Runner::run returns"
)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;

use apsis_core::{Backend, Error, MAX_COMMENT_CHARS, RunOutput, Runner, TimeshiftCli};

const DEVICE: &str = include_str!("fixtures/list-rsync-device.txt");
const UNCONFIGURED: &str = include_str!("fixtures/list-unconfigured.txt");
const UUID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Default)]
struct FakeRunner {
    calls: RefCell<Vec<Vec<OsString>>>,
    replies: RefCell<VecDeque<io::Result<RunOutput>>>,
}

impl FakeRunner {
    fn replying(replies: impl IntoIterator<Item = io::Result<RunOutput>>) -> Self {
        Self {
            calls: RefCell::default(),
            replies: RefCell::new(replies.into_iter().collect()),
        }
    }

    fn calls(&self) -> Vec<Vec<OsString>> {
        self.calls.borrow().clone()
    }
}

impl Runner for &FakeRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        self.calls.borrow_mut().push(argv.to_vec());
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected extra command")
    }
}

fn ok(stdout: &str) -> io::Result<RunOutput> {
    Ok(RunOutput {
        success: true,
        code: Some(0),
        stdout: stdout.to_owned(),
        stderr: String::new(),
    })
}

fn argv(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn first_list_uses_configured_device() {
    let runner = FakeRunner::replying([ok(DEVICE)]);
    let list = TimeshiftCli::new(&runner).list().unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&["timeshift", "--list", "--scripted"])]
    );
    assert_eq!(list.snapshots.len(), 5);
}

#[test]
fn calls_after_list_target_listed_uuid() {
    let runner = FakeRunner::replying([ok(DEVICE), ok(DEVICE), ok(""), ok("")]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    cli.list().unwrap();
    cli.create("before update").unwrap();
    cli.delete("2026-09-19_09-29-57").unwrap();
    assert_eq!(
        runner.calls()[1..],
        [
            argv(&[
                "timeshift",
                "--list",
                "--scripted",
                "--snapshot-device",
                UUID
            ]),
            argv(&[
                "timeshift",
                "--create",
                "--comments",
                "before update",
                "--scripted",
                "--snapshot-device",
                UUID,
            ]),
            argv(&[
                "timeshift",
                "--delete",
                "--snapshot",
                "2026-09-19_09-29-57",
                "--scripted",
                "--snapshot-device",
                UUID,
            ]),
        ]
    );
}

#[test]
fn device_path_is_used_when_list_has_no_uuid() {
    let no_uuid: String = DEVICE
        .lines()
        .filter(|l| !l.starts_with("UUID"))
        .flat_map(|l| [l, "\n"])
        .collect();
    let runner = FakeRunner::replying([ok(&no_uuid), ok("")]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    cli.create("").unwrap();
    assert_eq!(
        runner.calls()[1],
        argv(&[
            "timeshift",
            "--create",
            "--scripted",
            "--snapshot-device",
            "/dev/sdX1"
        ])
    );
}

#[test]
fn unconfigured_list_forgets_previous_device() {
    let runner = FakeRunner::replying([ok(DEVICE), ok(UNCONFIGURED), ok("")]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    cli.list().unwrap();
    cli.create("").unwrap();
    assert_eq!(
        runner.calls()[2],
        argv(&["timeshift", "--create", "--scripted"])
    );
}

#[test]
fn create_passes_comment_as_one_argument() {
    let runner = FakeRunner::replying([ok("")]);
    TimeshiftCli::new(&runner)
        .create("before kernel update; rm -rf /")
        .unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&[
            "timeshift",
            "--create",
            "--comments",
            "before kernel update; rm -rf /",
            "--scripted",
        ])]
    );
}

#[test]
fn create_trims_comment() {
    let runner = FakeRunner::replying([ok("")]);
    TimeshiftCli::new(&runner).create("  hi there \t").unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&[
            "timeshift",
            "--create",
            "--comments",
            "hi there",
            "--scripted"
        ])]
    );
}

#[test]
fn blank_comment_omits_comments_flag() {
    let runner = FakeRunner::replying([ok("")]);
    TimeshiftCli::new(&runner).create("   ").unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&["timeshift", "--create", "--scripted"])]
    );
}

#[test]
fn comment_with_control_characters_is_rejected_before_running() {
    for bad in ["two\nlines", "tab\tinside", "esc\u{1b}[31m", "nul\0"] {
        let runner = FakeRunner::default();
        let result = TimeshiftCli::new(&runner).create(bad);
        assert!(matches!(result, Err(Error::InvalidComment(_))), "{bad:?}");
        assert!(runner.calls().is_empty(), "{bad:?}");
    }
}

#[test]
fn comment_length_is_limited_in_characters() {
    // Multi-byte characters: the limit counts characters, not bytes.
    let at_limit = "é".repeat(MAX_COMMENT_CHARS);
    let runner = FakeRunner::replying([ok("")]);
    TimeshiftCli::new(&runner).create(&at_limit).unwrap();
    assert_eq!(runner.calls().len(), 1);

    let over = "é".repeat(MAX_COMMENT_CHARS + 1);
    let runner = FakeRunner::default();
    let result = TimeshiftCli::new(&runner).create(&over);
    assert!(matches!(result, Err(Error::InvalidComment(_))));
    assert!(runner.calls().is_empty());
}

#[test]
fn delete_rejects_anything_but_a_snapshot_name() {
    for bad in [
        "--delete-all",
        "",
        "2026-09-19_09-29-57 --delete-all",
        "../x",
    ] {
        let runner = FakeRunner::default();
        let result = TimeshiftCli::new(&runner).delete(bad);
        assert!(
            matches!(&result, Err(Error::InvalidSnapshotName(n)) if n == bad),
            "{bad:?}: {result:?}"
        );
        assert!(runner.calls().is_empty(), "{bad:?}");
    }
}

#[test]
fn missing_binary_is_not_installed() {
    let runner = FakeRunner::replying([Err(io::Error::from(io::ErrorKind::NotFound))]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(matches!(result, Err(Error::NotInstalled)), "{result:?}");
}

#[test]
fn other_spawn_errors_are_io_errors() {
    let runner = FakeRunner::replying([Err(io::Error::from(io::ErrorKind::PermissionDenied))]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(
        matches!(&result, Err(Error::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied),
        "{result:?}"
    );
}

fn failed() -> io::Result<RunOutput> {
    Ok(RunOutput {
        success: false,
        code: Some(1),
        // A failed list still printing a valid table must not be treated as success.
        stdout: DEVICE.to_owned(),
        stderr: "E: boom\n".to_owned(),
    })
}

#[test]
fn failed_list_reports_exit_code_and_stderr() {
    let runner = FakeRunner::replying([failed()]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(
        matches!(&result, Err(Error::Failed { code: Some(1), stderr }) if stderr == "E: boom\n"),
        "{result:?}"
    );
}

#[test]
fn failed_create_and_delete_report_failure() {
    let runner = FakeRunner::replying([failed(), failed()]);
    let cli = TimeshiftCli::new(&runner);
    assert!(matches!(cli.create("x"), Err(Error::Failed { .. })));
    assert!(matches!(
        cli.delete("2026-09-19_09-29-57"),
        Err(Error::Failed { .. })
    ));
}

#[test]
fn failed_list_keeps_previous_device() {
    let runner = FakeRunner::replying([ok(DEVICE), failed(), ok("")]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    let _ = cli.list();
    cli.create("").unwrap();
    assert_eq!(
        runner.calls()[2],
        argv(&[
            "timeshift",
            "--create",
            "--scripted",
            "--snapshot-device",
            UUID
        ])
    );
}

#[test]
fn comment_that_looks_like_an_option_is_rejected() {
    for bad in ["--delete-all", "-x", "  --yes"] {
        let runner = FakeRunner::default();
        let result = TimeshiftCli::new(&runner).create(bad);
        assert!(matches!(result, Err(Error::InvalidComment(_))), "{bad:?}");
        assert!(runner.calls().is_empty(), "{bad:?}");
    }
}

#[test]
fn comment_may_contain_dashes_after_the_start() {
    let runner = FakeRunner::replying([ok("")]);
    TimeshiftCli::new(&runner)
        .create("pre-upgrade - kernel 6.x")
        .unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&[
            "timeshift",
            "--create",
            "--comments",
            "pre-upgrade - kernel 6.x",
            "--scripted",
        ])]
    );
}
