// SPDX-License-Identifier: GPL-3.0-only

//! The restore's rsync command (PLAN 6b.6, step 3), as an argv run without a shell.

use std::ffi::OsString;
use std::path::Path;

/// rsync's messages and numbers are read as text ([`super::space`], the result's message), so
/// every run of it is in the C locale.
pub const LOCALE: (&str, &str) = ("LC_ALL", "C");

/// Options the restore never runs with, each for its own reason (PLAN 6b.6, 6b.10):
/// `--delete-excluded` would delete what the filter protects; `--ignore-errors` would delete
/// even after a read error, when rsync can't know what the snapshot holds; `-L` and
/// `--copy-links` would follow links; `--link-dest` and `-H` are for making snapshots.
pub const NEVER: [&str; 6] = [
    "--delete-excluded",
    "--ignore-errors",
    "-L",
    "--copy-links",
    "--link-dest",
    "-H",
];

/// rsync from a snapshot's `localhost/` over `target` (`/` for real), with the restore's
/// filter and its own log.
///
/// `old_format` is [`crate::native::Info::is_old_format`]: such a snapshot is restored without
/// `-A -X`. With `-X`, rsync would strip the live extended attributes (file capabilities)
/// even from unchanged files, to match a snapshot that never stored them.
///
/// Never any of [`NEVER`].
#[must_use]
pub fn rsync(
    localhost: &Path,
    target: &Path,
    filter: &Path,
    log: &Path,
    old_format: bool,
) -> Vec<OsString> {
    let run = [
        OsString::from("--info=progress2"),
        option("--log-file=", log),
    ];
    build(localhost, target, filter, old_format, run)
}

/// The same copy as a dry run, for the space check before anything is written (PLAN 6b.4):
/// the same flags and filter as [`rsync`], so `--stats` counts what the restore would copy.
/// `--no-human-readable` makes the sizes plain byte counts ([`super::space::dry_run_size`]).
/// Nothing is written, not even a log.
#[must_use]
pub fn rsync_dry_run(
    localhost: &Path,
    target: &Path,
    filter: &Path,
    old_format: bool,
) -> Vec<OsString> {
    let run = ["--dry-run", "--no-human-readable"].map(OsString::from);
    build(localhost, target, filter, old_format, run)
}

/// What the restore and its dry run share, with `run`'s options before the filter.
fn build(
    localhost: &Path,
    target: &Path,
    filter: &Path,
    old_format: bool,
    run: [OsString; 2],
) -> Vec<OsString> {
    let with_slash = |path: &Path| {
        let mut text = path.as_os_str().to_owned();
        if !text.as_encoded_bytes().ends_with(b"/") {
            text.push("/");
        }
        text
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
        ]
        .map(OsString::from),
    );
    argv.extend(run);
    argv.push(option("--exclude-from=", filter));
    argv.push(with_slash(localhost));
    argv.push(with_slash(target));
    argv
}

fn option(name: &str, value: &Path) -> OsString {
    let mut arg = OsString::from(name);
    arg.push(value);
    arg
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

    /// The dry run for the space check (PLAN 6b.4): the restore's own flags and filter, so it
    /// counts what the restore would copy, with plain byte counts and nothing written, not
    /// even a log.
    #[test]
    fn the_dry_run_is_the_restore_with_dry_run_and_plain_numbers() {
        let dry = |old_format| {
            rsync_dry_run(
                Path::new(LOCALHOST),
                Path::new("/"),
                Path::new(FILTER),
                old_format,
            )
        };
        assert_eq!(
            dry(false),
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
                "--dry-run",
                "--no-human-readable",
                "--exclude-from=/var/lib/apsis/restore/restore.filter",
                "/run/apsis/backup/timeshift/snapshots/2026-09-25_11-28-53/localhost/",
                "/",
            ]
            .map(OsString::from)
        );
        // It differs from the restore's argv by exactly these, for either format.
        for old_format in [false, true] {
            let without = |argv: Vec<OsString>, dropped: &[&str]| -> Vec<OsString> {
                argv.into_iter()
                    .filter(|arg| {
                        let arg = arg.to_str().unwrap();
                        !dropped.iter().any(|drop| arg.starts_with(drop))
                    })
                    .collect()
            };
            assert_eq!(
                without(dry(old_format), &["--dry-run", "--no-human-readable"]),
                without(argv(old_format), &["--info=progress2", "--log-file="])
            );
        }
    }

    /// With `--ignore-errors` rsync deletes even after a read error, when it can't know what
    /// the snapshot holds. Never, for the restore or its dry run (PLAN 6b.10).
    #[test]
    fn errors_are_never_ignored() {
        for old_format in [false, true] {
            let dry = rsync_dry_run(
                Path::new(LOCALHOST),
                Path::new("/"),
                Path::new(FILTER),
                old_format,
            );
            for argv in [argv(old_format), dry] {
                for arg in argv {
                    let arg = arg.to_str().unwrap().to_owned();
                    assert!(!arg.starts_with("--ignore-errors"), "{arg}");
                    assert!(!arg.starts_with("--force-delete"), "{arg}");
                }
            }
        }
        assert!(NEVER.contains(&"--ignore-errors"));
    }

    #[test]
    fn rsync_runs_in_the_c_locale() {
        assert_eq!(LOCALE, ("LC_ALL", "C"));
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
