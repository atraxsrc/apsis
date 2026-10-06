# Restore the system

How to put the whole system back to a snapshot, and what restore does not do yet.

Experimental, since 0.5.0. For Pop!_OS 24.04 with systemd-boot, on a plain partition or, since
0.6.0, on Pop!_OS's standard encrypted install. Read
[Known limitations of restore](#known-limitations-of-restore) first, and keep
[If a restore goes wrong](RECOVERY.md) where you can read it without this
computer.

1. Select one snapshot, **Restore**. Apsis checks that the snapshot fits this computer (same
   installation, UEFI, Pop!_OS with systemd-boot, an ext4 system disk on a plain partition or
   on Pop!_OS's standard encrypted install). If it doesn't, the dialog says why and nothing
   happens.
2. Choose: keep your home folders as they are now (default) or restore them too, and take a
   safety snapshot first (on by default).
3. **Restore** then prepares: it measures the space, takes the safety snapshot and writes the
   plan.
4. **Ready to restore** is the last word: **Restart now** restarts the computer, restores the
   system with the desktop stopped, and restarts once more; **Cancel restore** (or Esc, or
   closing the window) drops the plan and keeps the safety snapshot.
5. Once it restarts, the restore can't be stopped: don't turn off the computer until it's
   back at the login screen. On an encrypted system disk the passphrase is asked twice: at
   the restart that runs the restore, and at the start after it.
6. After you log in, the window's status line says how it went.

**The safety snapshot is the way back**: the system as it was right before the restore,
listed as "Safety snapshot, before restoring <date>", costing only the files that differ
from the newest snapshot. To undo the restore, restore it like any other snapshot; without
it there is no way back.

**How restore differs from Timeshift's.** Timeshift restores from the running system or a live
USB and rebuilds the boot files for the bootloader it finds. Apsis restores at the next start
(systemd's offline-update mode, like a Pop!_OS release upgrade), so no file is in use while it's
replaced; it keeps the live `fstab` and `crypttab` and the boot partition's layout, refreshes
the boot files with kernelstub exactly as Pop!_OS's own kernel hooks do, uses the snapshot's
initrds as they are, keeps the kernel the computer started with until the new boot files have
been checked, and keeps the old boot files if the refresh or the check fails. It never restores
onto another installation, and it refuses rather than guesses.

## Known limitations of restore

- **Experimental**, and for **Pop!_OS 24.04 with systemd-boot (kernelstub) only**. Other
  distributions, GRUB, BIOS boot, btrfs, and a system split over `/boot`, `/usr` or `/var`
  partitions are refused before anything happens.
- **An encrypted system disk works in one layout**: the one Pop!_OS's installer makes with
  "Encrypt drive" (an LVM volume inside one LUKS partition), on the installation the
  snapshot was made of. Any other encrypted or LVM layout is refused. The disk's passphrase
  is asked twice: at the restart that runs the restore, and at the start after it.
- **A snapshot made while Apsis 0.5.x was installed puts 0.5.x back**, and the dialog
  doesn't say so: it names an older Apsis only below 0.5. On an encrypted system disk,
  0.5.x then refuses to restore until Apsis 0.6.0 is installed again.
- **A snapshot from another installation is refused**: it must have been taken of this
  system disk.
- **A file changed in place with the same size and modification time** as in the snapshot
  isn't restored. rsync compares size and time; checking every file's content on both sides
  would take far too long for a whole system.
- **Hard links aren't kept** in snapshots (as in Timeshift). Flatpak's data is hard-link
  heavy, so it takes more room in a snapshot and can take more on the system disk after a
  restore. After a restore that keeps your home folders, a user Flatpak app may say its
  runtime isn't installed: `flatpak install` or `flatpak repair` fixes it.
- **The live `/etc/fstab` and `/etc/crypttab` are kept**, not the snapshot's: they describe
  the disks as they are now. A snapshot whose `crypttab` differs from the current one is
  refused.
- **The boot files are refreshed with kernelstub only**, as Pop!_OS's own kernel hooks do,
  told which kernel the snapshot boots; the snapshot's initrds are used as they are, not
  rebuilt. If the refresh fails, or the new boot files fail the check, the computer keeps
  the boot files and the kernel it started with
  ("still boots the previous kernel"); the next kernel update should set that right.
- `/root` is restored with the system if the snapshot has it (with content); otherwise it's
  left as it is.
- Home folders restored "too" go back to the snapshot entirely: files created or changed
  since are deleted or put back to their old version. The safety snapshot includes your home
  folders in that case, so they're on the backup disk.
- **A snapshot whose delete was cut off by Apsis 0.4.x or Timeshift can look whole** when
  the cut came before its `info.json` and `exclude.list` went. Restoring it would remove
  from the system what it lacks; the safety snapshot is the way back. Delete such a
  snapshot instead of restoring it.
