# Changelog

All notable changes to Apsis are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

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

[0.1.0]: https://github.com/atraxsrc/apsis/releases/tag/v0.1.0
