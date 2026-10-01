// SPDX-License-Identifier: GPL-3.0-only

//! `timeshift/apsis-restore-RECOVER.txt` on the backup disk (PLAN 6b.11): the README's "If a
//! restore goes wrong" steps with this machine's UUIDs and the snapshot's name filled in,
//! written while preparing. It holds only UUIDs and the snapshot name.

/// The note's name, in the backup disk's `timeshift/` folder next to `snapshots/`.
pub const FILE: &str = "apsis-restore-RECOVER.txt";

/// The note's text for a restore of `snapshot` on the machine whose root, ESP and backup
/// disk have these filesystem UUIDs. `old_format`: the snapshot was made without ACLs and
/// extended attributes, so its rsync line has no `-A -X`.
#[must_use]
pub fn text(
    root_uuid: &str,
    esp_uuid: &str,
    backup_uuid: &str,
    snapshot: &str,
    old_format: bool,
) -> String {
    let acls = if old_format { "" } else { " -A -X" };
    format!(
        "\
Apsis restore: if the computer doesn't start afterwards

Written by Apsis while preparing to restore the snapshot {snapshot}.
It holds only this machine's disk UUIDs and the snapshot's name.

The recovery partition and the boot menu's Pop_OS-oldkern entry are never touched by a
restore.

1. At power-on, hold Space for the systemd-boot menu and pick Pop!_OS Recovery, or boot a
   Pop!_OS live USB of the same version.

2. In a terminal:

   sudo mount /dev/disk/by-uuid/{root_uuid} /mnt
   sudo mount /dev/disk/by-uuid/{esp_uuid} /mnt/boot/efi
   sudo mkdir -p /media/backup && sudo mount -o ro /dev/disk/by-uuid/{backup_uuid} /media/backup
   # finish the same restore (or pick the safety snapshot to go back):
   sudo rsync -a{acls} --numeric-ids --delete --force --sparse \\
     --exclude-from=/mnt/var/lib/apsis/restore/restore.filter \\
     /media/backup/timeshift/snapshots/{snapshot}/localhost/ /mnt/
   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done
   sudo chroot /mnt update-initramfs -u -k all
   sudo chroot /mnt kernelstub --verbose
   sudo rm -f /mnt/system-update

3. Restart.

The filter's rules are anchored at the transfer root, so they work unchanged against /mnt/.
--numeric-ids matters here: the live system's user database isn't the installed one.
See also \"If a restore goes wrong\" in Apsis's README.
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "11111111-1111-1111-1111-111111111111";
    const ESP: &str = "AAAA-0001";
    const BACKUP: &str = "00000000-0000-0000-0000-000000000000";
    const NAME: &str = "2026-09-25_11-28-53";

    #[test]
    fn the_recovery_steps_name_the_disks_and_the_snapshot() {
        let note = text(ROOT, ESP, BACKUP, NAME, false);
        for needle in [
            &format!("/dev/disk/by-uuid/{ROOT} /mnt"),
            &format!("/dev/disk/by-uuid/{ESP} /mnt/boot/efi"),
            &format!("-o ro /dev/disk/by-uuid/{BACKUP} /media/backup"),
            &format!("/media/backup/timeshift/snapshots/{NAME}/localhost/ /mnt/"),
            "rsync -a -A -X --numeric-ids --delete --force --sparse",
            "--exclude-from=/mnt/var/lib/apsis/restore/restore.filter",
            "chroot /mnt update-initramfs -u -k all",
            "chroot /mnt kernelstub --verbose",
            "rm -f /mnt/system-update",
            "Pop!_OS Recovery",
        ] {
            assert!(note.contains(needle), "missing {needle:?}\n{note}");
        }
        assert!(!note.contains('@'), "no names, no emails");
        assert!(note.ends_with('\n'));
        // An old-format snapshot is restored without -A -X.
        let old = text(ROOT, ESP, BACKUP, NAME, true);
        assert!(old.contains("rsync -a --numeric-ids --delete --force --sparse"));
        assert!(!old.contains("-A -X"));
        assert_eq!(FILE, "apsis-restore-RECOVER.txt");
    }
}
