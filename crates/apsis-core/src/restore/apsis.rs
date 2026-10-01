// SPDX-License-Identifier: GPL-3.0-only

//! Which Apsis a snapshot holds (PLAN 6b.2).
//!
//! Apsis's packaged files aren't protected: they come back as the snapshot has them, together
//! with the snapshot's dpkg database. So after a restore the installed Apsis is the
//! snapshot's, or none, and the dialog says what that means. Read from the text of the
//! snapshot's `var/lib/dpkg/status`, which the helper passes in.

/// The package's name in dpkg's database.
const PACKAGE: &str = "apsis";

/// What a restore leaves of Apsis. Each but [`InSnapshot::Current`] has a dialog line (PLAN
/// 6b.8's table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InSnapshot {
    /// An Apsis that restores (0.5 or later): nothing to say.
    Current,
    /// dpkg's database has no installed Apsis, or the snapshot has no database: Apsis is gone
    /// after the restore.
    NotInstalled,
    /// 0.4.x: reads the current settings, but can't restore or show how the restore went.
    NoRestore { version: String },
    /// 0.3.x or older, or a version that can't be read: it refuses the current settings.
    OldSettings { version: String },
}

/// Reads the text of a snapshot's `var/lib/dpkg/status`; `None` if it has no such file.
#[must_use]
pub fn in_snapshot(dpkg_status: Option<&str>) -> InSnapshot {
    let Some(full) = dpkg_status.and_then(installed_version) else {
        return InSnapshot::NotInstalled;
    };
    // dpkg's `[epoch:]upstream[-revision]`. Apsis's own version has neither `:` nor `-`.
    let upstream = full.split_once(':').map_or(full, |(_, rest)| rest);
    let upstream = upstream
        .rsplit_once('-')
        .map_or(upstream, |(first, _)| first);
    let version = upstream.to_owned();
    let mut numbers = upstream.split('.').map(str::parse::<u32>);
    match (numbers.next(), numbers.next()) {
        // Restore came with 0.5.
        (Some(Ok(major)), Some(Ok(minor))) if (major, minor) >= (0, 5) => InSnapshot::Current,
        // The settings file is version 2 since 0.4.0.
        (Some(Ok(0)), Some(Ok(4))) => InSnapshot::NoRestore { version },
        _ => InSnapshot::OldSettings { version },
    }
}

/// The `Version` of the first stanza for [`PACKAGE`] whose files are on disk (empty if it has
/// none), or `None` if there's no such stanza.
fn installed_version(dpkg_status: &str) -> Option<&str> {
    dpkg_status.split("\n\n").find_map(|stanza| {
        // A line that starts with a space continues the field above it.
        let field = |name: &str| {
            stanza
                .lines()
                .filter(|line| !line.starts_with([' ', '\t']))
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.trim())
        };
        // `Status: <want> <flag> <state>`. Removed, with or without its config files left,
        // means the programs are gone; any other state has them unpacked.
        let state = field("Status")?.split(' ').next_back()?;
        (field("Package")? == PACKAGE && !matches!(state, "not-installed" | "config-files"))
            .then(|| field("Version").unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stanza as dpkg writes one for the .deb (no maintainer line).
    fn stanza(package: &str, status: &str, version: &str) -> String {
        format!(
            "Package: {package}\n\
             Status: {status}\n\
             Priority: optional\n\
             Section: admin\n\
             Installed-Size: 21504\n\
             Architecture: amd64\n\
             Version: {version}\n\
             Depends: rsync, dbus, polkitd, libc6 (>= 2.39)\n\
             Description: System snapshot and restore\n \
             A COSMIC panel applet and window.\n \
             Version: 9.9.9 is only text here.\n"
        )
    }

    const INSTALLED: &str = "install ok installed";

    /// A database with other packages around Apsis's stanza.
    fn status(apsis: &str) -> String {
        [
            stanza("adduser", INSTALLED, "3.137ubuntu1"),
            apsis.to_owned(),
            stanza("zstd", INSTALLED, "1.5.5+dfsg2-2build1.1"),
        ]
        .join("\n")
    }

    fn read(apsis: &str) -> InSnapshot {
        in_snapshot(Some(&status(apsis)))
    }

    #[test]
    fn an_apsis_that_restores_needs_no_line() {
        for version in [
            "0.5.0-1", "0.5.3-1", "0.6.0-1", "1.0.0-1", "0.10.2-1", "0.5.0",
        ] {
            let found = read(&stanza("apsis", INSTALLED, version));
            assert_eq!(found, InSnapshot::Current, "{version}");
        }
    }

    #[test]
    fn a_snapshot_without_apsis_loses_it() {
        let none = [
            stanza("adduser", INSTALLED, "3.137ubuntu1"),
            stanza("zstd", INSTALLED, "1.5.5+dfsg2-2build1.1"),
        ]
        .join("\n");
        assert_eq!(in_snapshot(Some(&none)), InSnapshot::NotInstalled);
        assert_eq!(in_snapshot(Some("")), InSnapshot::NotInstalled);
    }

    #[test]
    fn a_snapshot_without_a_dpkg_database_has_no_apsis() {
        assert_eq!(in_snapshot(None), InSnapshot::NotInstalled);
    }

    /// Removed, or removed with its config files left: the programs aren't there.
    #[test]
    fn a_removed_apsis_isnt_installed() {
        for state in ["deinstall ok config-files", "purge ok not-installed"] {
            let found = read(&stanza("apsis", state, "0.5.0-1"));
            assert_eq!(found, InSnapshot::NotInstalled, "{state}");
        }
    }

    #[test]
    fn apsis_0_4_is_named_with_its_version() {
        for (version, shown) in [
            ("0.4.1-1", "0.4.1"),
            ("0.4.0-1", "0.4.0"),
            ("0.4.12", "0.4.12"),
        ] {
            assert_eq!(
                read(&stanza("apsis", INSTALLED, version)),
                InSnapshot::NoRestore {
                    version: shown.to_owned()
                }
            );
        }
    }

    #[test]
    fn apsis_0_3_or_older_cant_read_the_settings() {
        for (version, shown) in [
            ("0.3.2-1", "0.3.2"),
            ("0.1.0-1", "0.1.0"),
            ("0.3.9", "0.3.9"),
        ] {
            assert_eq!(
                read(&stanza("apsis", INSTALLED, version)),
                InSnapshot::OldSettings {
                    version: shown.to_owned()
                }
            );
        }
    }

    /// dpkg's `[epoch:]upstream[-revision]`: the upstream part is what's compared and shown.
    #[test]
    fn epoch_and_revision_arent_part_of_the_version() {
        assert_eq!(
            read(&stanza("apsis", INSTALLED, "1:0.4.1-2~pop1")),
            InSnapshot::NoRestore {
                version: "0.4.1".to_owned()
            }
        );
        assert_eq!(
            read(&stanza("apsis", INSTALLED, "2:0.5.0-1")),
            InSnapshot::Current
        );
    }

    /// The cautious line: it tells the user to install Apsis again.
    #[test]
    fn a_version_that_cant_be_read_counts_as_the_oldest() {
        for (version, shown) in [
            ("unknown", "unknown"),
            ("x.4.1-1", "x.4.1"),
            ("5", "5"),
            ("", ""),
        ] {
            assert_eq!(
                read(&stanza("apsis", INSTALLED, version)),
                InSnapshot::OldSettings {
                    version: shown.to_owned()
                },
                "{version:?}"
            );
        }
        let no_field = "Package: apsis\nStatus: install ok installed\n";
        assert_eq!(
            read(no_field),
            InSnapshot::OldSettings {
                version: String::new()
            }
        );
    }

    #[test]
    fn only_the_package_called_apsis_counts() {
        let others = [
            stanza("apsis-extras", INSTALLED, "0.3.0-1"),
            stanza("libapsis", INSTALLED, "0.3.0-1"),
        ]
        .join("\n");
        assert_eq!(read(&others), InSnapshot::NotInstalled);
        // A package that only mentions it.
        let mention = "Package: other\nStatus: install ok installed\nVersion: 1.0\n\
                       Recommends: apsis\nDescription: x\n Package: apsis\n";
        assert_eq!(read(mention), InSnapshot::NotInstalled);
    }

    /// Unpacked or half-configured: the files are there, so it's the Apsis the restore leaves.
    #[test]
    fn an_apsis_that_isnt_fully_configured_still_counts() {
        for state in ["install ok unpacked", "install ok half-configured"] {
            assert_eq!(
                read(&stanza("apsis", state, "0.4.1-1")),
                InSnapshot::NoRestore {
                    version: "0.4.1".to_owned()
                },
                "{state}"
            );
        }
    }

    #[test]
    fn field_names_are_read_in_any_case_and_values_trimmed() {
        let odd = "package:  apsis \nSTATUS: install ok installed\nversion:\t0.3.1-1 \n";
        assert_eq!(
            read(odd),
            InSnapshot::OldSettings {
                version: "0.3.1".to_owned()
            }
        );
    }
}
