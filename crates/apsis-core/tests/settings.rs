// SPDX-License-Identifier: GPL-3.0-only

use apsis_core::Error;
use apsis_core::settings::{
    Config, HomeState, Level, User, btrfs_available, edit, parse_lsblk, parse_passwd, validate,
    validate_filter,
};

/// The user's real settings file, redacted (placeholder UUID and user names).
const CONFIG: &str = include_str!("fixtures/config-rsync.json");
/// Synthetic `lsblk` output: a system disk, the backup disk (`sdb1`, the UUID in `CONFIG`), a
/// LUKS disk with an unlocked filesystem, and a btrfs disk (size as a string, like older lsblk).
const LSBLK: &str = include_str!("fixtures/lsblk.json");

const BACKUP_UUID: &str = "00000000-0000-0000-0000-000000000000";
const SYSTEM_UUID: &str = "11111111-1111-1111-1111-111111111111";
const LUKS_UUID: &str = "22222222-2222-2222-2222-222222222222";
const UNLOCKED_UUID: &str = "33333333-3333-3333-3333-333333333333";
const BTRFS_UUID: &str = "44444444-4444-4444-4444-444444444444";

fn invalid(result: apsis_core::Result<impl std::fmt::Debug>) -> String {
    match result {
        Err(Error::InvalidSettings(reason)) => reason,
        other => panic!("expected InvalidSettings, got {other:?}"),
    }
}

#[test]
fn reads_the_real_file() {
    let settings = Config::parse(CONFIG).unwrap().settings();
    assert_eq!(settings.backup_device_uuid, BACKUP_UUID);
    assert!(!settings.btrfs_mode);
    assert!(!settings.include_btrfs_home);
    assert_eq!(settings.schedule, [false; 5]);
    assert_eq!(settings.counts, [2, 3, 5, 6, 5]);
    assert_eq!(settings.count(Level::Hourly), 6);
    assert_eq!(
        settings.exclude,
        [
            "+ /root/**",
            "+ /home/user1/**",
            "/var/lib/libvirt/**",
            "+ /home/user2/**"
        ]
    );
}

#[test]
fn unchanged_file_is_written_byte_for_byte() {
    assert_eq!(Config::parse(CONFIG).unwrap().to_text(), CONFIG);
    let devices = parse_lsblk(LSBLK).unwrap();
    let settings = Config::parse(CONFIG).unwrap().settings();
    assert_eq!(edit(CONFIG, &settings, &devices).unwrap(), CONFIG);
}

#[test]
fn edits_change_only_their_lines_and_stay_strings() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let mut settings = Config::parse(CONFIG).unwrap().settings();
    settings.schedule[Level::Daily.index()] = true;
    settings.counts[Level::Daily.index()] = 7;
    settings.exclude.push("*.mp3".to_owned());
    let text = edit(CONFIG, &settings, &devices).unwrap();

    let expected = CONFIG
        .replace(
            r#""schedule_daily" : "false""#,
            r#""schedule_daily" : "true""#,
        )
        .replace(r#""count_daily" : "5""#, r#""count_daily" : "7""#)
        .replace(
            "    \"+ /home/user2/**\"\n",
            "    \"+ /home/user2/**\",\n    \"*.mp3\"\n",
        );
    assert_ne!(expected, CONFIG);
    assert_eq!(text, expected);
    // Every other field is still there, in order, and the file still ends without a newline.
    assert!(text.ends_with("\n  \"exclude-apps\" : []\n}"));
    assert_eq!(Config::parse(&text).unwrap().settings(), settings);
}

#[test]
fn unknown_fields_are_kept_in_place() {
    let text = "{\n  \"future_field\" : { \"a\" : 1 },\n  \"count_boot\" : \"5\"\n}";
    let config = Config::parse(text).unwrap();
    let mut settings = config.settings();
    settings.counts[Level::Boot.index()] = 4;
    let out = edit(text, &settings, &[]).unwrap();
    assert!(
        out.starts_with(
            "{\n  \"future_field\" : {\n    \"a\" : 1\n  },\n  \"count_boot\" : \"4\","
        )
    );
}

#[test]
fn files_timeshift_would_misread_are_refused() {
    for bad in [
        "not json",
        "[]",
        r#"{"btrfs_mode": true}"#,
        r#"{"count_daily": 5}"#,
        r#"{"exclude": "/home"}"#,
        r#"{"exclude": [1]}"#,
    ] {
        assert!(
            matches!(Config::parse(bad), Err(Error::InvalidConfig(_))),
            "{bad}"
        );
    }
}

#[test]
fn missing_fields_get_timeshifts_defaults() {
    let settings = Config::parse("{}").unwrap().settings();
    assert_eq!(settings.counts, [2, 3, 5, 6, 5]);
    assert_eq!(settings.schedule, [false; 5]);
    assert!(settings.exclude.is_empty() && settings.backup_device_uuid.is_empty());
    // Vala's int.parse: not a number is 0, which validate then refuses.
    let settings = Config::parse(r#"{"count_weekly": "lots"}"#)
        .unwrap()
        .settings();
    assert_eq!(settings.count(Level::Weekly), 0);
}

#[test]
fn the_old_home_field_wins_when_present() {
    let text = r#"{"include_btrfs_home": "true", "include_btrfs_home_for_backup": "false"}"#;
    let config = Config::parse(text).unwrap();
    assert!(config.settings().include_btrfs_home);
    let mut settings = config.settings();
    settings.include_btrfs_home = false;
    let out = edit(text, &settings, &[]).unwrap();
    assert!(!Config::parse(&out).unwrap().settings().include_btrfs_home);
    assert!(out.contains(r#""include_btrfs_home" : "false""#));
}

#[test]
fn lsblk_devices_and_parents() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let find = |uuid: &str| devices.iter().find(|d| d.uuid == uuid).unwrap();
    let backup = find(BACKUP_UUID);
    assert_eq!(backup.path(), "/dev/sdb1");
    assert_eq!(
        (backup.size, backup.label.as_str()),
        (1_000_203_837_440, "Backup")
    );
    // A partition on a plain disk: the disk has no UUID, so no parent UUID (as in the real file).
    assert_eq!(backup.parent_uuid, "");
    assert!(backup.selectable());
    // Unlocked LUKS: shown by Timeshift, but not selectable in Apsis.
    let unlocked = find(UNLOCKED_UUID);
    assert_eq!(unlocked.parent_uuid, LUKS_UUID);
    assert_eq!(unlocked.path(), "/dev/mapper/luks-2222");
    assert!(unlocked.is_linux() && !unlocked.selectable());
    assert!(find(LUKS_UUID).is_linux() && !find(LUKS_UUID).selectable());
    // vfat isn't a Linux filesystem.
    assert!(!find("AAAA-0001").is_linux());
    assert_eq!(find(BTRFS_UUID).size, 256_059_465_728);
    assert!(btrfs_available(&devices));
    assert!(!btrfs_available(&devices[..3]));
    assert!(parse_lsblk("{}").is_err());
}

#[test]
fn a_new_device_must_be_connected_and_selectable() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let old = Config::parse(CONFIG).unwrap().settings();
    let with_device = |uuid: &str| {
        let mut new = old.clone();
        new.backup_device_uuid = uuid.to_owned();
        new
    };
    validate(&with_device(SYSTEM_UUID), &old, &devices).unwrap();
    assert!(invalid(validate(&with_device("99999999-0000"), &old, &devices)).contains("connected"));
    assert!(invalid(validate(&with_device(UNLOCKED_UUID), &old, &devices)).contains("can't hold"));
    assert!(invalid(validate(&with_device(""), &old, &devices)).contains("choose"));
    // Unchanged, the device may be unplugged (Timeshift keeps it too).
    validate(&old, &old, &[]).unwrap();
}

#[test]
fn changing_the_device_sets_its_parent_like_timeshift() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let mut new = Config::parse(CONFIG).unwrap().settings();
    new.backup_device_uuid = SYSTEM_UUID.to_owned();
    let out = edit(CONFIG, &new, &devices).unwrap();
    let config = Config::parse(&out).unwrap();
    assert_eq!(config.settings().backup_device_uuid, SYSTEM_UUID);
    assert_eq!(config.parent_device_uuid(), "");
}

#[test]
fn btrfs_mode_needs_btrfs() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let old = Config::parse(CONFIG).unwrap().settings();
    let mut new = old.clone();
    new.btrfs_mode = true;
    // The backup device is ext4.
    assert!(invalid(validate(&new, &old, &devices)).contains("btrfs device"));
    new.backup_device_uuid = BTRFS_UUID.to_owned();
    validate(&new, &old, &devices).unwrap();
    // No btrfs anywhere: can't be turned on.
    assert!(invalid(validate(&new, &old, &devices[..3])).contains("no"));
    // Turning it on with the device unplugged: can't check it, so no.
    let mut unplugged = old.clone();
    unplugged.btrfs_mode = true;
    let others: Vec<_> = devices
        .iter()
        .filter(|d| d.uuid != BACKUP_UUID)
        .cloned()
        .collect();
    assert!(invalid(validate(&unplugged, &old, &others)).contains("connected"));
}

#[test]
fn counts_are_1_to_999() {
    let old = Config::parse(CONFIG).unwrap().settings();
    for (count, ok) in [(0, false), (1, true), (999, true), (1000, false)] {
        let mut new = old.clone();
        new.counts[Level::Boot.index()] = count;
        assert_eq!(validate(&new, &old, &[]).is_ok(), ok, "{count}");
    }
}

#[test]
fn filters_take_what_timeshift_takes() {
    for good in [
        "*.mp3",
        "/var/lib/libvirt/**",
        "+ /home/user1/**",
        " /odd but fine ",
    ] {
        validate_filter(good).unwrap();
    }
    for bad in [
        "",
        "   ",
        "+ ",
        "+  ",
        "/a\nb",
        "/tab\there",
        &"x".repeat(5000),
    ] {
        assert!(validate_filter(bad).is_err(), "{bad:?}");
    }
    let old = Config::parse(CONFIG).unwrap().settings();
    let mut new = old.clone();
    new.exclude.push("/var/lib/libvirt/**".to_owned());
    assert!(invalid(validate(&new, &old, &[])).contains("twice"));
}

#[test]
fn home_states_use_timeshifts_patterns() {
    let user = User {
        name: "user1".to_owned(),
        home: "/home/user1".to_owned(),
        encrypted_home: false,
    };
    let mut exclude = Config::parse(CONFIG).unwrap().settings().exclude;
    assert_eq!(user.home_state(&exclude), HomeState::All);

    user.set_home_state(&mut exclude, HomeState::Hidden);
    assert_eq!(user.home_state(&exclude), HomeState::Hidden);
    assert_eq!(
        exclude,
        [
            "+ /root/**",
            "/var/lib/libvirt/**",
            "+ /home/user2/**",
            "+ /home/user1/.**"
        ]
    );
    user.set_home_state(&mut exclude, HomeState::Excluded);
    assert_eq!(user.home_state(&exclude), HomeState::Excluded);
    assert_eq!(exclude.last().unwrap(), "/home/user1/**");
    assert!(!exclude.iter().any(|p| p.starts_with("+ /home/user1")));
    user.set_home_state(&mut exclude, HomeState::All);
    assert_eq!(exclude.last().unwrap(), "+ /home/user1/**");
    assert!(user.owns("+ /home/user1/**") && !user.owns("+ /home/user2/**"));

    let encrypted = User {
        encrypted_home: true,
        ..user
    };
    let mut exclude = Vec::new();
    encrypted.set_home_state(&mut exclude, HomeState::All);
    assert_eq!(exclude, ["+ /home/.ecryptfs/user1/***"]);
}

#[test]
fn passwd_lists_root_and_people_not_system_accounts() {
    let passwd = "\
user2:x:1001:1001::/home/user2:/bin/bash
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
user1:x:1000:1000:User One,,,:/home/user1:/bin/bash
broken line
";
    let users = parse_passwd(passwd);
    let names: Vec<(&str, &str)> = users
        .iter()
        .map(|u| (u.name.as_str(), u.home.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("root", "/root"),
            ("user1", "/home/user1"),
            ("user2", "/home/user2")
        ]
    );
    // The real file's patterns read as "everything" for all three.
    let exclude = Config::parse(CONFIG).unwrap().settings().exclude;
    assert!(
        users
            .iter()
            .all(|u| u.home_state(&exclude) == HomeState::All)
    );
}
