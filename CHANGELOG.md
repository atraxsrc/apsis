# Changelog

All notable changes to Apsis are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.4.1] - 2026-10-01

Snapshots now hold everything a full restore (0.5.0) needs.

### Changed

- Snapshots keep POSIX ACLs and extended attributes, including file capabilities (for example
  on `ping`). rsync gets `-A -X --numeric-ids`. Hard links between files (`-H`) are still not
  kept, as in Timeshift.
- `info.json` gets one more entry at the end, `"apsis-rsync-flags"`. Timeshift ignores it.
  Snapshots without it (Timeshift's, and Apsis 0.4.0 and older) keep working; they are the
  older format, made without ACLs and extended attributes.
- The first snapshot after the upgrade copies again the few files that have ACLs or extended
  attributes, instead of linking them to the previous snapshot. Later snapshots link as before.

## [0.4.0] - 2026-09-30

Apsis does four things: snapshot the system (with `/root` and `/home` as choices), a filter
list, delete snapshots, and (in 0.5.0) restore the whole system. The window is now laid out
like Timeshift's, in standard COSMIC widgets.

### Upgrading

- Open the settings once and click **Save**. Settings from 0.3 (version 1 of
  `/etc/apsis/config.toml`) are shown converted: a home folder set to "everything" for every
  user becomes the `/home` choice, "hidden files only" becomes a `+ <home>/.**` filter, and
  plain excludes get a `-`. The converted settings take the same files (checked with rsync in
  the tests). One exception: a `+` filter that a later filter used to hide now works, because
  the folders above a kept path are let in.
- Going back to 0.3: `sudo cp /etc/apsis/config.toml.bak /etc/apsis/config.toml` (0.3 can't read
  version 2).
- File restore is gone. Anything in `~/Apsis-restored/`, and any `*.apsis-before-*` file next to
  an original, is yours; delete it by hand when you don't need it.
- The helper's D-Bus interface is now `io.github.atraxsrc.Apsis.Helper2`. After upgrading, log
  out and in (or re-add the applet) so the panel runs the new version.

### Added

- A window like Timeshift's: a toolbar (Create, Delete, Settings), the snapshot list (date and
  comment; Ctrl-click and Shift-click select several), and a status area with the last
  snapshot, the backup disk and its bar, and a running snapshot's progress.
- Settings as tabs: **Location** (the backup disk), **Include** (`/root`, on by default; `/home`,
  off by default), **Filters** (`+`/`-` list with Add Folder, Add File, Add Pattern, Remove,
  Move Up, Move Down; new lines go on top), **Misc** (reminder, panel label).
- **Stop** a snapshot while it's being made: rsync's process group is ended and what it copied
  removed. The user who started it isn't asked for a password (new polkit action `stop` for
  anyone else).
- Every window and the panel popup show a snapshot being made or deleted elsewhere, and list
  again when it ends.
- What an interrupted snapshot leaves behind is shown in the list, removed by the next
  snapshot, and can be deleted.
- A backup disk pulled out during a snapshot or delete is reported as such by the helper.
- Keyboard shortcuts: Ctrl+N, Delete, Ctrl+R / F5, Ctrl+,, Ctrl+A, arrows, Esc (man page and
  README).

### Changed

- No terminal look: no prompt line, no key-hint footer, no rooms or dock.
- The panel popup uses standard widgets: last snapshot, backup disk, a running job, the newest
  snapshots, Open Apsis and Refresh.
- A failed create's staging folder is removed with the delete's safe walk.
- rsync gets `--delete-excluded` once (Timeshift passes it twice).
- Summary: "System snapshot and restore".

### Removed

- Restoring single files (the browser, both restore modes, `Apsis-restored`, `.apsis-before`
  backups) and the polkit actions `browse`, `restore` and `restore-original`.
- Keep last N snapshots, and its preview.
- Per-user home folder modes (excluded / hidden files only / everything).

## [0.3.1] - 2026-09-30

Fixes from the first test on a clean machine.

### Fixed

- One window: "open apsis" in the panel and the app launcher bring the open window forward
  instead of opening another (libcosmic's single-instance support; the panel asks the
  compositor for an activation token first). `--settings` and `--about` switch the open window
  to that view when nothing is in progress there.
- A window opened while a snapshot job runs (started by another window, or by one that was
  closed) no longer looks idle: the activity pane says `busy · a job is running in the
  background`, and Apsis lists again every few seconds until the job is over, then goes back to
  idle.
- A create that fails because the backup disk was unplugged says `create failed · backup disk
  removed`; rsync's raw error ("Input/output error (os error 5)") goes to the log room only.
- The helper's systemd unit no longer says it runs Timeshift: `Apsis snapshot helper`.

### Added

- The create room shows whether your home folder goes into the snapshot: `home  included`,
  `home  hidden files only` or `home  excluded` (muted; changed in settings).
- The file browser says `home not included in this snapshot` for an empty home folder, instead
  of `empty folder`.
- After a folder-mode restore, the activity pane says where the files went (`restored to
  ~/Apsis-restored/<snapshot>/...`), and `o` (or `[o]pen folder`) opens that folder.
- The backup disk is watched without `r`: every 5 seconds and when the popup or window opens,
  Apsis looks for it in `/dev/disk/by-uuid` (no root, no mount). When it's gone, the strip and
  the tooltip say `disk  not connected` and creating waits; when it's back, Apsis lists again.

## [0.3.0] - 2026-09-29

A status you can read at a glance, and a window that does the work. The panel popup is now a
read-only overview; creating, deleting, settings and restore are in the window.

### Added

- A status model behind the panel: the tooltip shows `last`, `next` (manual only, for now) and
  `disk`; the icon takes the warning colour when a snapshot is overdue or the disk is low, and
  the destructive colour when the disk is nearly full.
- An optional panel label beside the icon: the age of the newest snapshot and the share of the
  backup disk in use (horizontal panels only). Off by default; a setting.
- The `apsis` strip in the window: last and next on the left, the backup disk and its bar on the
  right.
- Disk warning colours: the bar turns to the theme's warning and destructive colours as free
  space runs low.
- The activity pane shows progress and the time left while a snapshot is created or restored.
- A four-room window: snapshots, create (beside the list), schedule (keep and remind) and log,
  switched with `1`-`4` or by clicking the dock.

### Changed

- The panel popup is a read-only overview: the strip, the newest snapshots, `open apsis`,
  refresh and close. Nothing is created, deleted or restored from it.
- Theme audit: every colour comes from the COSMIC theme (accent, divider, background, warning,
  destructive); the only derived colours are faded versions of those.
- A leftover from an interrupted snapshot is reported as one plain line ("leftover from an
  interrupted snapshot, safe to delete") instead of the raw staging path and timestamp; the path
  goes to the journal.
- New screenshots in the README and the metainfo: the window, the create and schedule rooms,
  and the panel popup.

### Known issues

- With a theme whose background is translucent, the accent border of the active pane tints the
  whole pane. Changing the theme avoids it; a proper fix waits for libcosmic.

## [0.2.0] - 2026-09-29

Apsis is standalone now: it takes rsync snapshots itself and no longer needs or runs
Timeshift. Snapshots keep Timeshift's on-disk layout, so the ones Timeshift made keep working
(listed, browsed, restored from, deleted), and an installed Timeshift still reads Apsis's.
Manual only: there is no schedule, and a snapshot is only deleted when you delete it.

**Upgrading:** open the settings (`s`) once. They show what was taken from Timeshift's
settings (backup disk, home folders, filters); press `w` to save them as Apsis's own
(`/etc/apsis/config.toml`). Your snapshots stay where they are.

### Added

- Apsis's own settings file, `/etc/apsis/config.toml` (backup disk and filters), written only
  by `apsis-helper`, atomically, with a `.bak` of the previous one. The first time, it's
  imported from Timeshift's `/etc/timeshift/timeshift.json`, which is only read.
- Keep the last N snapshots (optional): `p` previews which older snapshots would go, and
  deletes them only after `y`; offered after each create. Commented snapshots always stay and
  don't count.
- Reminder: the panel icon turns the warning colour, and the tooltip says so, when the last
  snapshot is older than 7 days (configurable, or off).
- A disk usage line under the panes: the backup disk's used and free space as a bar.
- Progress with the time left for creates and restores.
- rsync runs at idle I/O priority and nice 19, so the desktop stays responsive.
- `just deb-install` builds the .deb and installs it, for testing.

### Changed

- Creating, listing and deleting snapshots is done by Apsis (rsync), no longer by Timeshift.
  Deleting as root refuses anything that isn't plainly one snapshot folder: a snapshot name,
  no symlink on the way, an `info.json`, nothing mounted inside; it never follows a symlink or
  leaves the filesystem, and removes only that snapshot's tag links.
- The settings view has the backup disk, home folders and filters, plus Apsis's own keep and
  remind.
- `apsis-helper` is required (the .deb installs it); there's no `pkexec` fallback any more.
- The .deb no longer recommends `timeshift`; `apt purge` also removes `/etc/apsis`.

### Removed

- Timeshift as the backend, and everything that ran `timeshift`.
- Timeshift's settings in Apsis: schedules and retention counts, rsync/btrfs mode, `@home`.
  Apsis doesn't schedule or delete snapshots by itself.
- The native backend's dry-run setting (the native backend is the only one now).

### Not supported (yet)

- Encrypted (LUKS) backup disks: Timeshift unlocks them itself; Apsis doesn't.
- btrfs snapshots and full-system restore.

## [0.1.2] - 2026-09-27

### Added

- Bulk delete in the snapshot list, like Timeshift: `space` marks or unmarks a snapshot, `J`
  marks and moves down. Marked rows get an accent `*` and a faint accent background, and the
  title shows `· N marked`. `d` asks once, listing them; they're deleted one by one through
  `apsis-helper` with a single password prompt, with progress (`deleting 2/4: …`) in the
  activity pane. It stops at the first failure and says which were deleted and which weren't.
  Esc clears the marks.
- Right-click menu: **Close** (closes the menu, like Esc).
- README: "Settings explained": device, rsync or btrfs mode, schedules and "keep N", home
  folders (excluded / hidden files only / everything), filters with examples, and the Apsis
  backend and dry run settings.
- Manual page `apsis(1)` (`man apsis`), installed by `sudo just install` and the .deb.
- Settings view: home rows explain the three options.

### Changed

- Delete prompts, progress and results name a snapshot by its date and comment
  (`09-27 09:12 "bulk 1"`) instead of the raw name; the details pane and the helper's journal
  keep the full name.
- Right-click menu: "Panel settings…" is now "Remove or move applet…" (it still opens COSMIC
  Settings on the panel page).

### Fixed

- Settings view: the explanations in the details pane are no longer cut off. In the popup the
  pane is sized to the longest one and keeps its height while moving between rows; the notes
  fit the smallest window.
- Timeshift's `Ret=NNN` lines are recognised as warnings like `E:`/`W:`, instead of being
  ignored or reported as `not a snapshot row`.

## [0.1.1] - 2026-09-26

### Added

- `.deb` package for Pop!_OS / Ubuntu 24.04 or newer (amd64): `just deb` builds it with
  cargo-deb, with the same files as `sudo just install`. Pushing a `v*` tag builds it in CI and
  attaches it to the GitHub release.
- README: a "How to use" section (install, first run, everyday use, restoring files, settings,
  troubleshooting).

### Fixed

- `just vendor` now writes `vendor.tar` (it used to delete its output), with versioned vendor
  folders so `atspi-common` builds, and `just build-vendored` builds offline (`--frozen`) and
  cleans up the unpacked sources afterwards.

## [0.1.0] - 2026-09-26

First release.

### Added

- COSMIC panel applet with a terminal-style popup that follows the live COSMIC theme: snapshot
  list, details pane, activity pane, `>` input line and key hints. Keyboard (vim-style keys) and
  mouse.
- List Timeshift snapshots (`timeshift --list`), with tags, comments, and the age of the last
  snapshot in the panel button's tooltip.
- Create snapshots with a comment, and delete them after a `[y/N]` confirmation.
- Settings view for Timeshift: backup device, rsync or btrfs mode, `@home`, schedules and their
  counts, home folders per user, and filters, written to `/etc/timeshift/timeshift.json` with a
  backup of the old file.
- Window mode: `apsis --window`, and an app launcher entry that opens it; resizable, opens
  floating.
- Right-click menu on the panel button (refresh, settings, about, panel settings).
- `apsis-helper`: a root D-Bus service, started on demand and guarded by polkit, so creating or
  deleting asks for a password once per few minutes instead of every time. Falls back to
  `pkexec` when it isn't installed. See `SECURITY.md`.
- **Experimental:** native rsync backend. Creates Timeshift-compatible rsync snapshots without
  running `timeshift`, with a dry run (on by default) that shows the whole plan. Opt-in from
  the settings view.
- **Experimental:** file-level restore. Browse a snapshot's files with a comparison to the
  running system, mark entries (space marks in place, `J` marks and moves down), and restore
  them to `~/Apsis-restored/<snapshot>/` or back to their original place, after a dry run.
  Original mode keeps each replaced file as `<name>.apsis-before-<snapshot>` and asks for the
  password every time.
- App and symbolic icons, AppStream metainfo, desktop entries, `just install` / `just uninstall`.

[0.4.0]: https://github.com/atraxsrc/apsis/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/atraxsrc/apsis/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/atraxsrc/apsis/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/atraxsrc/apsis/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/atraxsrc/apsis/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/atraxsrc/apsis/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/atraxsrc/apsis/releases/tag/v0.1.0
