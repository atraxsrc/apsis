<p align="center">
  <img src="resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg" width="128" alt="Apsis logo">
</p>

<h1 align="center">Apsis</h1>

<p align="center">Timeshift-style system snapshots for the COSMIC™ desktop.</p>

A panel applet with a small terminal-style popup to list, create and delete system snapshots.
It works with keyboard or mouse and follows your COSMIC theme.

> **Status:** 0.1.2. Listing, creating and deleting snapshots through Timeshift is the stable
> core. The native rsync backend and restoring files are **experimental**; see below.

<p align="center"><img src="docs/screenshot.png" width="560" alt="Apsis popup: snapshot list, details pane and activity pane"></p>
<p align="center"><img src="docs/screenshot1.png" width="560" alt="Apsis popup: snapshot list, details pane and activity pane"></p>

<p align="center"><sub>The screenshots predate the disk usage line under the panes.</sub></p>

## Features

- **Snapshot list** in the panel popup: date, tags (O/B/H/D/W/M) and comment, with a details
  pane, and the age of the last snapshot in the panel button's tooltip.
- **Disk usage line** under the list: a bar of the backup disk's used and free space, in the
  theme's warning colour under 10% free and its destructive colour under 5% (free space only,
  no bar, when Apsis can't read the disk's size).
- **Create and delete** snapshots through Timeshift, with a comment, and `[y/N]` before a delete.
  Creates and restores show a progress bar with the time left.
- **Keep the last N manual snapshots** (optional): `p` shows which older on-demand snapshots
  would go, and deletes them only after `y`. Commented ones always stay and don't count.
- **Reminder**: the panel icon turns the theme's warning colour, and the tooltip says so, when
  the last snapshot is older than 7 days (configurable).
- **Settings**: backup device, rsync or btrfs mode, schedules and how many to keep, home
  folders per user, and filters, written to Timeshift's own settings file.
- **Keyboard first**: vim-style keys (`j`/`k`, `c`reate, `d`elete, `s`ettings, `?` help) and a
  `>` input line, or just use the mouse.
- **Follows the COSMIC theme** live: colours, corner radius, font. No hard-coded colours.
- **Window mode**: the app launcher's **Apsis** entry opens the same view in a resizable window
  (`apsis --window`).
- **One password prompt** per few minutes, not per action, through a small polkit-guarded
  helper.
- **Experimental: native rsync backend.** Creates Timeshift-compatible rsync snapshots without
  running `timeshift` (Timeshift still lists and deletes them), with a dry run that shows the
  whole plan first. Off by default; switch it on in *settings → apsis → backend*.
- **Experimental: file-level restore.** Browse a snapshot's files, see what differs from the
  running system, mark files or folders, and restore them to `~/Apsis-restored/<snapshot>/`
  (default) or back to their original place, after a dry run. Original mode keeps the replaced
  file as `<name>.apsis-before-<snapshot>` and asks for the password every time.

### Experimental features

The native backend and restore have been tested on the author's machine (Pop!_OS, Timeshift
24.01.1, rsync mode, an ext4 backup disk) and have unit and integration tests, but they haven't
had wide use yet. Before relying on them:

- keep Timeshift itself working as your main way back;
- read the dry run before running anything for real;
- prefer restoring to `~/Apsis-restored/` and copying files over by hand;
- btrfs mode and encrypted backup devices aren't supported by either yet.

Full-system restore isn't in Apsis; use Timeshift for that.

## Requirements

- COSMIC desktop (Pop!_OS 24.04 or any distribution shipping COSMIC)
- [Timeshift](https://github.com/linuxmint/timeshift), set up at least once (backup device
  chosen). Apsis drives its command-line interface.
- `rsync` (for the native backend and restore; Timeshift depends on it anyway)
- A polkit agent (COSMIC has one)

To build:

- Rust (stable) and [`just`](https://github.com/casey/just)
- libcosmic's build dependencies. On Pop!_OS / Ubuntu / Debian:

  ```sh
  sudo apt install cargo just pkgconf libexpat1-dev libfontconfig-dev libfreetype-dev \
    libxkbcommon-dev libwayland-dev
  ```

## How to use

New to COSMIC or Timeshift? **Timeshift** takes *snapshots*: copies of your system files
(programs, settings in `/etc`, and optionally your home folder) on a backup disk, so you can go
back if an update or a change breaks something. **Apsis** is a small window into Timeshift that
lives in the COSMIC panel (the bar at the top or bottom of the screen).

### Install

**From the .deb (Pop!_OS / Ubuntu 24.04 or newer, amd64)**

The .deb is built on Ubuntu 24.04 and needs its system libraries (glibc 2.39 or newer), so it
won't install on older releases; build from source there.

1. Download `apsis_<version>_amd64.deb` from the
   [releases page](https://github.com/atraxsrc/apsis/releases).
2. Install it from the folder you downloaded it to:

   ```sh
   sudo apt install ./apsis_*.deb
   ```

   This also pulls in `rsync`, and `timeshift` unless you skip recommended packages.

**From source**

```sh
git clone https://github.com/atraxsrc/apsis
cd apsis
just                 # build (release); needs the build requirements above
sudo just install    # the applet, its launcher entry, and apsis-helper
```

> Switching from a source install to the .deb? Run `sudo just uninstall` in the source folder
> first, so no stray copy is left behind.

Both install the same files, including `apsis-helper`: a small root service that runs Timeshift
for Apsis, with polkit deciding who may do what. What runs as root: [SECURITY.md](SECURITY.md).
Files and design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

### First run

1. **Pick a backup disk.** Timeshift needs to know where snapshots go. Open Apsis's settings
   (`s`) and choose a `device`, then `w` to save; or set it up once in Timeshift's own window.
   An external disk or a second partition is best; a snapshot on the same disk won't help if
   that disk dies.
2. **Add the applet to the panel:** *COSMIC Settings → Desktop → Panel → Configure panel
   applets*, then add **Apsis**. Its icon (an orbit) appears in the panel; click it.
3. **Or use it as a window:** open **Apsis** from the app launcher. Same view, same keys.

### Everyday use

| to... | do this |
|---|---|
| see your snapshots | click the panel icon. The newest is at the top; the right pane shows details |
| create a snapshot | press `c`, type a comment (e.g. `before driver update`), press Enter |
| delete a snapshot | select it, press `d`, type `y`, press Enter (anything else cancels) |
| delete several | mark each with `space` (or `J` to mark and move down), press `d`, type `y`, press Enter. The password is asked once; they're deleted one by one and it stops at the first failure, saying which were deleted |
| refresh | `r` |

A spinner in the **activity** pane shows what's running; the result stays there afterwards.
The first rsync snapshot copies the whole system and can take a while; later ones only copy
what changed.

**Why does it ask for my password?** Snapshots touch system files, so creating, deleting,
browsing, restoring and changing settings run as root, and your system asks you (polkit) to
confirm with your password. It's remembered for a few minutes, so a second action right after
doesn't ask again. Listing needs no password. The one exception is putting files back in their
original place: that asks every time, on purpose.

### Keys

Every `[key]` hint at the bottom of the popup is also a button, every row can be clicked, and
`?` shows this list inside Apsis.

| where | key | does | mouse |
|---|---|---|---|
| list | `↑` `↓` / `j` `k`, `Home` `End` | move | click a row |
| list | `c` / `d` / `r` | create / delete / refresh | `[c]reate` / `[d]elete` / `[r]efresh` |
| list | `p` | prune old manual snapshots (preview, then `y`) | |
| list | `Enter` | browse the snapshot's files | double-click |
| list | `Tab` | details pane, and back | |
| list | `space` / `J` | mark or unmark for deletion / mark and move down | |
| list | `s` / `?` | settings / help | `[s]ettings` / `[?]help`, or right-click the panel icon |
| browser | `Enter` or `l` / `⌫` or `h` | into a folder / up one | double-click a folder |
| browser | `space` / `J` | mark or unmark / mark and move down | double-click a file |
| browser | `R` / `r` | restore marked / read folder again | `[R]estore` / `[r]eload` |
| settings | `space` | change the selected row | `[space]change` |
| settings | `+` `-` `e` | keep one more / one fewer / type it | `[+]` `[-]` |
| settings | `a` / `x` | add filter / remove selected filter | `[a]dd` / `[x]remove` |
| settings | `w` / `r` | write to Timeshift / reload, dropping changes | `[w]rite` / `[r]eload` |
| anywhere | `Esc` | cancel, go back, clear marks, then close | `[esc]` |

### Restoring a file (experimental)

Say you broke `~/.bashrc` or a file in `/etc` and want yesterday's copy:

1. Select a snapshot from before the change and press **Enter**. Apsis asks for your password
   and shows the snapshot's files, starting at `/`.
2. Move to the file with the arrow keys, **Enter** to go into folders, **h** to go up.
   The mark in front says how it compares to your system now: `+` missing now, `~` changed,
   `=` same.
3. Press **space** to mark it (mark as many as you like, across folders).
4. Press **R**. The `>` line asks *folder or original*; press **f**, then **Enter**.
5. Apsis does a dry run and shows the plan: what it would copy and where. Nothing is written
   yet. Read it, then press **Enter** to run it (or **Esc** to back out).

**Where do the files go?** Into `~/Apsis-restored/<snapshot>/<original path>`, owned by you,
e.g. `~/Apsis-restored/2026-09-25_03-00-01/home/you/.bashrc`. Your live files aren't touched;
compare and copy what you need by hand. A second restore from the same snapshot goes to
`<snapshot>-2/`, and so on.

**Original mode** (`o` instead of `f`) writes the files back over the running system. Use it
only when you're sure, e.g. for a config file you know is broken:

- it asks `put N item(s) back over the running system? [y/N]`, only `y` continues, and the
  password is asked every time;
- each file it replaces is kept next to it as `<name>.apsis-before-<snapshot>`
  (e.g. `/etc/fstab.apsis-before-2026-09-25_03-00-01`), so you can undo it by renaming that
  back;
- the dry run warns about `/etc` and `/usr`; replacing system files while they're in use can
  break things. For a whole broken system, use Timeshift's full restore instead.

### Settings explained

`s` (or right-click the panel icon → *Settings…*) opens Timeshift's own settings, the same file
Timeshift's window edits (`/etc/timeshift/timeshift.json`). Move with `j`/`k`, change the
selected row with `space`. Changes are marked `(unsaved)` until `w` writes them (password
asked once); `r` reads the file again and drops them. The right pane explains the selected row.

**`device`: where snapshots go.** `space` picks the next disk or partition that can hold
snapshots (a Linux filesystem such as ext4 or btrfs, not encrypted). Use an external disk or at
least another disk than the system one: a snapshot on the same disk won't survive that disk
dying. When the disk isn't plugged in, Timeshift keeps it selected and Apsis shows its UUID.

**`mode`: rsync or btrfs.**

- `rsync` (the usual choice) copies the system to the backup disk. The first snapshot copies
  everything; later ones only copy what changed and hard-link the rest, so each looks complete
  but costs only the changes.
- `btrfs` is only offered when the system is on btrfs (Pop!_OS installs ext4 by default). It
  takes instant snapshots of the `@` subvolume on the system disk itself; `@home` adds the
  home subvolume. Fast and small, but they're on the same disk.

**`schedule` and `keep N`.** Five levels: monthly, weekly, daily, hourly and boot. `space`
turns a level on or off; `+`/`-` keep one more or one fewer, `e` types the number (1 to 999).
Timeshift runs the schedule itself (cron), not Apsis. When a level has more than `keep N`
snapshots, the older ones lose that level's tag, and a snapshot with no tags left is deleted.
Snapshots you make with `c` (tag `O`, on-demand) are never removed this way, and neither are
hourly, daily, weekly or monthly snapshots that have a comment.

**`home`: what to save of each user's home folder** (rsync mode). `space` cycles:

| option | what it saves | when to use it |
|---|---|---|
| `excluded` | nothing from that home folder | the default. Snapshots are for the system; your documents belong in a real backup |
| `hidden files only` | the dot-files and dot-folders (`~/.config`, `~/.bashrc`, ...): app and desktop settings | you want settings rolled back with the system, but not documents |
| `everything` | all of it, documents, photos and downloads included | you have no other backup and the backup disk has room |

> **Careful with `everything`:** Timeshift's full restore puts the whole snapshot back, so it
> also rolls your documents back to that day; anything newer in your home folder is lost.
> Restoring single files with Apsis (`Enter` on a snapshot) doesn't have this problem.

An encrypted (ecryptfs) home uses other patterns; change it in Timeshift's window.

**`filters`: extra paths to leave out or keep.** Each filter is an rsync pattern. A plain
pattern excludes; `+ ` in front includes. `a` adds one at the `>` line, `x` removes the
selected one. rsync takes the **first** filter that matches a path, and Timeshift's own
excludes (caches, `/proc`, `/tmp`, other mounts, ...) come first.

- `*` matches any name within one folder level; it doesn't cross a `/`.
- `**` matches anything, across folders.
- A pattern starting with `/` starts at the root of the system.

| filter | effect |
|---|---|
| `/var/lib/libvirt/**` | leave out virtual machine disk images (huge, and they change all the time) |
| `/home/*/Downloads/**` | leave out every user's Downloads folder (`*` is one user name) |
| `+ /home/you/Projects/***` | keep `~/Projects` even though the home folder is `excluded` (`***` is the folder itself plus everything in it; `**` alone wouldn't let rsync into the folder) |

**`apsis` → `backend`** (Apsis's own setting, saved at once, not by `w`). `timeshift`, the
default, runs Timeshift for everything. `native rsync` (**experimental**) has Apsis make rsync
snapshots itself, in Timeshift's exact layout, so Timeshift still lists, restores and deletes
them; deleting always goes through Timeshift. Needs `apsis-helper`; rsync mode only.

**`apsis` → `dry run`** (with the native backend; on by default). `c` then only shows what a
snapshot would do (the rsync command, the exclude list, `info.json`) and writes nothing. Turn it
off to make real native snapshots.

**`apsis` → `keep manual`** (Apsis's own, saved at once; off by default). Keeps the newest N
on-demand snapshots without a comment (the ones made with `c`). `p` shows which older ones would go and why the
others stay, and deletes only after `y`; after each create it offers this by itself when
something is past N. A snapshot with a comment is pinned: it always stays and doesn't count towards N, and
so does one Timeshift's schedule also tagged (hourly, daily...): Timeshift's own retention
decides about those. The newest snapshot is never deleted. `space` turns it on (N = 10) or off,
`+`/`-`/`e` change N.

**`apsis` → `remind`** (Apsis's own, saved at once; 7 days by default). When the newest
snapshot is older than this, the panel icon turns the theme's warning colour and the tooltip
says `none for over 7 days`. With the helper installed, the panel lists in the background at
login and every 6 hours for this (never with a password prompt). `0` turns it off.

> **Close Timeshift's own window first.** If it's open while you save here, it writes its own
> settings over yours when it closes. Apsis warns you in the activity pane when it's open.

### Troubleshooting

| you see | what to do |
|---|---|
| `backup disk not connected (...)` | plug the backup disk in (and unlock it if needed), then press `r` |
| the applet isn't in the panel list, or doesn't show after installing | log out and back in, or remove it in *Configure panel applets* and add it again |
| `list: E: Failed to remove directory` (often followed by `list: Ret=256`) in the activity pane | a harmless Timeshift warning, usually a leftover mount from an earlier run (often after the disk was unplugged). The list itself is still correct; a reboot clears the stale mount |
| `... needs apsis-helper (sudo just install)` | the helper isn't installed: reinstall the .deb or run `sudo just install` |
| anything else | read the helper's log: `journalctl -u apsis-helper -e` (add `-f` to follow it live) |

### Uninstall

```sh
sudo apt remove apsis      # installed from the .deb
sudo just uninstall        # installed from source, run in the source folder
```

Your snapshots stay on the backup disk; Timeshift still lists and restores them.

## Development

```sh
just run             # run in a window, no root needed
cargo test --workspace
just check           # clippy
just deb             # build the .deb into target/debian/ (needs cargo-deb)
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
