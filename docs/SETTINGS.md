# Settings

The settings in detail, deleting snapshots, passwords and keyboard shortcuts.

**Location**: the disk snapshots go to, one row per disk. Only disks that can hold snapshots
have a radio (a Linux filesystem, not encrypted); the others are dimmed with the reason under
their name. The chosen disk says "Not connected" while it's unplugged.

**Include**: the system is always included. `/root` is the root user's home folder (on by
default). `/home` is every user's home folder: documents, photos and settings (off by
default). With `/home` included, a full restore puts your documents back as they were
in the snapshot too.

**Filters**: each row is a pattern with **Keep** (`+` in the config file) or **Leave out**
(`-`). rsync takes the **first** row that matches, top to bottom, so order matters: new rows
go on top, and the up and down buttons on the selected row reorder them. Add folder and Add
file pick a path; Add pattern takes a typed one (start it with `+ ` to keep; anything else is
left out). Built-in excludes (`/proc`, `/dev`, `/tmp`, caches, other mounts) always come
first; "Built-in excludes" under the list unfolds them.

- `*` matches any name within one folder level; it doesn't cross a `/`.
- `**` matches anything, across folders.
- `***` after a folder means the folder and everything in it.
- A pattern starting with `/` starts at the root of the system. Without the leading `/`, a
  pattern with a `/` in it matches the end of any path: `- home/Downloads` leaves out
  `/home/Downloads` and `/srv/home/Downloads`, not `/home/you/Downloads`.

| filter | effect |
|---|---|
| `- /var/lib/libvirt/***` | leave out virtual machine disk images |
| `- /home/*/Downloads/***` | leave out every user's Downloads folder (with `/home` included) |
| `+ /home/you/Projects/***` | keep one folder of your home even with `/home` left out |

**General** (saved at once, for you only): the **Reminder** (remind me when the last snapshot
is older than this many days; 0 turns it off), and the **Panel** label beside the icon.

## Deleting snapshots

To delete: select one or more snapshots (Ctrl-click or Shift-click for several), **Delete**,
confirm. Several are deleted in one go, in list order, and your password is asked once; if one
fails, the rest are left alone and the status line says which were deleted. A dimmed "Unfinished
snapshot or delete" row is what's left of a snapshot or a delete that was cut off (power loss, a
crash): select it and **Delete**. The next snapshot removes most of them by itself, but never
one still in `timeshift/snapshots/` (a delete cut off by an older Apsis leaves it there).
**Known limit:** a folder in `timeshift/snapshots/` with no `info.json` is only a warning, and
Apsis won't delete it; it needs removing by hand, as root.

## Passwords

**Why does it ask for my password?** Snapshots touch system files, so creating, deleting and
saving the settings run as root, and your system asks you (polkit) to confirm. It's remembered
for a few minutes. Listing needs no password, and neither does stopping a snapshot you started.
A restore asks every time.

## Keyboard shortcuts

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
