# Apsis - UI contract

Replaces the terminal-style contract (2026-09-29, in git history) from 0.4.0 on. Owner's
direction, 2026-09-30. Rules in `CLAUDE.md` and `docs/PRIVACY.md` still win.

## What Apsis is

"Apsis". Where a summary is required (AppStream, Debian): "System snapshot and restore". No
tagline, and never "Timeshift-style".

It does four things: snapshot the system (with `/root` and `/home` as include choices; the
system is always in), a simple include/exclude filter list, delete snapshots, and restore the
whole system (0.5.0). Nothing else. Start simple; add features only when users ask.

## The model: Timeshift's UI and ease of use

- **Main window** (one window, `apsis --window`): a toolbar of labelled buttons with symbolic
  icons, **Create**, **Restore** (from 0.5.0), **Delete**, **Settings**; the snapshot list
  (date, comment; newest first; multi-select); a status area at the bottom (last snapshot, the
  backup disk's used and free with a bar; while a job runs: its label, percent, time left, a
  progress bar and **Stop** for a create).
- **Settings**, like Timeshift's tabs: **Location** (backup device), **Include** (`/root`,
  `/home`), **Filters** (a `+`/`-` list with Add Folder, Add File, Add Pattern, Remove, Move Up,
  Move Down; new rows at the top; first match wins, said in one plain line). Save / Cancel.
- **Confirmations** are dialogs (create with an optional comment, delete, stop), with the
  destructive button style for delete and stop.
- **Panel applet** stays: a read-only popup (last snapshot, disk, the newest few snapshots,
  the running job, Open Apsis) and the tooltip. The optional panel label and the reminder stay.

## Look

- Standard libcosmic widgets. COSMIC theme colours only: no hard-coded colours, no second
  accent. Warning and destructive roles only for the reminder, low disk space and destructive
  buttons. It follows theme changes live.
- No terminal styling: no monospace body, no bordered title-in-border panes, no prompt line,
  no key-hint footer, no dock or rooms.
- Plain sentence-case copy: "Create snapshot", "Last snapshot 12 hours ago", "Backup disk not
  connected". No em or en dashes.

## Input

- Everything works with the mouse.
- Keyboard shortcuts may work (Ctrl+N, Delete, Ctrl+R / F5, Ctrl+,, Ctrl+A, Esc) but are
  documented only in the man page and README, not in the UI.

## Architecture (unchanged)

`apsis-core` (no UI), `apsis` (libcosmic applet and window), `apsis-helper` (root, polkit,
D-Bus, one lock). The UI never runs as root.

## When a phase conflicts with this file

Append the conflict to `docs/DECISIONS.md` with the date and why. If the COSMIC theme can't
express something, stop and ask; don't invent a colour.
