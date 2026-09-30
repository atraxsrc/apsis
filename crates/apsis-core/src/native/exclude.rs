// SPDX-License-Identifier: GPL-3.0-only

//! The rsync filter list a snapshot is taken with, saved as its `exclude.list`.
//!
//! Timeshift builds it in `create_exclude_list_for_backup` and writes it in
//! `save_exclude_list_for_backup` (linuxmint/timeshift 24.01.1, `Main.vala:702-807` and
//! `:883-898`), from its defaults (`add_default_exclude_entries`, `Main.vala:538-669`), the
//! user's filters from `timeshift.json` (`load_app_config`, `Main.vala:3345-3360`),
//! `/etc/fstab` (`FsTabEntry.read_file`, `FsTabEntry.vala:59-121`) and the users in
//! `/etc/passwd` with their ecryptfs folders (`detect_encrypted_dirs`, `Main.vala:498`;
//! `SystemUser.vala:74-198`). Same order, same de-duplication, with Apsis's own config in the
//! user's part (see [`for_backup`]).
//!
//! 24.01.1 is the version this follows. 26.09.0 moves the user's filters to the front (so
//! rsync sees them first) and runs the per-user step after they're copied; see DECISIONS.md.

use std::fs;
use std::path::Path;

use super::info::{vala_int64, vala_strip};

/// `exclude_list_default` (`Main.vala:553-578`, `:614-624`), in order.
pub const DEFAULT: [&str; 35] = [
    "/dev/*",
    "/proc/*",
    "/sys/*",
    "/media/*",
    "/mnt/*",
    "/tmp/*",
    "/run/*",
    "/var/run/*",
    "/var/lock/*",
    "/var/lib/dhcpcd/*",
    "/var/lib/docker/*",
    "/var/lib/schroot/*",
    "/lost+found",
    "/timeshift/*",
    "/timeshift-btrfs/*",
    "/data/*",
    "/DATA/*",
    "/cdrom/*",
    "/sdcard/*",
    "/system/*",
    "/etc/timeshift.json",
    "/var/log/timeshift/*",
    "/var/log/timeshift-btrfs/*",
    "/swapfile",
    "/snap/*",
    "/root/.thumbnails",
    "/root/.cache",
    "/root/.dbus",
    "/root/.gvfs",
    "/root/.local/share/[Tt]rash",
    "/home/*/.thumbnails",
    "/home/*/.cache",
    "/home/*/.dbus",
    "/home/*/.gvfs",
    "/home/*/.local/share/[Tt]rash",
];

/// The fixed part of `exclude_list_default_extra` (`Main.vala:628-647`), after the fstab
/// entries.
pub const DEFAULT_EXTRA: [&str; 18] = [
    "/root/.mozilla/firefox/*.default/Cache",
    "/root/.mozilla/firefox/*.default/OfflineCache",
    "/root/.opera/cache",
    "/root/.kde/share/apps/kio_http/cache",
    "/root/.kde/share/cache/http",
    "/home/*/.mozilla/firefox/*.default/Cache",
    "/home/*/.mozilla/firefox/*.default/OfflineCache",
    "/home/*/.opera/cache",
    "/home/*/.kde/share/apps/kio_http/cache",
    "/home/*/.kde/share/cache/http",
    "/var/cache/apt/archives/*",
    "/var/cache/pacman/pkg/*",
    "/var/cache/yum/*",
    "/var/cache/dnf/*",
    "/var/cache/eopkg/*",
    "/var/cache/xbps/*",
    "/var/cache/zypp/*",
    "/var/cache/edb/*",
];

/// `exclude_list_home` (`Main.vala:653-654`).
pub const HOME: [&str; 2] = ["/root/**", "/home/*/**"];

/// Mount points that don't get an extra exclude: `/` and anything starting with these
/// (`Main.vala:580-612`; plain string prefixes, so `/homework` is skipped too).
const STANDARD_PREFIXES: [&str; 22] = [
    "/bin", "/boot", "/cdrom", "/dev", "/etc", "/home", "/lib", "/lib64", "/media", "/mnt", "/opt",
    "/proc", "/root", "/run", "/sbin", "/snap", "/srv", "/sys", "/system", "/tmp", "/usr", "/var",
];

/// A user Timeshift doesn't count as a system user, with their ecryptfs folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeUser {
    pub name: String,
    pub home: String,
    /// `/home/.ecryptfs/<name>/.ecryptfs/Private.mnt` names the home folder.
    pub encrypted_home: bool,
    /// Other folders `<home>/.ecryptfs/Private.mnt` names.
    pub encrypted_private_dirs: Vec<String>,
}

/// The users in `passwd` Timeshift's exclude list looks at, with their ecryptfs folders read
/// under `root` (`/` for real).
///
/// Timeshift keeps them in a hash map, so its order is the hash's; this keeps the file's. The
/// order only shows with two or more ecryptfs users or folders, or two or more users without
/// a home filter.
#[must_use]
pub fn home_users(passwd: &str, root: &Path) -> Vec<HomeUser> {
    let under_root = |path: &str| root.join(path.trim_start_matches('/'));
    let lines_of = |path: &str| {
        fs::read_to_string(under_root(path))
            .map(|text| {
                text.split('\n')
                    .map(vala_strip)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut users: Vec<HomeUser> = Vec::new();
    for (name, uid, home) in passwd_users(passwd) {
        // `is_system` (`SystemUser.vala:194-198`).
        if (uid != 0 && uid < 1000) || uid == 65534 || name == "PinguyBuilder" {
            continue;
        }
        // `check_encrypted_dirs` (`SystemUser.vala:146-192`).
        let home_mount = lines_of(&format!("/home/.ecryptfs/{name}/.ecryptfs/Private.mnt"));
        let private_mount = lines_of(&format!("{home}/.ecryptfs/Private.mnt"));
        let user = HomeUser {
            encrypted_home: home_mount.iter().any(|path| *path == home),
            encrypted_private_dirs: private_mount.into_iter().filter(|p| *p != home).collect(),
            name: name.to_owned(),
            home: home.to_owned(),
        };
        // A hash map by name: a later line with the same name replaces the earlier one.
        match users.iter_mut().find(|u| u.name == user.name) {
            Some(existing) => *existing = user,
            None => users.push(user),
        }
    }
    users
}

/// `read_users_from_file` / `parse_line_passwd` (`SystemUser.vala:74-143`): lines with
/// exactly seven `:`-separated fields, as (name, uid, home), stripped.
fn passwd_users(passwd: &str) -> impl Iterator<Item = (&str, i64, &str)> {
    passwd.split('\n').filter_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() == 7).then(|| {
            (
                vala_strip(fields[0]),
                vala_int64(fields[2]),
                vala_strip(fields[5]),
            )
        })
    })
}

/// The rsync filter list for a backup, as a list of patterns: Timeshift's order, with Apsis's
/// config in the user's part.
///
/// 1. Timeshift's defaults, the fstab mounts, the default extras.
/// 2. Decrypted ecryptfs contents (never backed up).
/// 3. The config's `filters`, as written (`+ x` / `- x`). In front of each `+` filter that is
///    an absolute path, its parent folders as `+ <dir>/` (the folder, not its contents): rsync
///    never looks inside an excluded folder, so `+ /home/user1/Videos/keep/***` with `/home`
///    left out needs `+ /home/`, `+ /home/user1/` and `+ /home/user1/Videos/` first. Not
///    Timeshift's; see DECISIONS.md, 0.4.0.
/// 4. `+ /root/**` if `include_root`, `+ /home/**` if `include_home`.
/// 5. The built-in `/root/**` and `/home/*/**`, then `/timeshift/*`.
///
/// So a filter beats the includes, and a `+` can't bring back a built-in exclude (they come
/// first). `users` are only for the ecryptfs folders; there is no per-user home step (Apsis
/// 0.3 had Timeshift's, see [`crate::config::convert`]).
#[must_use]
pub fn for_backup(
    filters: &[String],
    include_root: bool,
    include_home: bool,
    fstab: &str,
    users: &[HomeUser],
) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    let add = |list: &mut Vec<String>, pattern: String| {
        if !list.contains(&pattern) {
            list.push(pattern);
        }
    };
    for pattern in DEFAULT {
        add(&mut list, pattern.to_owned());
    }
    for mount in fstab_mount_points(fstab) {
        add(&mut list, format!("{mount}/*"));
    }
    for pattern in DEFAULT_EXTRA {
        add(&mut list, pattern.to_owned());
    }
    // Decrypted ecryptfs contents are never backed up (`Main.vala:724-749`). Added without
    // the duplicate check the other steps have.
    for user in users {
        if user.encrypted_home {
            list.push(format!("{}/**", user.home));
        }
        for dir in &user.encrypted_private_dirs {
            list.push(format!("{dir}/**"));
        }
    }
    for filter in filters {
        if let Some(body) = filter.strip_prefix("+ ") {
            for parent in parent_folders(body) {
                add(&mut list, format!("+ {parent}"));
            }
        }
        add(&mut list, filter.clone());
    }
    if include_root {
        add(&mut list, "+ /root/**".to_owned());
    }
    if include_home {
        add(&mut list, "+ /home/**".to_owned());
    }
    for pattern in HOME {
        add(&mut list, pattern.to_owned());
    }
    add(&mut list, "/timeshift/*".to_owned());
    list
}

/// The folders above an absolute pattern's last part, each as `/a/`, `/a/b/`: what rsync has
/// to be let into to reach it. None for a relative pattern, and none from a part with `**` on
/// (it can't be spelled as one folder).
fn parent_folders(pattern: &str) -> Vec<String> {
    let Some(path) = pattern.strip_prefix('/') else {
        return Vec::new();
    };
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    // `dir/***` matches `dir` itself too: its parents start one up.
    let last = if parts.last() == Some(&"***") { 2 } else { 1 };
    let mut folders = Vec::new();
    let mut prefix = String::from("/");
    for part in &parts[..parts.len().saturating_sub(last)] {
        if part.is_empty() || part.contains("**") {
            break;
        }
        prefix.push_str(part);
        prefix.push('/');
        folders.push(prefix.clone());
    }
    folders
}

/// `exclude.list`'s text: each pattern that isn't blank, then `\n`. The pattern itself is
/// written as is, unstripped.
#[must_use]
pub fn to_text(patterns: &[String]) -> String {
    let mut text = String::new();
    for pattern in patterns.iter().filter(|p| !vala_strip(p).is_empty()) {
        text.push_str(pattern);
        text.push('\n');
    }
    text
}

/// Mount points in `/etc/fstab` that get their own `<mount>/*` exclude, in file order.
fn fstab_mount_points(fstab: &str) -> Vec<&str> {
    fstab
        .split('\n')
        .filter(|line| {
            let stripped = vala_strip(line);
            !stripped.is_empty() && !stripped.starts_with('#')
        })
        .filter_map(|line| {
            // `line.replace("\t"," ").split(" ")`, skipping empty parts; the second is the
            // mount point.
            line.split([' ', '\t'])
                .map(vala_strip)
                .filter(|part| !part.is_empty())
                .nth(1)
        })
        .filter(|mount| mount.starts_with('/'))
        .filter(|mount| *mount != "/" && !STANDARD_PREFIXES.iter().any(|p| mount.starts_with(p)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn defaults_alone() {
        let list = for_backup(&[], false, false, "", &[]);
        let expected: Vec<String> = DEFAULT
            .iter()
            .chain(DEFAULT_EXTRA.iter())
            .chain(HOME.iter())
            .map(|p| (*p).to_owned())
            .collect();
        assert_eq!(list, expected);
        // `/timeshift/*` is a default already, so it isn't added twice.
        assert_eq!(list.iter().filter(|p| *p == "/timeshift/*").count(), 1);
    }

    #[test]
    fn filters_come_after_the_defaults_then_the_includes_then_the_homes() {
        let filters = strings(&["- /var/lib/libvirt/**", "- *.iso", "- /proc/*", "- *.iso"]);
        let list = for_backup(&filters, true, true, "", &[]);
        let at = DEFAULT.len() + DEFAULT_EXTRA.len();
        assert_eq!(
            list[at..],
            strings(&[
                "- /var/lib/libvirt/**",
                "- *.iso",
                "- /proc/*",
                "+ /root/**",
                "+ /home/**",
                "/root/**",
                "/home/*/**"
            ])
        );
        // Neither include: only the built-in home excludes.
        let list = for_backup(&[], false, false, "", &[]);
        assert!(!list.iter().any(|p| p.starts_with('+')));
    }

    #[test]
    fn a_kept_folder_gets_its_parents_let_in_first() {
        let filters = strings(&[
            "+ /home/user1/Videos/keep/***",
            "- /home/*/Videos/***",
            "+ /root/.ssh",
            "+ *.conf",
            "+ /srv/**/keep",
        ]);
        let list = for_backup(&filters, false, false, "", &[]);
        let at = DEFAULT.len() + DEFAULT_EXTRA.len();
        assert_eq!(
            list[at..],
            strings(&[
                "+ /home/",
                "+ /home/user1/",
                "+ /home/user1/Videos/",
                "+ /home/user1/Videos/keep/***",
                "- /home/*/Videos/***",
                "+ /root/",
                "+ /root/.ssh",
                "+ *.conf",
                "+ /srv/",
                "+ /srv/**/keep",
                "/root/**",
                "/home/*/**"
            ])
        );
        assert_eq!(parent_folders("/a/b/"), ["/a/"]);
        assert_eq!(parent_folders("/a/b/***"), ["/a/"]);
        assert!(parent_folders("relative/x").is_empty());
        assert!(parent_folders("/top").is_empty());
    }

    #[test]
    fn non_standard_fstab_mounts_are_excluded() {
        let fstab = "# /etc/fstab\n\
            UUID=0000 / ext4 defaults 0 1\n\
            UUID=0001 /boot/efi vfat umask=0077 0 0\n\
            UUID=0002\t/games\text4\tdefaults 0 2\n\
            /swapfile none swap sw 0 0\n\
            UUID=0003 /homework ext4 defaults 0 2\n\
            \n\
            UUID=0004   /srv2   ext4 defaults 0 2\n\
            UUID=0005 /games ext4 defaults 0 2\n\
            tmpfs relative tmpfs defaults 0 0";
        let list = for_backup(&[], false, false, fstab, &[]);
        let defaults = DEFAULT.len();
        assert_eq!(list[defaults..defaults + 1], strings(&["/games/*"]));
        // `/srv2` starts with `/srv`: skipped, like `/homework`.
        assert!(
            !list
                .iter()
                .any(|p| p.starts_with("/srv2") || p.starts_with("/homework"))
        );
        assert_eq!(list[defaults + 1], DEFAULT_EXTRA[0]);
    }

    #[test]
    fn ecryptfs_folders_come_before_the_user_filters() {
        let root = std::env::temp_dir().join(format!("apsis-exclude-{}", std::process::id()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "home/.ecryptfs/user2/.ecryptfs/Private.mnt",
            "/home/user2\n",
        );
        write("home/user3/.ecryptfs/Private.mnt", "/home/user3/Private\n");
        let passwd = "root:x:0:0:root:/root:/bin/bash\n\
            daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n\
            user1:x:1000:1000:,,,:/home/user1:/bin/bash\n\
            user2:x:1001:1001::/home/user2:/bin/bash\n\
            user3:x:1002:1002::/home/user3:/bin/bash\n\
            nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n\
            broken:x:1003\n";
        let users = home_users(passwd, &root);
        fs::remove_dir_all(&root).unwrap();
        let names: Vec<&str> = users.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["root", "user1", "user2", "user3"]);
        assert!(users[2].encrypted_home && !users[1].encrypted_home);
        assert_eq!(users[3].encrypted_private_dirs, ["/home/user3/Private"]);

        let list = for_backup(
            &strings(&["+ /home/user2/.config/***"]),
            false,
            true,
            "",
            &users,
        );
        let at = DEFAULT.len() + DEFAULT_EXTRA.len();
        // Decrypted contents stay out, even of an included /home and before a `+` filter.
        assert_eq!(
            list[at..],
            strings(&[
                "/home/user2/**",
                "/home/user3/Private/**",
                "+ /home/",
                "+ /home/user2/",
                "+ /home/user2/.config/***",
                "+ /home/**",
                "/root/**",
                "/home/*/**"
            ])
        );
    }

    #[test]
    fn text_is_one_pattern_per_line_blank_ones_dropped() {
        let text = to_text(&strings(&["/a", "  ", "+ /b/** ", ""]));
        assert_eq!(text, "/a\n+ /b/** \n");
    }
}
