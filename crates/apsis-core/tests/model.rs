// SPDX-License-Identifier: GPL-3.0-only

use apsis_core::{Tag, parse_snapshot_name};
use jiff::civil::date;

#[test]
fn snapshot_name_parses_to_local_time() {
    assert_eq!(
        parse_snapshot_name("2026-09-25_11-28-53"),
        Some(date(2026, 9, 25).at(11, 28, 53, 0))
    );
}

#[test]
fn snapshot_name_rejects_anything_else() {
    for bad in [
        "",
        "--delete-all",
        "2026-09-25_11-28-5",
        "2026-09-25_11-28-530",
        "2026-09-25 11-28-53",
        "2026-09-25_11:28:53",
        " 2026-09-25_11-28-53",
        "2026-09-25_11-28-53 ",
        "+026-09-25_11-28-53",
        "2026-13-01_00-00-00",
        "2026-02-30_00-00-00",
        "2026-09-25_24-00-00",
        "2026-09-25_11-60-00",
    ] {
        assert_eq!(parse_snapshot_name(bad), None, "{bad:?}");
    }
}

#[test]
fn tag_letters_map_to_tags() {
    let table = [
        ('O', Some(Tag::OnDemand)),
        ('B', Some(Tag::Boot)),
        ('H', Some(Tag::Hourly)),
        ('D', Some(Tag::Daily)),
        ('W', Some(Tag::Weekly)),
        ('M', Some(Tag::Monthly)),
        ('o', None),
        ('X', None),
        (' ', None),
    ];
    for (c, want) in table {
        assert_eq!(Tag::from_char(c), want, "{c:?}");
    }
}

#[test]
fn tag_letter_round_trips() {
    for c in "OBHDWM".chars() {
        assert_eq!(Tag::from_char(c).map(Tag::letter), Some(c));
    }
}
