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

use apsis_core::{
    Backend, Error, MAX_COMMENT_CHARS, MAX_OUTPUT_LINES, RunOutput, Runner, TimeshiftCli,
};

const DEVICE: &str = include_str!("fixtures/list-rsync-device.txt");
const UNCONFIGURED: &str = include_str!("fixtures/list-unconfigured.txt");
const UUID: &str = "00000000-0000-0000-0000-000000000000";
const STALE_MOUNT: &str = include_str!("fixtures/list-rsync-stale-mount.txt");
const DEVICE_NOT_FOUND: &str = include_str!("fixtures/list-device-not-found.txt");

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

/// A backend that has listed the fixture device, as the applet always has before a create or
/// delete. `replies` answer the calls after that list.
fn listed(
    runner: &FakeRunner,
    replies: impl IntoIterator<Item = io::Result<RunOutput>>,
) -> TimeshiftCli<&FakeRunner> {
    runner.replies.borrow_mut().push_front(ok(DEVICE));
    runner.replies.borrow_mut().extend(replies);
    let cli = TimeshiftCli::new(runner);
    cli.list().unwrap();
    runner.calls.borrow_mut().clear();
    cli
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
    let runner = FakeRunner::replying([ok(DEVICE), ok(UNCONFIGURED), ok(UNCONFIGURED)]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    cli.list().unwrap();
    cli.list().unwrap();
    assert_eq!(
        runner.calls()[2],
        argv(&["timeshift", "--list", "--scripted"])
    );
}

#[test]
fn create_passes_comment_as_one_argument() {
    let runner = FakeRunner::default();
    listed(&runner, [ok("")])
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
            "--snapshot-device",
            UUID,
        ])]
    );
}

#[test]
fn create_trims_comment() {
    let runner = FakeRunner::default();
    listed(&runner, [ok("")]).create("  hi there \t").unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&[
            "timeshift",
            "--create",
            "--comments",
            "hi there",
            "--scripted",
            "--snapshot-device",
            UUID,
        ])]
    );
}

#[test]
fn blank_comment_omits_comments_flag() {
    let runner = FakeRunner::default();
    listed(&runner, [ok("")]).create("   ").unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&[
            "timeshift",
            "--create",
            "--scripted",
            "--snapshot-device",
            UUID
        ])]
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
    let runner = FakeRunner::default();
    listed(&runner, [ok("")]).create(&at_limit).unwrap();
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
        matches!(&result, Err(Error::Failed { code: Some(1), output }) if output == "E: boom"),
        "{result:?}"
    );
}

#[test]
fn failed_create_and_delete_report_failure() {
    let runner = FakeRunner::default();
    let cli = listed(&runner, [failed(), failed()]);
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
    let runner = FakeRunner::default();
    listed(&runner, [ok("")])
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
            "--snapshot-device",
            UUID,
        ])]
    );
}

#[test]
fn delete_targets_the_listed_device() {
    let runner = FakeRunner::default();
    listed(&runner, [ok("")])
        .delete("2026-09-19_09-29-57")
        .unwrap();
    assert_eq!(
        runner.calls(),
        [argv(&[
            "timeshift",
            "--delete",
            "--snapshot",
            "2026-09-19_09-29-57",
            "--scripted",
            "--snapshot-device",
            UUID,
        ])]
    );
}

#[test]
fn create_and_delete_need_a_listed_device() {
    // Nothing listed yet: nothing runs, rather than falling back to Timeshift's default device.
    let runner = FakeRunner::default();
    let cli = TimeshiftCli::new(&runner);
    assert!(matches!(cli.create("x"), Err(Error::NoSnapshotDevice)));
    assert!(matches!(
        cli.delete("2026-09-19_09-29-57"),
        Err(Error::NoSnapshotDevice)
    ));
    assert!(runner.calls().is_empty());

    // A list without a configured device forgets the old one, so the same holds after it.
    let runner = FakeRunner::replying([ok(DEVICE), ok(UNCONFIGURED)]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    cli.list().unwrap();
    assert!(matches!(cli.create(""), Err(Error::NoSnapshotDevice)));
    assert_eq!(runner.calls().len(), 2);
}

#[test]
fn bad_input_is_reported_before_a_missing_device() {
    let runner = FakeRunner::default();
    let cli = TimeshiftCli::new(&runner);
    assert!(matches!(cli.create("-x"), Err(Error::InvalidComment(_))));
    assert!(matches!(
        cli.delete("--delete-all"),
        Err(Error::InvalidSnapshotName(_))
    ));
}

#[test]
fn validate_comment_is_what_create_checks() {
    assert_eq!(apsis_core::validate_comment("  hi  ").unwrap(), "hi");
    assert!(apsis_core::validate_comment("-x").is_err());
    assert!(apsis_core::validate_comment("a\nb").is_err());
}

/// A failed run as Timeshift does it: `E:` lines on stdout, often nothing on stderr.
fn failed_with(code: i32, stdout: &str, stderr: &str) -> io::Result<RunOutput> {
    Ok(RunOutput {
        success: false,
        code: Some(code),
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    })
}

#[test]
fn unplugged_disk_is_device_not_found() {
    let runner = FakeRunner::replying([failed_with(1, DEVICE_NOT_FOUND, "")]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(
        matches!(&result, Err(Error::DeviceNotFound { device }) if device == "/dev/sdX1"),
        "{result:?}"
    );
}

#[test]
fn unplugged_disk_is_recognised_even_with_exit_code_zero() {
    let runner = FakeRunner::replying([ok(DEVICE_NOT_FOUND)]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(
        matches!(result, Err(Error::DeviceNotFound { .. })),
        "{result:?}"
    );
}

#[test]
fn failure_output_comes_from_stdout_errors_too() {
    // Before: only stderr was kept, which was empty, so all the user saw was the exit code.
    let stdout = "Mounted '/dev/sdX1' at '/run/timeshift/1/backup'\nE: first\nRet=256\nW: second\n";
    let runner = FakeRunner::replying([failed_with(1, stdout, "E: on stderr\n")]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(
        matches!(&result, Err(Error::Failed { code: Some(1), output })
            if output == "E: first\nW: second\nE: on stderr"),
        "{result:?}"
    );
}

#[test]
fn failure_output_keeps_the_last_lines() {
    let stdout: String = (1..=9).map(|i| format!("E: line {i}\n")).collect();
    let runner = FakeRunner::replying([failed_with(1, &stdout, "")]);
    let Err(Error::Failed { output, .. }) = TimeshiftCli::new(&runner).list() else {
        panic!("expected Failed")
    };
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), MAX_OUTPUT_LINES);
    assert_eq!(lines.last(), Some(&"E: line 9"));
}

#[test]
fn failure_without_error_lines_shows_stdout() {
    let runner = FakeRunner::replying([failed_with(2, "something else\n", "")]);
    let result = TimeshiftCli::new(&runner).list();
    assert!(
        matches!(&result, Err(Error::Failed { output, .. }) if output == "something else"),
        "{result:?}"
    );
}

#[test]
fn stale_mount_warning_keeps_the_list_and_the_uuid() {
    let runner = FakeRunner::replying([ok(STALE_MOUNT), ok("")]);
    let cli = TimeshiftCli::new(&runner);
    let list = cli.list().unwrap();
    assert_eq!(list.warnings, ["E: Failed to remove directory"]);
    assert_eq!(cli.snapshot_device().as_deref(), Some(UUID));
    cli.create("").unwrap();
    let create = &runner.calls()[1];
    assert!(create.iter().any(|a| a == UUID));
    assert!(
        !create
            .iter()
            .any(|a| a.to_string_lossy().starts_with("/dev/"))
    );
}

#[test]
fn unplugged_disk_keeps_the_uuid_for_the_next_try() {
    let runner =
        FakeRunner::replying([ok(DEVICE), failed_with(1, DEVICE_NOT_FOUND, ""), ok(DEVICE)]);
    let cli = TimeshiftCli::new(&runner);
    cli.list().unwrap();
    assert!(cli.list().is_err());
    cli.list().unwrap();
    let third = argv(&[
        "timeshift",
        "--list",
        "--scripted",
        "--snapshot-device",
        UUID,
    ]);
    assert_eq!(runner.calls()[2], third);
}
