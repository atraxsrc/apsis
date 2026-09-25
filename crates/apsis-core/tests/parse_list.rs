// SPDX-License-Identifier: GPL-3.0-only

//! Parser tests. `fixtures/` holds real (redacted) Timeshift v24.01.1 output; the inline strings
//! below are synthetic variations built from that layout.

use apsis_core::{Error, Mode, Snapshot, SnapshotList, Tag, parse_list};
use jiff::civil::{DateTime, date};

const DEVICE: &str = include_str!("fixtures/list-rsync-device.txt");
const PLAIN: &str = include_str!("fixtures/list-rsync-plain.txt");
const UNCONFIGURED: &str = include_str!("fixtures/list-unconfigured.txt");

const HEADER: &str = concat!(
    "Device : /dev/sdX1\n",
    "UUID   : 00000000-0000-0000-0000-000000000000\n",
    "Path   : /run/timeshift/99999/backup\n",
    "Mode   : RSYNC\n",
    "Status : OK\n",
    "1 snapshots, 123.4 GB free\n",
    "\n",
    "Num     Name                 Tags  Description  \n",
    "------------------------------------------------------------------------------\n",
);

fn on_demand(name: &str, created: DateTime, comment: Option<&str>) -> Snapshot {
    Snapshot {
        name: name.to_owned(),
        created,
        tags: vec![Tag::OnDemand],
        comment: comment.map(str::to_owned),
    }
}

#[test]
fn reads_device_uuid_and_mode_from_header() {
    let list = parse_list(DEVICE).unwrap();
    assert_eq!(list.device.as_deref(), Some("/dev/sdX1"));
    assert_eq!(
        list.uuid.as_deref(),
        Some("00000000-0000-0000-0000-000000000000")
    );
    assert_eq!(list.mode, Some(Mode::Rsync));
}

#[test]
fn reads_every_snapshot_row_with_comment() {
    let list = parse_list(DEVICE).unwrap();
    assert_eq!(
        list.snapshots,
        vec![
            on_demand(
                "2026-09-19_09-29-57",
                date(2026, 9, 19).at(9, 29, 57, 0),
                None
            ),
            on_demand(
                "2026-09-19_09-58-41",
                date(2026, 9, 19).at(9, 58, 41, 0),
                None
            ),
            on_demand(
                "2026-09-22_13-28-36",
                date(2026, 9, 22).at(13, 28, 36, 0),
                None
            ),
            on_demand(
                "2026-09-23_08-33-55",
                date(2026, 9, 23).at(8, 33, 55, 0),
                None
            ),
            on_demand(
                "2026-09-25_11-28-53",
                date(2026, 9, 25).at(11, 28, 53, 0),
                Some("apsis test: comment with spaces")
            ),
        ]
    );
}

#[test]
fn reads_rows_without_comments() {
    let list = parse_list(PLAIN).unwrap();
    assert_eq!(
        list.snapshots,
        vec![
            on_demand(
                "2026-09-19_09-29-57",
                date(2026, 9, 19).at(9, 29, 57, 0),
                None
            ),
            on_demand(
                "2026-09-19_09-58-41",
                date(2026, 9, 19).at(9, 58, 41, 0),
                None
            ),
            on_demand(
                "2026-09-22_13-28-36",
                date(2026, 9, 22).at(13, 28, 36, 0),
                None
            ),
            on_demand(
                "2026-09-23_08-33-55",
                date(2026, 9, 23).at(8, 33, 55, 0),
                None
            ),
        ]
    );
}

#[test]
fn unconfigured_device_gives_empty_list_without_device() {
    assert_eq!(parse_list(UNCONFIGURED).unwrap(), SnapshotList::default());
}

#[test]
fn configured_device_without_snapshots_keeps_header() {
    let output = "\
Device : /dev/sdX1
UUID   : 00000000-0000-0000-0000-000000000000
Mode   : BTRFS
Status : OK
No snapshots found
";
    let list = parse_list(output).unwrap();
    assert_eq!(list.device.as_deref(), Some("/dev/sdX1"));
    assert_eq!(list.mode, Some(Mode::Btrfs));
    assert!(list.snapshots.is_empty());
}

#[test]
fn row_with_several_tags_keeps_all_of_them() {
    let output = format!("{HEADER}0    >  2026-09-24_03-00-01  BD    \n");
    let list = parse_list(&output).unwrap();
    assert_eq!(list.snapshots[0].tags, vec![Tag::Boot, Tag::Daily]);
    assert_eq!(list.snapshots[0].comment, None);
}

#[test]
fn comment_keeps_inner_spacing() {
    let output = format!("{HEADER}0    >  2026-09-24_03-00-01  O     a  b:  c   \n");
    let list = parse_list(&output).unwrap();
    assert_eq!(list.snapshots[0].comment.as_deref(), Some("a  b:  c"));
}

#[test]
fn row_without_tags_keeps_rest_as_comment() {
    let output = format!("{HEADER}0    >  2026-09-24_03-00-01  hello world\n");
    let list = parse_list(&output).unwrap();
    assert_eq!(list.snapshots[0].tags, vec![]);
    assert_eq!(list.snapshots[0].comment.as_deref(), Some("hello world"));
}

#[test]
fn glib_warnings_inside_table_are_ignored() {
    let output = [
        HEADER,
        "0    >  2026-09-24_03-00-01  O     \n",
        "** (process:99999): CRITICAL **: 11:24:41.486: gee_abstract_collection_get_size: assertion 'self != NULL' failed\n",
        "(timeshift:99999): GLib-WARNING **: 11:24:41.487: something\n",
        "1    >  2026-09-25_03-00-01  D     \n",
    ]
    .concat();
    let names: Vec<_> = parse_list(&output)
        .unwrap()
        .snapshots
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["2026-09-24_03-00-01", "2026-09-25_03-00-01"]);
}

#[test]
fn malformed_row_is_reported_with_line_number() {
    // HEADER is 9 lines, so the bad row is line 11.
    let output =
        format!("{HEADER}0    >  2026-09-24_03-00-01  O     \n1    >  2026-09-24_03-00  O     \n");
    match parse_list(&output) {
        Err(Error::BadRow { line, text }) => {
            assert_eq!(line, 11);
            assert_eq!(text, "1    >  2026-09-24_03-00  O     ");
        }
        other => panic!("expected BadRow, got {other:?}"),
    }
}

#[test]
fn output_without_table_or_empty_marker_is_rejected() {
    for output in [
        "",
        "\n",
        "E: Failed to mount device '/dev/sdX1'\n",
        "Device : /dev/sdX1\n",
    ] {
        assert!(
            matches!(parse_list(output), Err(Error::UnrecognisedOutput)),
            "{output:?}"
        );
    }
}
