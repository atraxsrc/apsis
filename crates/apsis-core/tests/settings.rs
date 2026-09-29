// SPDX-License-Identifier: GPL-3.0-only

use apsis_core::config::import_timeshift;
use apsis_core::settings::{HomeState, User, parse_lsblk, parse_passwd, validate_filter};

/// The user's real settings file, redacted (placeholder UUID and user names).
const CONFIG: &str = include_str!("fixtures/config-rsync.json");
/// Synthetic `lsblk` output: a system disk, the backup disk (`sdb1`, the UUID in `CONFIG`), a
/// LUKS disk with an unlocked filesystem, and a btrfs disk (size as a string, like older lsblk).
const LSBLK: &str = include_str!("fixtures/lsblk.json");

const BACKUP_UUID: &str = "00000000-0000-0000-0000-000000000000";
const LUKS_UUID: &str = "22222222-2222-2222-2222-222222222222";
const UNLOCKED_UUID: &str = "33333333-3333-3333-3333-333333333333";
const BTRFS_UUID: &str = "44444444-4444-4444-4444-444444444444";

/// The real file's filter list, as the import takes it.
fn imported_filters() -> Vec<String> {
    import_timeshift(CONFIG, &[], &[]).unwrap().config.filters
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
    // Unlocked LUKS: shown by Timeshift, but not selectable in Apsis (not supported yet).
    let unlocked = find(UNLOCKED_UUID);
    assert_eq!(unlocked.parent_uuid, LUKS_UUID);
    assert_eq!(unlocked.path(), "/dev/mapper/luks-2222");
    assert!(unlocked.is_linux() && !unlocked.selectable());
    assert!(find(LUKS_UUID).is_linux() && !find(LUKS_UUID).selectable());
    // vfat isn't a Linux filesystem.
    assert!(!find("AAAA-0001").is_linux());
    assert_eq!(find(BTRFS_UUID).size, 256_059_465_728);
    assert!(parse_lsblk("{}").is_err());
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
}

#[test]
fn home_states_use_timeshifts_patterns() {
    let user = User {
        name: "user1".to_owned(),
        home: "/home/user1".to_owned(),
        encrypted_home: false,
    };
    let mut exclude = imported_filters();
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
    let exclude = imported_filters();
    assert!(
        users
            .iter()
            .all(|u| u.home_state(&exclude) == HomeState::All)
    );
}
