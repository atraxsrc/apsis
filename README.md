<p align="center">
  <img src="resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg" width="128" alt="Apsis logo">
</p>

<h1 align="center">Apsis</h1>

<p align="center">System snapshot and restore for the COSMIC™ desktop.</p>
<img width="357" height="386" alt="4" src="https://github.com/user-attachments/assets/c30d9de7-d667-4ac7-92bb-88ae40fa78a7" />
<img width="647" height="445" alt="1" src="https://github.com/user-attachments/assets/94fd0b46-59fb-4c0c-92bf-5369118920c3" />
<img width="649" height="448" alt="2" src="https://github.com/user-attachments/assets/000b38e9-dd00-4d11-8157-b867264dca2b" />
<img width="649" height="448" alt="3" src="https://github.com/user-attachments/assets/5d322264-0302-47c8-9817-454ef0449ac2" />

Apsis takes snapshots of your system, the way Timeshift does, and lives in the COSMIC panel.
The panel shows when the last snapshot was taken and how full the backup disk is; the window
creates, lists and deletes snapshots and holds the settings.

> **Status:** 0.4.0. Apsis takes rsync snapshots itself and doesn't need Timeshift. It uses
> Timeshift's layout on the backup disk, so snapshots Timeshift made keep working in Apsis
> (and the other way round). Manual only: there is no schedule, and a snapshot is only deleted
> when you delete it. Restoring the whole system comes in 0.5.0.

## What it does

- **Snapshots with rsync.** The first one copies the system to your backup disk; later ones
  copy only what changed and hard-link the rest, so each looks complete but costs only the
  changes. rsync runs at idle priority, so the desktop stays responsive.
- **Two choices of what to include** besides the system: `/root` and `/home`.
- **A filter list**, like Timeshift's Filters tab: folders, files or patterns to leave out
  (`-`) or keep (`+`).
- **Delete snapshots**, one or several at once.
- **Stop** a snapshot while it's being made; what it copied so far is removed.
- **Status in the panel**: the tooltip shows the last snapshot and the backup disk; an optional
  label beside the icon shows both at a glance. The icon turns the warning colour when the last
  snapshot is older than a week (you choose how long), or the disk is nearly full.
- **Follows the COSMIC theme** live: light and dark, accent colour, fonts.

Not in Apsis: scheduled snapshots, automatic deletion, restoring single files, btrfs
snapshots, and encrypted backup disks.

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
confirm. A dimmed "Interrupted snapshot" row is what's left of a snapshot that was cut off
(power loss, a crash); the next snapshot removes it, or delete it yourself.

**Why does it ask for my password?** Snapshots touch system files, so creating, deleting and
saving the settings run as root, and your system asks you (polkit) to confirm. It's remembered
for a few minutes. Listing needs no password, and neither does stopping a snapshot you started.

### Keyboard shortcuts

| key | does |
|---|---|
| `Ctrl+N` | Create |
| `Delete` | Delete the selected snapshots |
| `Ctrl+R`, `F5` | Refresh |
| `Ctrl+,` | Settings |
| `Ctrl+A` | Select all |
| `↑` `↓` | Move the selection |
| `Esc` | Close a dialog, leave the settings, clear the selection |

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
APSIS_LAYOUT_TEST=1 cargo test -p apsis fit                     # layouts fit the smallest window
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
