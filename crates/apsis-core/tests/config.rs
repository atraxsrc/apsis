// SPDX-License-Identifier: GPL-3.0-only

//! Apsis's own config (`/etc/apsis/config.toml`), the conversion of old settings (Apsis 0.3's
//! version 1, and Timeshift's), and the one-time import from Timeshift's settings.

use std::fs;
use std::path::Path;
use std::process::Command;

use apsis_core::Error;
use apsis_core::config::{
    Config, Legacy, Stored, System, convert, effective, import_timeshift, validate,
    validate_signed_filter,
};
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
        ..Config::default()
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn legacy(filters: &[&str]) -> Legacy {
    Legacy {
        backup_device_uuid: BACKUP_UUID.to_owned(),
        filters: strings(filters),
    }
}

/// root, user1 and user2, and `/home` holding just their homes.
fn system() -> System {
    System {
        users: users(),
        home_entries: strings(&["user1", "user2", "lost+found"]),
    }
}

fn invalid(result: apsis_core::Result<()>) -> String {
    match result {
        Err(Error::InvalidSettings(reason)) => reason,
        other => panic!("expected InvalidSettings, got {other:?}"),
    }
}

fn current(text: &str) -> Config {
    match Config::read(text).unwrap() {
        Stored::Current(config) => config,
        other => panic!("not version 2: {other:?}"),
    }
}

#[test]
fn the_file_reads_back_what_it_writes() {
    let written = Config {
        backup_device_uuid: BACKUP_UUID.to_owned(),
        include_root: false,
        include_home: true,
        filters: strings(&[
            "+ /home/user1/**",
            "- /var/lib/libvirt/**",
            "- \"quoted\" \\ back",
            "- ünïcode",
        ]),
    };
    let text = written.to_text();
    assert!(text.starts_with("# Written by apsis-helper"));
    assert!(text.contains("version = 2\n"));
    assert!(text.contains("include_home = true\n"));
    assert!(text.contains(&format!("backup_device_uuid = \"{BACKUP_UUID}\"\n")));
    assert_eq!(current(&text), written);
    // Missing fields are the defaults: /root in, /home out.
    let fresh = current("version = 2");
    assert_eq!(fresh, Config::default());
    assert!(fresh.include_root && !fresh.include_home);
}

#[test]
fn version_1_is_read_for_converting() {
    let text = "version = 1\nbackup_device_uuid = \"x\"\nfilters = [\"+ /root/**\", \"*.mp3\"]\n";
    assert_eq!(
        Config::read(text).unwrap(),
        Stored::Legacy(Legacy {
            backup_device_uuid: "x".to_owned(),
            filters: strings(&["+ /root/**", "*.mp3"]),
        })
    );
    assert_eq!(Config::read(text).unwrap().device(), "x");
}

#[test]
fn files_it_cant_read_safely_are_refused() {
    for bad in [
        "",
        "version = 3",
        "version = \"2\"",
        "version = 2\nbackup_device_uuid = 5",
        "version = 2\nfilters = \"- *.mp3\"",
        "version = 2\nfilters = [1]",
        "version = 2\ninclude_home = \"yes\"",
        "not toml at all [",
    ] {
        assert!(
            matches!(Config::read(bad), Err(Error::InvalidConfig(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn a_new_device_must_be_a_connected_unencrypted_linux_filesystem() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let saved = config(BACKUP_UUID, &[]);
    // Unchanged: fine, even with the disk unplugged.
    validate(&saved, Some(BACKUP_UUID), &[]).unwrap();
    // A first save counts as a change.
    validate(&saved, None, &devices).unwrap();
    assert!(invalid(validate(&saved, None, &[])).contains("isn't connected"));
    assert!(invalid(validate(&config("", &[]), None, &devices)).contains("choose"));
    for (uuid, why) in [
        (LUKS_UUID, "unencrypted"),
        (UNLOCKED_UUID, "unencrypted"),
        ("AAAA-0001", "unencrypted Linux filesystem"),
    ] {
        let reason = invalid(validate(&config(uuid, &[]), Some(BACKUP_UUID), &devices));
        assert!(reason.contains(why), "{uuid}: {reason}");
    }
    // Another plain Linux filesystem is fine (btrfs too: Apsis takes rsync snapshots onto it).
    validate(&config(SYSTEM_UUID, &[]), Some(BACKUP_UUID), &devices).unwrap();
    validate(&config(BTRFS_UUID, &[]), Some(BACKUP_UUID), &devices).unwrap();
}

#[test]
fn filters_need_a_sign_and_arent_repeated() {
    let twice = config(BACKUP_UUID, &["- *.mp3", "- /a", "- *.mp3"]);
    assert!(invalid(validate(&twice, Some(BACKUP_UUID), &[])).contains("twice"));
    for (bad, why) in [
        ("*.mp3", "starts with"),
        ("+ ", "blank"),
        ("-   ", "blank"),
        ("- a\nb", "control"),
        ("+a", "starts with"),
    ] {
        let reason = invalid(validate_signed_filter(bad));
        assert!(reason.contains(why), "{bad:?}: {reason}");
    }
    validate_signed_filter("- #not a comment").unwrap();
    validate_signed_filter(&format!("- /{}", "x".repeat(4000))).unwrap();
    assert!(validate_signed_filter(&format!("- /{}", "x".repeat(4096))).is_err());
}

#[test]
fn every_home_everything_becomes_the_two_includes() {
    // The real file: root and both users "everything", and a filter that isn't a home's.
    let import = import_timeshift(TIMESHIFT, &[]).unwrap();
    let (config, notes) = convert(&import.legacy, &system());
    assert!(config.include_root && config.include_home);
    assert_eq!(config.filters, ["- /var/lib/libvirt/**"]);
    let notes = notes.join("\n");
    assert!(notes.contains("/root    included"), "{notes}");
    assert!(notes.contains("/home    included (every home"), "{notes}");
    assert!(notes.contains("filters  1"), "{notes}");
}

#[test]
fn nothing_about_homes_leaves_both_out() {
    let (config, _) = convert(&legacy(&["*.iso", "/home/user1/**"]), &system());
    assert!(!config.include_root && !config.include_home);
    assert_eq!(config.filters, ["- *.iso"]);
}

#[test]
fn hidden_files_only_becomes_a_visible_row_in_place() {
    let (config, notes) = convert(
        &legacy(&[
            "/a",
            "+ /home/user2/.**",
            "+ /root/.**",
            "+ /home/user1/**",
            "/b",
        ]),
        &system(),
    );
    assert!(!config.include_root && !config.include_home);
    assert_eq!(
        config.filters,
        [
            "- /a",
            "+ /home/user2/.**",
            "+ /root/.**",
            "+ /home/user1/**",
            "- /b"
        ]
    );
    let notes = notes.join("\n");
    assert!(
        notes.contains("kept as a filter  + /home/user2/.** (user2)"),
        "{notes}"
    );
    assert!(
        notes.contains("/root    not included; its hidden files"),
        "{notes}"
    );
}

#[test]
fn a_folder_in_home_that_isnt_a_users_keeps_the_rows() {
    let all = ["+ /home/user1/**", "+ /home/user2/**"];
    let mut shared = system();
    shared.home_entries.push("shared".to_owned());
    let (config, notes) = convert(&legacy(&all), &shared);
    assert!(!config.include_home);
    assert_eq!(config.filters, all);
    assert!(notes.join("\n").contains("/home/shared"), "{notes:?}");
}

#[test]
fn a_filter_the_home_include_used_to_shadow_keeps_the_rows() {
    // `*.iso` after `+ /home/user1/**` never applied inside user1's home: with /home included
    // after all filters it would.
    let list = ["+ /home/user1/**", "+ /home/user2/**", "*.iso"];
    let (config, notes) = convert(&legacy(&list), &system());
    assert!(!config.include_home);
    assert_eq!(
        config.filters,
        ["+ /home/user1/**", "+ /home/user2/**", "- *.iso"]
    );
    assert!(
        notes.join("\n").contains("- *.iso would then apply"),
        "{notes:?}"
    );
    // Anchored outside the homes is fine.
    let list = ["+ /home/user1/**", "+ /home/user2/**", "/var/tmp/**"];
    let (config, _) = convert(&legacy(&list), &system());
    assert!(config.include_home);
    assert_eq!(config.filters, ["- /var/tmp/**"]);
    // Before the includes is fine too (it already applied inside the homes).
    let list = ["*.iso", "+ /home/user1/**", "+ /home/user2/**"];
    let (config, _) = convert(&legacy(&list), &system());
    assert!(config.include_home);
    assert_eq!(config.filters, ["- *.iso"]);
}

#[test]
fn a_home_elsewhere_that_was_left_out_stays_out() {
    let mut odd = system();
    odd.users.push(User {
        name: "svc".to_owned(),
        home: "/srv/svc".to_owned(),
        encrypted_home: false,
    });
    let (config, _) = convert(&legacy(&["*.iso"]), &odd);
    assert_eq!(config.filters, ["- *.iso", "- /srv/svc/**"]);
}

#[test]
fn ecryptfs_homes_convert_by_their_own_patterns() {
    let mut encrypted = system();
    encrypted.users[2].encrypted_home = true;
    encrypted.home_entries.push(".ecryptfs".to_owned());
    let list = ["+ /home/user1/**", "+ /home/.ecryptfs/user2/***"];
    let (config, _) = convert(&legacy(&list), &encrypted);
    assert!(config.include_home);
    assert!(config.filters.is_empty());
}

#[test]
fn import_takes_the_device_and_the_old_list_in_order() {
    let devices = parse_lsblk(LSBLK).unwrap();
    let import = import_timeshift(TIMESHIFT, &devices).unwrap();
    assert_eq!(import.legacy.backup_device_uuid, BACKUP_UUID);
    assert_eq!(
        import.legacy.filters,
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
    assert!(notes.contains("schedule and counts"), "{notes}");
    assert!(!notes.contains("btrfs"), "{notes}");
}

#[test]
fn import_notes_what_it_cant_use() {
    let btrfs = TIMESHIFT.replace("\"btrfs_mode\" : \"false\"", "\"btrfs_mode\" : \"true\"");
    assert_ne!(btrfs, TIMESHIFT);
    let notes = import_timeshift(&btrfs, &[]).unwrap().notes.join("\n");
    assert!(notes.contains("btrfs mode"), "{notes}");
    assert!(notes.contains("not connected"), "{notes}");

    let luks = TIMESHIFT.replace(BACKUP_UUID, LUKS_UUID);
    let devices = parse_lsblk(LSBLK).unwrap();
    let notes = import_timeshift(&luks, &devices).unwrap().notes.join("\n");
    assert!(notes.contains("can't be used, pick another"), "{notes}");

    let odd = r#"{"backup_device_uuid" : "", "exclude" : ["+ ", "*.mp3", "*.mp3", 5]}"#;
    let import = import_timeshift(odd, &[]).unwrap();
    assert_eq!(import.legacy.filters, ["*.mp3"]);
    let notes = import.notes.join("\n");
    assert!(notes.contains("none set in Timeshift"), "{notes}");
    assert!(notes.contains("filters Apsis can't use"), "{notes}");
}

#[test]
fn the_file_wins_then_the_import_then_the_defaults() {
    let saved = config(BACKUP_UUID, &["- *.iso"]);
    let (from_file, notes) =
        effective(Some(&saved.to_text()), Some(TIMESHIFT), &[], &system()).unwrap();
    assert_eq!((from_file, notes.len()), (saved, 0));

    // Version 1 is converted, with notes, until it's saved.
    let old = "version = 1\nbackup_device_uuid = \"x\"\nfilters = [\"+ /home/user2/.**\"]\n";
    let (converted, notes) = effective(Some(old), None, &[], &system()).unwrap();
    assert_eq!(converted.filters, ["+ /home/user2/.**"]);
    assert!(notes[0].starts_with("settings from Apsis 0.3"), "{notes:?}");

    let (imported, notes) = effective(None, Some(TIMESHIFT), &[], &system()).unwrap();
    assert_eq!(imported.backup_device_uuid, BACKUP_UUID);
    assert!(imported.include_home);
    assert!(notes[0].starts_with("imported from"), "{notes:?}");

    // No timeshift.json: a first run starts with the defaults.
    assert_eq!(
        effective(None, None, &[], &system()).unwrap(),
        (Config::default(), Vec::new())
    );
    // An unreadable timeshift.json isn't an error either.
    let (empty, notes) = effective(None, Some("[]"), &[], &system()).unwrap();
    assert_eq!(empty, Config::default());
    assert!(notes[0].starts_with("not imported"), "{notes:?}");
    // A broken config.toml is: it's Apsis's own and shouldn't be silently replaced.
    assert!(effective(Some("version = 9"), Some(TIMESHIFT), &[], &system()).is_err());
}

/// What rsync copies from `tree` with the filter `list` (a dry run; the file names only).
fn copied(tree: &Path, list: &[String]) -> Vec<String> {
    let dir = tree.parent().unwrap();
    let list_file = dir.join(format!("list-{}", list.len()));
    fs::write(&list_file, exclude::to_text(list)).unwrap();
    let out = Command::new("rsync")
        .args(["-a", "--dry-run", "--out-format=%n"])
        .arg(format!("--exclude-from={}", list_file.display()))
        .arg(format!("{}/", tree.display()))
        .arg(dir.join("dest"))
        .output()
        .expect("rsync");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut names: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

#[test]
fn a_converted_config_copies_exactly_what_the_old_list_did() {
    // The real snapshot's exclude.list (0.3.x, Timeshift's builder), against the v2 list built
    // from the converted real settings: rsync must copy the same files from the same tree.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/convert-equal");
    let _ = fs::remove_dir_all(&dir);
    let tree = dir.join("tree");
    for file in [
        "etc/hosts",
        "root/notes",
        "root/.bashrc",
        "root/.cache/x",
        "home/user1/doc.txt",
        "home/user1/.config/app",
        "home/user1/.cache/c",
        "home/user2/doc.txt",
        "var/lib/libvirt/disk.img",
        "var/lib/other/keep",
        "recovery/x",
        "proc/1/status",
        "tmp/x",
    ] {
        let path = tree.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, file).unwrap();
    }
    fs::create_dir_all(tree.join("home/lost+found")).unwrap();
    let fstab = "PARTUUID=0002 /recovery vfat umask=0077 0 0\n";
    let old_user = strings(&[
        "+ /root/**",
        "+ /home/user1/**",
        "/var/lib/libvirt/**",
        "+ /home/user2/**",
    ]);
    // Apsis 0.3's list: Timeshift's order, the user part as it was.
    let mut old = exclude::for_backup(&[], false, false, fstab, &[]);
    let at = old.iter().position(|p| p == "/root/**").unwrap();
    old.splice(at..at, old_user);
    let import = import_timeshift(TIMESHIFT, &[]).unwrap();
    let (config, _) = convert(&import.legacy, &system());
    let new = exclude::for_backup(
        &config.filters,
        config.include_root,
        config.include_home,
        fstab,
        &[],
    );
    let before = copied(&tree, &old);
    assert!(
        before.iter().any(|n| n == "home/user1/doc.txt"),
        "{before:?}"
    );
    assert!(
        !before.iter().any(|n| n.contains("libvirt/disk")),
        "{before:?}"
    );
    assert_eq!(copied(&tree, &new), before);
    fs::remove_dir_all(&dir).unwrap();
}
