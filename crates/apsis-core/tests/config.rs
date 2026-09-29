// SPDX-License-Identifier: GPL-3.0-only

//! Apsis's own config (`/etc/apsis/config.toml`) and the one-time import from Timeshift's
//! settings.

use apsis_core::Error;
use apsis_core::config::{Config, effective, import_timeshift, validate};
use apsis_core::native::exclude;
use apsis_core::settings::{User, parse_lsblk};

/// The user's real Timeshift settings file, redacted (placeholder UUID and user names).
const TIMESHIFT: &str = include_str!("fixtures/config-rsync.json");
/// Synthetic lsblk: system disk, backup disk `sdb1`, a LUKS disk and its unlocked filesystem,
/// a btrfs disk.
const LSBLK: &str = include_str!("fixtures/lsblk.json");

const BACKUP_UUID: &str = "00000000-0000-0000-0000-000000000000";
const SYSTEM_UUID: &str = "11111111-1111-1111-1111-111111111111";
const LUKS_UUID: &str = "22222222-2222-2222-2222-222222222222";
const UNLOCKED_UUID: &str = "33333333-3333-3333-3333-333333333333";
const BTRFS_UUID: &str = "44444444-4444-4444-4444-444444444444";

fn users() -> Vec<User> {
    ["root:/root", "user1:/home/user1", "user2:/home/user2"]
        .iter()
        .map(|entry| {
            let (name, home) = entry.split_once(':').unwrap();
            User {
                name: name.to_owned(),
                home: home.to_owned(),
                encrypted_home: false,
            }
        })
        .collect()
}

fn config(uuid: &str, filters: &[&str]) -> Config {
    Config {
        backup_device_uuid: uuid.to_owned(),
        filters: filters.iter().map(|f| (*f).to_owned()).collect(),
    }
}

fn invalid(result: apsis_core::Result<()>) -> String {
    match result {
        Err(Error::InvalidSettings(reason)) => reason,
        other => panic!("expected InvalidSettings, got {other:?}"),
    }
}

#[test]
fn the_file_reads_back_what_it_writes() {
    let written = config(
        BACKUP_UUID,
        &[
            "+ /home/user1/**",
            "/var/lib/libvirt/**",
            "\"quoted\" \\ back",
            "ünïcode",
        ],
    );
    let text = written.to_text();
    assert!(text.starts_with("# Written by apsis-helper"));
    assert!(text.contains("version = 1\n"));
    assert!(text.contains(&format!("backup_device_uuid = \"{BACKUP_UUID}\"\n")));
    assert_eq!(Config::parse(&text).unwrap(), written);
    // Empty is fine too.
    let empty = Config::default();
    assert_eq!(Config::parse(&empty.to_text()).unwrap(), empty);
}

#[test]
fn files_it_cant_read_safely_are_refused() {
    for bad in [
        "",
        "version = 2",
        "version = \"1\"",
        "version = 1\nbackup_device_uuid = 5",
        "version = 1\nfilters = \"*.mp3\"",
        "version = 1\nfilters = [1]",
        "not toml at all [",
    ] {
        assert!(
            matches!(Config::parse(bad), Err(Error::InvalidConfig(_))),
            "{bad:?}"
        );
    }
    // Missing fields are empty.
    assert_eq!(Config::parse("version = 1").unwrap(), Config::default());
}

#[test]
fn a_new_device_must_be_a_connected_unencrypted_linux_filesystem() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let saved = config(BACKUP_UUID, &[]);
    // Unchanged: fine, even with the disk unplugged.
    validate(&saved, Some(&saved), &[]).unwrap();
    // A first save counts as a change.
    validate(&saved, None, &devices).unwrap();
    assert!(invalid(validate(&saved, None, &[])).contains("isn't connected"));
    assert!(invalid(validate(&config("", &[]), None, &devices)).contains("choose"));
    for (uuid, why) in [
        (LUKS_UUID, "unencrypted"),
        (UNLOCKED_UUID, "unencrypted"),
        ("AAAA-0001", "unencrypted Linux filesystem"),
    ] {
        let reason = invalid(validate(&config(uuid, &[]), Some(&saved), &devices));
        assert!(reason.contains(why), "{uuid}: {reason}");
    }
    // Another plain Linux filesystem is fine (btrfs too: Apsis takes rsync snapshots onto it).
    validate(&config(SYSTEM_UUID, &[]), Some(&saved), &devices).unwrap();
    validate(&config(BTRFS_UUID, &[]), Some(&saved), &devices).unwrap();
}

#[test]
fn filters_are_checked_and_not_repeated() {
    let saved = config(BACKUP_UUID, &[]);
    let twice = config(BACKUP_UUID, &["*.mp3", "/a", "*.mp3"]);
    assert!(invalid(validate(&twice, Some(&saved), &[])).contains("twice"));
    let blank = config(BACKUP_UUID, &["+ "]);
    assert!(invalid(validate(&blank, Some(&saved), &[])).contains("blank"));
}

#[test]
fn import_takes_the_device_and_the_filters_in_order() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let import = import_timeshift(TIMESHIFT, &devices, &users()).unwrap();
    assert_eq!(import.config.backup_device_uuid, BACKUP_UUID);
    // Exactly Timeshift's list, home patterns and all, in its order.
    assert_eq!(
        import.config.filters,
        [
            "+ /root/**",
            "+ /home/user1/**",
            "/var/lib/libvirt/**",
            "+ /home/user2/**"
        ]
    );
    let notes = import.notes.join("\n");
    assert!(
        notes.contains("imported from /etc/timeshift/timeshift.json"),
        "{notes}"
    );
    assert!(notes.contains("device   00000000… (sdb1, ext4)"), "{notes}");
    assert!(
        notes.contains("home     root: everything, user1: everything, user2: everything"),
        "{notes}"
    );
    assert!(notes.contains("filters  1"), "{notes}");
    assert!(notes.contains("schedule and counts"), "{notes}");
    assert!(!notes.contains("btrfs"), "{notes}");
}

#[test]
fn an_imported_config_builds_the_same_rsync_filters() {
    // So the first snapshot after the switch excludes what the ones before it did, and
    // `--link-dest` goes on hard-linking.
    let timeshift: serde_json::Value = serde_json::from_str(TIMESHIFT).unwrap();
    let timeshift_filters: Vec<String> = timeshift["exclude"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_owned())
        .collect();
    let import = import_timeshift(TIMESHIFT, &[], &[]).unwrap();
    let fstab = "UUID=x /recovery vfat umask=0077 0 0\n";
    assert_eq!(
        exclude::for_backup(&import.config.filters, fstab, &[]),
        exclude::for_backup(&timeshift_filters, fstab, &[])
    );
}

#[test]
fn import_notes_what_it_cant_use() {
    let btrfs = TIMESHIFT.replace("\"btrfs_mode\" : \"false\"", "\"btrfs_mode\" : \"true\"");
    assert_ne!(btrfs, TIMESHIFT);
    let notes = import_timeshift(&btrfs, &[], &[]).unwrap().notes.join("\n");
    assert!(notes.contains("btrfs mode"), "{notes}");
    assert!(notes.contains("not connected"), "{notes}");

    let luks = TIMESHIFT.replace(BACKUP_UUID, LUKS_UUID);
    let devices = parse_lsblk(LSBLK).unwrap();
    let notes = import_timeshift(&luks, &devices, &[])
        .unwrap()
        .notes
        .join("\n");
    assert!(notes.contains("can't be used, pick another"), "{notes}");

    let odd = r#"{"backup_device_uuid" : "", "exclude" : ["+ ", "*.mp3", "*.mp3", 5]}"#;
    let import = import_timeshift(odd, &[], &[]).unwrap();
    assert_eq!(import.config.filters, ["*.mp3"]);
    let notes = import.notes.join("\n");
    assert!(notes.contains("none set in Timeshift"), "{notes}");
    assert!(notes.contains("filters Apsis can't use"), "{notes}");
}

#[test]
fn the_file_wins_then_the_import_then_nothing() {
    let saved = config(BACKUP_UUID, &["*.iso"]);
    let (from_file, notes) = effective(Some(&saved.to_text()), Some(TIMESHIFT), &[], &[]).unwrap();
    assert_eq!((from_file, notes.len()), (saved, 0));

    let (imported, notes) = effective(None, Some(TIMESHIFT), &[], &[]).unwrap();
    assert_eq!(imported.backup_device_uuid, BACKUP_UUID);
    assert!(!notes.is_empty());

    // No timeshift.json: a first run starts empty, and the user picks a device.
    assert_eq!(
        effective(None, None, &[], &[]).unwrap(),
        (Config::default(), Vec::new())
    );
    // An unreadable timeshift.json isn't an error either.
    let (empty, notes) = effective(None, Some("[]"), &[], &[]).unwrap();
    assert_eq!(empty, Config::default());
    assert!(notes[0].starts_with("not imported"), "{notes:?}");
    // A broken config.toml is: it's Apsis's own and shouldn't be silently replaced.
    assert!(effective(Some("version = 9"), Some(TIMESHIFT), &[], &[]).is_err());
}
