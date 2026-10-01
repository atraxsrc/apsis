// SPDX-License-Identifier: GPL-3.0-only

//! The restore's rsync command (PLAN 6b.6, step 3), as an argv run without a shell.

use std::ffi::OsString;
use std::path::Path;

/// rsync from a snapshot's `localhost/` over `target` (`/` for real), with the restore's
/// filter and its own log.
///
/// `old_format` is [`crate::native::Info::is_old_format`]: such a snapshot is restored without
/// `-A -X`. With `-X`, rsync would strip the live extended attributes (file capabilities)
/// even from unchanged files, to match a snapshot that never stored them.
///
/// Never `--delete-excluded` (an excluded path stays as it is on the live system), `-L`,
/// `--link-dest` or `-H`.
#[must_use]
pub fn rsync(
    localhost: &Path,
    target: &Path,
    filter: &Path,
    log: &Path,
    old_format: bool,
) -> Vec<OsString> {
    let with_slash = |path: &Path| {
        let mut text = path.as_os_str().to_owned();
        if !text.as_encoded_bytes().ends_with(b"/") {
            text.push("/");
        }
        text
    };
    let option = |name: &str, value: &Path| {
        let mut arg = OsString::from(name);
        arg.push(value);
        arg
    };
    let mut argv: Vec<OsString> = vec!["rsync".into(), "-a".into()];
    if !old_format {
        argv.extend(["-A".into(), "-X".into()]);
    }
    argv.extend(
        [
            "--numeric-ids",
            "--delete",
            "--force",
            "--sparse",
            "--stats",
            "--info=progress2",
        ]
        .map(OsString::from),
    );
    argv.push(option("--log-file=", log));
    argv.push(option("--exclude-from=", filter));
    argv.push(with_slash(localhost));
    argv.push(with_slash(target));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native;

    const LOCALHOST: &str = "/run/apsis/backup/timeshift/snapshots/2026-09-25_11-28-53/localhost";
    const FILTER: &str = "/var/lib/apsis/restore/restore.filter";
    const LOG: &str = "/var/lib/apsis/restore/rsync-log";

    fn argv(old_format: bool) -> Vec<OsString> {
        rsync(
            Path::new(LOCALHOST),
            Path::new("/"),
            Path::new(FILTER),
            Path::new(LOG),
            old_format,
        )
    }

    #[test]
    fn a_new_format_snapshot_is_restored_with_acls_and_xattrs() {
        assert_eq!(
            argv(false),
            [
                "rsync",
                "-a",
                "-A",
                "-X",
                "--numeric-ids",
                "--delete",
                "--force",
                "--sparse",
                "--stats",
                "--info=progress2",
                "--log-file=/var/lib/apsis/restore/rsync-log",
                "--exclude-from=/var/lib/apsis/restore/restore.filter",
                "/run/apsis/backup/timeshift/snapshots/2026-09-25_11-28-53/localhost/",
                "/",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn an_old_format_snapshot_is_restored_without_them() {
        assert_eq!(
            argv(true),
            [
                "rsync",
                "-a",
                "--numeric-ids",
                "--delete",
                "--force",
                "--sparse",
                "--stats",
                "--info=progress2",
                "--log-file=/var/lib/apsis/restore/rsync-log",
                "--exclude-from=/var/lib/apsis/restore/restore.filter",
                "/run/apsis/backup/timeshift/snapshots/2026-09-25_11-28-53/localhost/",
                "/",
            ]
            .map(OsString::from)
        );
    }

    /// Both paths end in `/`: rsync copies the folder's contents, not the folder.
    #[test]
    fn source_and_target_end_in_a_slash() {
        let argv = rsync(
            Path::new("/b/localhost/"),
            Path::new("/mnt"),
            Path::new("/f"),
            Path::new("/l"),
            false,
        );
        assert_eq!(argv[argv.len() - 2], "/b/localhost/");
        assert_eq!(argv[argv.len() - 1], "/mnt/");
    }

    /// The restore's excludes protect: with `--delete-excluded`, rsync would delete every
    /// excluded path on the live system (the ESP, other disks, `/home`). Create keeps it: a
    /// snapshot mustn't hold what its filters leave out.
    #[test]
    fn the_restore_never_deletes_excluded_files_and_create_still_does() {
        for old_format in [false, true] {
            let argv = argv(old_format);
            assert!(argv.iter().any(|a| a == "--delete"), "{argv:?}");
            assert!(!argv.iter().any(|a| a == "--delete-excluded"), "{argv:?}");
        }
        let create = native::rsync_argv(Path::new("/"), Path::new("/s/x"), None);
        assert!(
            create.iter().any(|a| a == "--delete-excluded"),
            "{create:?}"
        );
    }

    #[test]
    fn nothing_follows_links_or_links_files() {
        for old_format in [false, true] {
            for arg in argv(old_format) {
                let arg = arg.to_string_lossy();
                let short = arg.starts_with('-') && !arg.starts_with("--");
                assert!(!(short && arg.contains(['L', 'H', 'K', 'k'])), "{arg}");
                for long in [
                    "--copy-links",
                    "--copy-dirlinks",
                    "--copy-unsafe-links",
                    "--keep-dirlinks",
                    "--hard-links",
                    "--link-dest",
                    "--delete-excluded",
                ] {
                    assert!(!arg.starts_with(long), "{arg}");
                }
            }
        }
    }
}
