# Changelog

All notable changes to Apsis are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

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

[0.2.0]: https://github.com/atraxsrc/apsis/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/atraxsrc/apsis/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/atraxsrc/apsis/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/atraxsrc/apsis/releases/tag/v0.1.0
