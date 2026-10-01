// SPDX-License-Identifier: GPL-3.0-only

//! The files the arm writes under `/etc/systemd/system/` (PLAN 6b.6): the restore unit, and
//! the drop-in that keeps Pop!_OS's release upgrade from running in the restore's boot. The
//! texts live here, byte for byte, so the helper writes exactly what the tests say.
//!
//! Neither is shipped in `/usr`: a snapshot older than Apsis would delete a packaged file
//! mid-restore, and a retry boot needs both. Both are on the protect list
//! ([`super::filter::PROTECTED`]) and are removed with the arm.

/// The restore unit, written on arm, read by systemd at the next boot (no `daemon-reload`).
pub const UNIT_PATH: &str = "/etc/systemd/system/apsis-restore.service";

/// The link that makes `system-update.target` want the unit.
pub const UNIT_WANTS_LINK: &str =
    "/etc/systemd/system/system-update.target.wants/apsis-restore.service";

/// The helper copy the unit runs: `/usr/libexec/apsis-helper` copied on arm, for the same
/// reason the unit isn't packaged.
pub const HELPER_COPY: &str = "/var/lib/apsis/restore/apsis-helper";

/// The drop-in for Pop!_OS's `pop-upgrade-init.service` (in `/usr/lib`), written on arm,
/// removed with the unit. Protected, so it's still there in a retry boot after a copy that
/// restored a snapshot without it.
pub const DROP_IN_PATH: &str = "/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf";

/// The unit's text (PLAN 6b.6). `StandardOutput=journal`, not `journal+console`: what the
/// person sees goes through plymouth only. `FailureAction=reboot`, not `OnFailure=`. Never
/// `KillMode=none`. No `Before=` or `Conflicts=` against the other offline-update units.
#[must_use]
pub fn unit_text() -> &'static str {
    "[Unit]\n\
     Description=Apsis: restore the system from a snapshot\n\
     DefaultDependencies=no\n\
     Requires=sysinit.target\n\
     After=sysinit.target system-update-pre.target local-fs.target\n\
     Before=system-update.target shutdown.target\n\
     ConditionPathIsSymbolicLink=/system-update\n\
     FailureAction=reboot\n\
     \n\
     [Service]\n\
     Type=oneshot\n\
     ExecStart=/var/lib/apsis/restore/apsis-helper --apply-restore\n\
     StandardOutput=journal\n"
}

/// The drop-in's text (PLAN 6b.6, "pop-upgrade-init"). The condition is a path through the
/// link: `/system-update` is a symlink to `/var/lib/apsis/restore`, `ConditionPathExists=`
/// follows it, so the path exists exactly while the link points at Apsis's folder, where the
/// helper copy is. Nothing is made or removed to keep it true; the link is the one commit
/// point. Without the link the path doesn't resolve and the condition passes: a leaked
/// drop-in is inert.
#[must_use]
pub fn drop_in_text() -> &'static str {
    "# Written by apsis-helper while a restore is armed, removed when it ends.\n\
     # Pop!_OS's release upgrade must not run in the boot that applies an Apsis restore.\n\
     # The path below exists exactly while /system-update is Apsis's link.\n\
     [Unit]\n\
     ConditionPathExists=!/system-update/apsis-helper\n"
}

#[cfg(test)]
mod tests {
    use super::super::filter::PROTECTED;
    use super::*;

    #[test]
    fn the_unit_is_byte_for_byte_the_plans() {
        let expected = "\
[Unit]
Description=Apsis: restore the system from a snapshot
DefaultDependencies=no
Requires=sysinit.target
After=sysinit.target system-update-pre.target local-fs.target
Before=system-update.target shutdown.target
ConditionPathIsSymbolicLink=/system-update
FailureAction=reboot

[Service]
Type=oneshot
ExecStart=/var/lib/apsis/restore/apsis-helper --apply-restore
StandardOutput=journal
";
        assert_eq!(unit_text(), expected);
        assert_eq!(unit_text().as_bytes(), expected.as_bytes());
    }

    #[test]
    fn the_unit_keeps_the_rules_the_design_settled() {
        let unit = unit_text();
        let lines: Vec<&str> = unit.lines().collect();
        assert!(lines.contains(&"StandardOutput=journal"));
        assert!(!unit.contains("journal+console"), "plymouth only");
        assert!(lines.contains(&"FailureAction=reboot"));
        assert!(!unit.contains("OnFailure="));
        assert!(!unit.contains("KillMode"), "never KillMode=none");
        // No ordering against the other offline-update units.
        for other in [
            "pop-upgrade-init",
            "packagekit-offline-update",
            "fwupd-offline-update",
        ] {
            assert!(!unit.contains(other), "{other}");
        }
        assert!(!unit.contains("Conflicts="));
        assert!(unit.ends_with('\n') && !unit.ends_with("\n\n"));
        // It runs the helper copy, not the packaged helper.
        assert!(lines.contains(&&*format!("ExecStart={HELPER_COPY} --apply-restore")));
        assert!(!unit.contains("/usr/libexec"));
    }

    #[test]
    fn the_drop_in_is_byte_for_byte_the_plans() {
        let expected = "\
# Written by apsis-helper while a restore is armed, removed when it ends.
# Pop!_OS's release upgrade must not run in the boot that applies an Apsis restore.
# The path below exists exactly while /system-update is Apsis's link.
[Unit]
ConditionPathExists=!/system-update/apsis-helper
";
        assert_eq!(drop_in_text(), expected);
        assert_eq!(drop_in_text().as_bytes(), expected.as_bytes());
    }

    #[test]
    fn the_drop_ins_condition_is_the_helper_copy_through_the_link() {
        let condition = drop_in_text()
            .lines()
            .find_map(|l| l.strip_prefix("ConditionPathExists="))
            .expect("a condition");
        assert_eq!(condition, "!/system-update/apsis-helper");
        // The same place the unit runs the helper from, reached through the link.
        let through_link = HELPER_COPY.replace("/var/lib/apsis/restore", "/system-update");
        assert_eq!(condition.strip_prefix('!'), Some(through_link.as_str()));
        // One section, one directive: nothing else changes about Pop's unit.
        let directives: Vec<&str> = drop_in_text()
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .collect();
        assert_eq!(
            directives,
            ["[Unit]", "ConditionPathExists=!/system-update/apsis-helper"]
        );
    }

    #[test]
    fn the_paths_are_under_etc_and_on_the_protect_list() {
        for path in [UNIT_PATH, UNIT_WANTS_LINK, DROP_IN_PATH] {
            assert!(path.starts_with("/etc/systemd/system/"), "{path}");
            assert!(PROTECTED.contains(&path), "{path} isn't protected");
        }
        assert!(DROP_IN_PATH.ends_with("/pop-upgrade-init.service.d/50-apsis.conf"));
        assert!(HELPER_COPY.starts_with("/var/lib/apsis/"));
        // The helper copy is covered by the folder's rule.
        assert!(PROTECTED.contains(&"/var/lib/apsis/***"));
    }
}
