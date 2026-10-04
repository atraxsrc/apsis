<p align="center">
  <img src="resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg" width="128" alt="Apsis logo">
</p>

<h1 align="center">Apsis</h1>

<p align="center">System snapshot and restore for the COSMIC™ desktop.</p>
<p align="center">
  <img width="357" height="386" alt="The panel popup" src="docs/4.png" />
</p>
<p align="center">
  <img width="647" height="445" alt="The window" src="docs/1.png" />
</p>
<p align="center">
  <img width="649" height="448" alt="Settings: Location" src="docs/2.png" />
</p>
<p align="center">
  <img width="649" height="448" alt="Settings: Include" src="docs/3.png" />
</p>

Apsis takes snapshots of your system, the way Timeshift does, and lives in the COSMIC panel.
The panel shows when the last snapshot was taken and how full the backup disk is; the window
creates, lists and deletes snapshots and holds the settings.

> **Status:** 0.5.0 (in development). Apsis takes rsync snapshots itself and doesn't need
> Timeshift. It uses Timeshift's layout on the backup disk, so snapshots Timeshift made keep
> working in Apsis (and the other way round). Manual only: there is no schedule, and a snapshot
> is only deleted when you delete it. **Restoring the whole system is new in 0.5.0 and
> experimental**: it works on Pop!_OS with systemd-boot, and the restore runs at the next
> start, outside the desktop. Read "If a restore goes wrong" below before you rely on it.

## What it does

- **Snapshots with rsync.** The first one copies the system to your backup disk; later ones
  copy only what changed and hard-link the rest, so each looks complete but costs only the
  changes. rsync runs at idle priority, so the desktop stays responsive.
- **Two choices of what to include** besides the system: `/root` and `/home`.
- **A filter list**, like Timeshift's Filters tab: folders, files or patterns to leave out
  (`-`) or keep (`+`).
- **Delete snapshots**, one or several at once.
- **Restore the whole system** to a snapshot (0.5.0, experimental). Apsis checks the snapshot
  fits this computer, takes a safety snapshot of the system as it is now, and puts the system
  files back at the next start, outside the desktop, then starts normally. Your home folders
  are kept unless you choose to restore them too.
- **Stop** a snapshot while it's being made; what it copied so far is removed.
- **Status in the panel**: the tooltip shows the last snapshot and the backup disk; an optional
  label beside the icon shows both at a glance. The icon turns the warning colour when the last
  snapshot is older than a week (you choose how long), or the disk is nearly full.
- **Follows the COSMIC theme** live: light and dark, accent colour, fonts.

Not in Apsis: scheduled snapshots, automatic deletion, restoring single files, btrfs
snapshots, and encrypted backup disks.

**How restore differs from Timeshift's.** Timeshift restores from the running system or a live
USB and rebuilds the boot files for the bootloader it finds. Apsis restores at the next start
(systemd's offline-update mode, like a Pop!_OS release upgrade), so no file is in use while it's
replaced; it keeps the live `fstab` and `crypttab` and the boot partition's layout, refreshes
the boot files with kernelstub exactly as Pop!_OS's own kernel hooks do, uses the snapshot's
initrds as they are, keeps the kernel the computer started with until the new boot files have
been checked, and keeps the old boot files if the refresh or the check fails. It never restores
onto another installation, and it refuses rather than guesses.

## Requirements

- COSMIC desktop (Pop!_OS 24.04 or any distribution shipping COSMIC)
- `rsync`, and a polkit agent (COSMIC has one)
- A backup disk with a plain, **unencrypted** Linux filesystem (ext4, btrfs, xfs...).
  **Encrypted (LUKS) backup disks aren't supported yet.**
- Timeshift isn't needed. If it was set up before, Apsis takes its backup disk and filters
  once (you confirm with Save), keeps using its snapshots, and never runs it.

To build:

- Rust (stable) and [`just`](https://github.com/casey/just)
- libcosmic's build dependencies. On Pop!_OS / Ubuntu / Debian:

  ```sh
  sudo apt install cargo just pkgconf libexpat1-dev libfontconfig-dev libfreetype-dev \
    libxkbcommon-dev libwayland-dev
  ```

## Install

**From the .deb (Pop!_OS / Ubuntu 24.04 or newer, amd64)**

1. Download `apsis_<version>_amd64.deb` from the
   [releases page](https://github.com/atraxsrc/apsis/releases).
2. Install it from the folder you downloaded it to:

   ```sh
   sudo apt install ./apsis_*.deb
   ```

**From source**

```sh
git clone https://github.com/atraxsrc/apsis
cd apsis
just                 # build (release); needs the build requirements above
sudo just install    # the applet, its launcher entry, and apsis-helper
```

Both install `apsis-helper`: a small root service that does the work that needs root
(mounting the backup disk, rsync, deleting a snapshot, writing the settings), with polkit
deciding who may do what. What runs as root: [SECURITY.md](SECURITY.md). Files and design:
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

**Upgrading from 0.3?** Open the settings once. Your old settings are shown as they will be
saved (a note at the top says what changed: a home folder set to "everything" becomes the
`/home` choice, "hidden files only" becomes a `+` filter); snapshots keep holding the same
files. Click Save. File restore is gone in 0.4: anything already in `~/Apsis-restored/`, and
any `*.apsis-before-*` file next to an original, is yours and can be deleted by hand.

## Use

1. **Add the applet to the panel:** *COSMIC Settings → Desktop → Panel → Configure panel
   applets*, then add **Apsis**. Click its icon for the overview, then **Open Apsis**. Or open
   **Apsis** from the app launcher.
2. **Choose a backup disk:** **Settings → Location**, pick the disk, **Save** (your password is
   asked). An external disk is best; a snapshot on the same disk won't help if that disk dies.
3. **Take a snapshot:** **Create**, type a comment if you like, **Create**. The first one copies
   the whole system and can take a while. The bottom of the window shows how far it is and the
   time left, and **Stop** if you change your mind.

To delete: select one or more snapshots (Ctrl-click or Shift-click for several), **Delete**,
confirm. Several are deleted in one go, in list order, and your password is asked once; if one
fails, the rest are left alone and the status line says which were deleted. A dimmed "Interrupted snapshot" row is what's left of a snapshot that was cut off
(power loss, a crash); the next snapshot removes it, or delete it yourself.

4. **Restore the system** (0.5.0, experimental): select one snapshot, **Restore**. Apsis checks
   that the snapshot fits this computer (same installation, UEFI, Pop!_OS with systemd-boot,
   a plain ext4 system disk) and shows the choices: keep your home folders as they are now
   (default) or restore them too, and take a safety snapshot first (on by default). The safety
   snapshot is the way back: the system as it was right before the restore, listed as "Safety
   snapshot, before restoring <date>", costing only the files that differ from the newest
   snapshot. To undo the restore, restore it like any other snapshot; without it there is no
   way back. **Restore**
   then prepares: it measures the space, takes the safety snapshot and writes the plan. **Ready to restore** is the last word: **Restart now** restarts the
   computer, restores the system with the desktop stopped, and restarts once more; **Cancel
   restore** (or Esc, or closing the window) drops the plan and keeps the safety snapshot.
   Once it restarts, the restore can't be stopped: don't turn off the computer until it's
   back at the login screen. After you log in, the window's status line says how it went.

**Why does it ask for my password?** Snapshots touch system files, so creating, deleting and
saving the settings run as root, and your system asks you (polkit) to confirm. It's remembered
for a few minutes. Listing needs no password, and neither does stopping a snapshot you started.
A restore asks every time.

### Known limitations of restore

- **Experimental**, and for **Pop!_OS 24.04 with systemd-boot (kernelstub) only**. Other
  distributions, GRUB, BIOS boot, btrfs or an encrypted or LVM system disk, and a system split
  over `/boot`, `/usr` or `/var` partitions are refused before anything happens.
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

### If a restore goes wrong

The recovery partition and the boot menu's **Pop_OS-oldkern** entry are never touched by a
restore. While preparing, Apsis also writes `timeshift/apsis-restore-RECOVER.txt` on the
backup disk with these steps and this machine's disk UUIDs filled in.

1. At power-on, hold Space for the systemd-boot menu and pick **Pop!_OS Recovery**, or boot
   a Pop!_OS live USB of the same version.
2. In a terminal (the UUIDs are in `RECOVER.txt`, or `lsblk -f`):

   ```sh
   sudo mount /dev/disk/by-uuid/<root-uuid> /mnt
   sudo mount /dev/disk/by-uuid/<esp-uuid> /mnt/boot/efi
   sudo mkdir -p /media/backup && sudo mount -o ro /dev/disk/by-uuid/<backup-uuid> /media/backup
   # finish the same restore (or pick the safety snapshot to go back):
   sudo rsync -a -A -X --numeric-ids --delete --force --sparse      --exclude-from=/mnt/var/lib/apsis/restore/restore.filter      /media/backup/timeshift/snapshots/<name>/localhost/ /mnt/
   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done
   sudo chroot /mnt update-initramfs -u -k all
   sudo chroot /mnt kernelstub --verbose
   sudo rm -f /mnt/system-update
   ```

   (`-A -X` only for a snapshot made by Apsis 0.4.1 or later; `RECOVER.txt` has the right
   line.) The last line matters: while `/system-update` exists, the next start would try the
   restore again.
3. Restart.

If the status line after a restore says **incomplete**, the copy broke (most often the backup
disk was disconnected): reconnect it and click **Restore again**, or restore the safety
snapshot. **The restore didn't start** means nothing was changed. See the helper's log for
the details: `journalctl -b -1 -u apsis-restore` (the restore's own boot) and
`journalctl -u apsis-helper`.

### Keyboard shortcuts

| key | does |
|---|---|
| `Ctrl+N` | Create |
| `Delete` | Delete the selected snapshots |
| `Ctrl+R`, `F5` | Refresh |
| `Ctrl+,` | Settings |
| `Ctrl+A` | Select all |
| `↑` `↓` | Move the selection |
| `Esc` | Close a dialog (on the "Ready to restore" prompt: cancel the restore), leave the settings, clear the selection |

Restore has no shortcut: click it, or Tab to it and press Enter.

## Settings

**Location**: the disk snapshots go to. Only disks that can hold snapshots can be picked (a
Linux filesystem, not encrypted); the others are listed with the reason.

**Include**: the system is always included. `/root` is the root user's home folder (on by
default). `/home` is every user's home folder: documents, photos and settings (off by
default). With `/home` included, a full restore (0.5.0) puts your documents back as they were
in the snapshot too.

**Filters**: each line is `+` (keep) or `-` (leave out) and a pattern. rsync takes the
**first** line that matches, top to bottom, so order matters: new lines go on top, and Move Up
/ Move Down reorder them. Add Folder and Add File pick a path; Add Pattern takes a typed one
(start it with `+ ` to include; anything else is excluded). Built-in excludes (`/proc`, `/dev`,
`/tmp`, caches, other mounts) always come first.

- `*` matches any name within one folder level; it doesn't cross a `/`.
- `**` matches anything, across folders.
- `***` after a folder means the folder and everything in it.
- A pattern starting with `/` starts at the root of the system.

| filter | effect |
|---|---|
| `- /var/lib/libvirt/***` | leave out virtual machine disk images |
| `- /home/*/Downloads/***` | leave out every user's Downloads folder (with `/home` included) |
| `+ /home/you/Projects/***` | keep one folder of your home even with `/home` left out |

**Misc** (saved at once, for you only): **Remind me** after this many days without a snapshot
(0 turns it off), and the **Panel label**.

## Troubleshooting

| you see | what to do |
|---|---|
| Backup disk not connected | plug it in; Apsis notices and lists again |
| can't hold snapshots: it's encrypted or a container | encrypted backup disks aren't supported yet; pick a plain one |
| No backup disk chosen | Settings → Location |
| Apsis needs apsis-helper | reinstall the .deb, or `sudo just install` |
| the applet doesn't show after installing | log out and back in, or re-add it in *Configure panel applets* |
| Can't restore this snapshot | the dialog says why and what to do; see "Known limitations of restore" |
| The preparation is too old / is gone | the "Ready to restore" prompt waited more than 30 minutes, or the helper was restarted; start the restore again |
| Restore incomplete · system partly restored | the copy broke; reconnect the backup disk and **Restore again**, or restore the safety snapshot; see "If a restore goes wrong" |
| System restored · still boots the previous kernel | the boot files couldn't be refreshed, or failed the check, and the old ones were kept; the next kernel update should set it right |
| the computer keeps restarting into the restore | boot the recovery and `rm /mnt/system-update` as in "If a restore goes wrong" |
| anything else | the helper's log: `journalctl -u apsis-helper -e` |

## Uninstall

```sh
sudo apt remove apsis      # installed from the .deb (apt purge also removes /etc/apsis)
sudo just uninstall        # installed from source, run in the source folder
```

Your snapshots stay on the backup disk.

## Development

```sh
just run             # run in a window, no root needed
cargo test --workspace
just check           # clippy
just deb             # build the .deb into target/debian/ (needs cargo-deb)
just ext4-image      # an ext4 image for the rsync tests (then mount it, just test-ext4)
APSIS_LAYOUT_TEST=1 cargo test -p apsis fit                     # layouts fit the smallest window (and every restore state)
APSIS_SCREENSHOTS=/tmp/shots cargo test -p apsis screenshots    # renders the views to .rgba files
```

## Name

Apsis is not an acronym.

In orbital mechanics an **apsis** (plural *apsides*, pronounced *AP-sis* / *AP-sih-deez*) is a
turning point on a body's path: **periapsis** at the nearest point, **apoapsis** at the farthest.
A system snapshot is the same idea: a fixed point on the machine's timeline that you can return to.

The logo shows exactly that: an orbit with its two apsides, the glowing one being the point you
come back to.

## Security

Please report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## License

GPL-3.0-only. See [LICENSE](LICENSE).
