# UI — terminal-style popup

A panel applet. Clicking the panel icon opens a popup that looks like a tiny terminal.
It's a normal libcosmic popup, so it always floats and follows the theme (colours, radius, font scale).

## Mock

Phase 3.5 layout (superfile-style panes; superfile was a visual reference only, no code):

```
 ◎ ~/apsis $ ls --snapshots                                      rsync · 3 snapshots
 ╭─ snapshots ──────────────────────────────────╮ ╭─ details ────────────────────╮
 │ ▸ 2026-09-25 03:00   D     boot              │ │ name     2026-09-25_03-00-01 │
 │   2026-09-24 03:00   D                       │ │ created  2026-09-25 03:00:01 │
 │   2026-09-18 12:41   O     "before kernel u… │ │ age      3h ago              │
 │                                              │ │ tags     D daily             │
 │                                              │ │ comment  boot                │
 ╰──────────────────────────────────────────────╯ ╰──────────────────────────────╯
   disk  sdX1  ███████████████░░░░░░░░░░░░░░░░  448G used · 483G free · 3 snapshots
 ╭─ activity ───────────────────────────────────────────────────────────────────╮
 │ snapshot created                                                             │
 ╰──────────────────────────────────────────────────────────────────────────────╯
 > _
 [c]reate  [d]elete  [r]efresh  [?]help                                     [esc]
```

- Panes are rounded, bordered containers with the title set into the top border. The border
  radius is the theme's small corner radius. The active pane's border and title use the accent
  colour, the others the theme's divider colour (title dimmed).
- Left pane (3/5 of the width): the snapshot list, or help / About in its place (title `help` /
  `about`). Right pane (2/5): details of the selected snapshot, always shown. The left pane fits
  the list: at least 5 rows, at most 8, then it scrolls (help and errors use 8, About 6). The
  details pane has the same height; longer content scrolls.
- Each pane is opaque, filled with the theme's background colour.
- Tab makes the details pane active; Tab again, Esc or a click on a row goes back to the list.
  (Enter and a double-click open the snapshot browser since Phase 6a.)
- Activity pane: the running create/delete with a spinner (accent border while it runs), else the
  last result, else `idle` (dimmed). A failure shows Timeshift's last lines. Below that, any
  warnings from the last list (`list: E: Failed to remove directory`) in the theme's warning colour.
- The `>` line stays the input line (comment, `[y/N]`, and the `timeshift --list` spinner). The
  key hints are the footer, the last row.
- Disk line, between the panes and the activity pane: see "Disk usage" below.
- Progress of a create or a (real) restore replaces the spinner line: see "Progress" below.

## Progress

```
 ╭─ activity ───────────────────────────────────────────────────────────────────╮
 │ creating ████████████░░░░░░░░ 58%  ~3 min left                               │
 ╰──────────────────────────────────────────────────────────────────────────────╯
```

- For creates (Timeshift and native) and real restores, through `apsis-helper`: its
  `Progress` signal, at most about twice a second.
- Before the first `Progress`: the usual `creating snapshot… ⠋`. That's all there ever is
  through pkexec, or with an older helper still running (it has no such signal): a spinner,
  no error.
- After the helper's first (number-less) `Progress`, until there's a real number:
  `creating ⠋ estimating…`. Timeshift estimates the system's size first and prints nothing,
  then `0.00% complete (??? remaining)`; neither counts as a number. `0%` never does.
- With a number: label, a 20-cell `█`/`░` bar in the theme's accent colour (the empty part
  dimmed), the whole percent rounded down, and the time left, rough: `<1 min`, `~3 min`,
  `~2 h 5 min`. No time while it's unknown (Timeshift's `???`), and for all but the last path
  of an original-mode restore (one rsync per path, the percent spread over them).
- Where the numbers come from: Timeshift's own `NN.NN% complete (hh:mm:ss remaining)` line
  (see TIMESHIFT-CLI.md), and rsync's `--info=progress2` for the native create and restores.
- Deletes, bulk deletes and dry runs keep the spinner.

## Disk usage

```
  disk  sdX1  ███████████████░░░░░░░░░░░░░░░░  448G used · 483G free · 9 snapshots
```

- One line under the panes, from the last list, in the popup and in `--window` alike. `disk`
  and the text are dimmed; the device is the listed `/dev` name without `/dev/`.
- The bar is monospace `█` (used) and `░` (free, dimmed) cells, as many as fit the space left
  in the line (a `responsive` widget, 8.4 px per cell: 14 px mono text at 0.6 em). Used share
  is `used / (used + free)`, as `df`'s `Use%`; anything used or free gets at least one cell.
- Filled cells use the accent colour, the theme's warning colour when less than 10% is free,
  and its destructive colour under 5%. All from the theme.
- Sizes are binary, like lsblk's (`448G`, `4.5G`), whole numbers from 10 up.
- Where the numbers come from (`apsis-helper`, `ListWithUsage` / `NativeListWithUsage`):
  - Native backend: `statvfs` on `/run/apsis/backup` while the list has the device mounted.
  - Timeshift backend: `statvfs` on a mount of the backup device if one exists after the list
    (looked up by device number or source in `/proc/self/mountinfo`: an automount, or `/` in
    btrfs mode). Timeshift unmounts its own `/run/timeshift/<pid>/backup` when it exits, so a
    dedicated backup disk usually has no mount left: then the helper mounts it
    `ro,nosuid,nodev,noexec` at `/run/apsis/backup` by UUID for the `statvfs` and unmounts it
    (as for browsing; only a device lsblk shows with a Linux filesystem). Only if that fails
    (e.g. an encrypted disk) is Timeshift's own `N snapshots, X GB free` line all there is:
    the line shows `115G free · 9 snapshots` with no bar. The journal says which it was
    (`disk usage: statvfs of ...`).
  - pkexec fallback (no helper): the same free-only line, parsed from `timeshift --list`.
- Unknown usage (no numbers at all, e.g. no device selected or no snapshots yet): no line. No
  bar is ever drawn from a guess.
- Hidden in the settings view (its notes are sized to fit the smallest window without it).
- Refreshed by every list: after create, delete and bulk delete (they list anyway), and after
  a real restore once the browser is left (Esc) or the popup is opened again. Not while the
  browser is open: its `Browse` calls hold the helper's lock, and a list beside them would be
  refused as busy.

## Rules

- Monospace font for everything. Text uses theme text colours; the selected row uses the accent
  colour as a left marker `▸` and a subtle accent background — not a hard-coded colour.
- Every `[key]` hint is also a clickable button. Every row is clickable (select), double-click = details.
- Marked snapshots (space / `J`, for a bulk delete): an accent `*` after the `▸` column and a
  fainter accent background than the selected row (the selection's wins on the selected row).
  The pane title reads `snapshots · 3 marked`, and the details pane adds `delete   marked`.
- The bottom `>` line is the input line: used for the create comment and delete confirm.
  `[c]reate` and `[d]elete` are only enabled once a list has shown a snapshot device, and
  nothing else is running; delete also needs a selected snapshot.
- Width ~ 720 px logical, height fits up to ~8 rows then scrolls. A row's comment is cut to the
  pane width with `…`; the details pane shows all of it.

## Keys

| key | action |
|---|---|
| ↑/↓, j/k | move selection |
| c | create → input line becomes `> comment: _`, Enter runs, Esc cancels |
| space | mark or unmark the selected snapshot for deletion, and stay on it |
| J | mark or unmark, then move down (for marking a run of rows) |
| d | no marks: delete selected → `> delete 09-18 12:41 "before kernel update"? [y/N] _`. With marks: `> delete 3 snapshots: 09-25 11:28 "bulk 1", 09-23 08:33, 09-19 09:29? [y/N] _` (list order; the line wraps if it's long). See "Naming snapshots" below. Enter with `y` deletes, anything else cancels and keeps the marks |
| r | refresh list |
| Enter | browse the selected snapshot's files (Phase 6a) |
| Tab | make the details pane active, or go back to the list |
| s | settings view (see below) |
| ? | help overlay |
| Esc | cancel input, then go back from details/help/about/settings, then clear the marks, then close the popup |

## States

| state | shows |
|---|---|
| loading | `> timeshift --list` then a spinner character cycling `⠋⠙⠹⠸…` |
| empty | `no snapshots yet — press [c] to create one` |
| running | `creating snapshot… ⠹` / `deleting <label>… ⠹` / `deleting 2/4: <label>… ⠹` in the activity pane, keys disabled except Esc (does not cancel root op) |
| result | in the activity pane: `snapshot created` / `deleted <label>` / `deleted 4 snapshots` (dimmed), or `create failed: <last stderr line>` (error colour); stays until the next create/delete |
| bulk delete stopped | `delete stopped at <label>: <reason>`, `deleted (1): <labels>`, `not deleted (3): <labels>` (error colour); the ones not deleted stay marked, the list refreshes if any was deleted |
| error | `error:` + Timeshift's last lines (its `E:`/`W:` lines and stderr), `[r]etry` |
| disk missing | `backup disk not connected (UUID 1a2b…): plug it in and press r` (list error, or the create/delete result) |
| not installed | `timeshift not found — install it or wait for the native backend` |

## Bulk delete

Like Timeshift's window: mark several snapshots, then `d`.

- space marks or unmarks the selected snapshot and stays on it; `J` marks or unmarks and moves
  down. Marks work in the list and the details pane, not on help, About or settings. A refresh
  drops marks of snapshots that are gone; a failed list or closing the popup drops them all.
- `d` with marks: `> delete 3 snapshots: <label>, <label>, <label>? [y/N] _`, in list order
  (newest first). Only `y` deletes. Without marks, `d` is the single delete as before.
- The snapshots go one at a time through the helper (`Delete`, one call each), so polkit asks for
  the password once (`auth_admin_keep`) and the next calls use the cached answer. Without the
  helper each pkexec call asks again. Activity: `deleting 2/4: <label>… ⠹`. No list runs between
  steps; one runs at the end.
- All deleted: `deleted 4 snapshots`, marks cleared. A failure stops it there:
  `delete stopped at <label>: <reason>`, then `deleted (1): ...` and `not deleted (3): ...`
  (the failed one counts as not deleted). The ones not deleted stay marked. A refused password
  on the first one deletes nothing and doesn't refresh.
- While anything runs (list, create, delete, restore, settings write) `d` does nothing; while a
  create or delete runs, marking doesn't either. Esc clears the marks (after closing a prompt or
  overlay), and a second Esc closes the popup.

### Naming snapshots

Delete prompts, progress and results (single and bulk) name a snapshot by its label: the short
date and time, then the comment in quotes if it has one, `09-27 09:12 "bulk 1"` or
`09-27 09:12`. The whole comment is shown; the `>` line and the activity pane wrap. A name no
longer in the list shows as itself. The details pane and the helper's journal keep the full
name (`2026-09-27_09-12-33`), and the helper is always called with it.

## Snapshot browser (Phase 6a)

Enter or a double-click on a snapshot opens its files in the left pane. Needs `apsis-helper`
(`browsing and restoring need apsis-helper (sudo just install)`); the first folder asks for the
password (polkit `browse`, cached a few minutes).

```
 ╭─ 2026-09-25_03-00-01:/etc · 2 marked ─────────╮ ╭─ details ────────────────────╮
 │   = * fstab                          1.2KiB   │ │ path      /etc/hosts         │
 │ ▸ ~   hosts                            98B    │ │ type      file               │
 │   +   NetworkManager/                         │ │ size      98B                │
 │   =   vi -> ../usr/bin/vim.basic              │ │ modified  2026-09-20 10:02:11│
 │                                               │ │ mode      -rw-r--r--         │
 │                                               │ │ owner     root:root          │
 │                                               │ │ live      differs: size ...  │
 ╰───────────────────────────────────────────────╯ ╰──────────────────────────────╯
 [space]mark  [R]estore  [h]up  [r]eload                                      [esc]
```

- Title: the breadcrumb `snapshot:/path` (cut from the left with `…`), then `· N marked`.
- Rows: the running system's marker (`+` missing, accent; `~` changed, warning colour; `=`
  same, or a folder that exists, dimmed), `*` if marked, the name (`/` after folders,
  `-> target` for symlinks), the size of files. Folders first, then by name.
- Keys: j/k/↑/↓/Home/End move; Enter or `l` into a folder; Backspace or `h` up (selecting the
  folder you came from); space marks or unmarks and stays on the row; `J` marks or unmarks
  and moves down, for marking a run of rows (marks are kept across folders); `r` reads the
  folder again; `R` restores; Esc back to the snapshot list. Click selects, double-click goes
  into a folder or marks a file.
- Marked rows: an accent `*` and a fainter accent background than the selected row (the
  selection's background wins on the selected row).
- Details: path, type, size, modified, mode (`ls -l` style), owner, link target, and `live`:
  not on the running system / same size and time / folder exists (contents not compared) /
  differs (with both sizes and times for a file). A marked entry adds `restore   marked`.
- `R` restores the marked entries, or the selected one if none are marked:
  1. `> restore 3 item(s) to [f]older (~/Apsis-restored) or [o]riginal? _`; Enter with nothing
     or `f` is folder mode, `o` original, anything else cancels.
  2. The dry run (activity `restore dry run… ⠹`), then its plan in the left pane (title
     `restore plan`): every item to create, copy, link, replace (with its backup name) or
     change attributes of, the totals, warnings for `/etc` and `/usr`, and each rsync argv.
  3. Enter runs it. Original mode first asks `> put 3 item(s) back over the running system?
     [y/N] _` (only `y` runs it), and polkit asks for the password every time.
  4. Activity `restoring… ⠹`, then the result in the left pane (title `restore result`); the
     folder is read again. Esc goes back to the browser at any step.
- Folder mode copies into `~/Apsis-restored/<snapshot>/<original path>` (a new
  `<snapshot>-2/` and so on for each later restore), owned by you, without setuid bits,
  file capabilities, device nodes or FIFOs. Original mode keeps each file it replaces as
  `<name>.apsis-before-<snapshot>` next to it.

## Panel button

- Symbolic icon `io.github.atraxsrc.Apsis-symbolic` (an orbit with its two apsides), tinted by
  the theme. The popup header shows it in the accent colour before `~/apsis`.
- Tooltip: `Apsis - last snapshot 3h ago` or `Apsis - no snapshots`, plus ` · 483G free` when
  the last list knew the free space (statvfs, else Timeshift's own line).
- Left click: the popup. Right click: a small menu (a standard COSMIC applet menu, not the
  terminal look):

  ```
  Refresh                  opens the popup and lists, like [r]
  Settings…                opens the popup on the settings view
  About Apsis              opens the popup on the About view
  ─────────────────
  Remove or move applet…   cosmic-settings panel (where applets are removed or moved)
  Close                    closes the menu, like Esc
  ```

  No "Quit": the panel owns the applet process (removing the applet is the way to stop it). Only
  one of popup and menu is open at a time, so Close only ever has the menu to close. Esc closes
  the menu too.

## Settings view (Phase 4.5)

`s`, the `[s]ettings` hint, or **Settings…** in the right-click menu shows Timeshift's settings
in the left pane (title `settings`, `settings (unsaved)` with changes). The right pane explains
the selected row. Needs `apsis-helper`: it reads and writes `/etc/timeshift/timeshift.json`.

The explanation wraps and is never cut off. In the popup the details pane is sized once, to
the tallest row's details (at least as high as a list), and the settings list beside it
stretches to the same height, so the popup doesn't change height while moving between rows. A
window's panes keep the window's height, so the notes are short enough to fit its smallest size
(640 x 440). A layout test (`APSIS_LAYOUT_TEST=1`) checks every row both ways. A home row lists the three options, one line each:
`excluded`, `hidden files only` (app settings, not documents), `everything` (a full restore
rolls documents back too).

```
 ╭─ settings (unsaved) ──────────────────────────╮ ╭─ details ────────────────────╮
 │ ▸ device    sdb1  ext4  931.5G  Backup        │ │ path   /dev/sdb1             │
 │   mode      rsync                             │ │ type   ext4                  │
 │   schedule  [ ] monthly  keep 2               │ │ ...                          │
 │             [x] daily    keep 5               │ │                              │
 │   home      root       everything             │ │                              │
 │   filters   + /root/**                        │ │                              │
 │             + add filter…                     │ │                              │
 ╰───────────────────────────────────────────────╯ ╰──────────────────────────────╯
 [space]change  [+]  [-]  [a]dd  [x]remove  [w]rite  [r]eload                [esc]
```

- Rows: device (space picks the next device that can hold snapshots), mode (rsync/btrfs; btrfs
  only when a btrfs filesystem exists), `@home` (btrfs mode), the five schedules (space on/off,
  `+`/`-` or `e` for the count), home folders per user (rsync mode; space cycles excluded /
  hidden files only / everything, as Timeshift's Users tab), the filters (`x` removes), and
  `+ add filter…` (`a` anywhere).
- `> add filter: _` and `> keep daily: _` use the `>` line; Enter sets, Esc cancels. A bad value
  stays in the prompt with the reason in the activity pane.
- `w` checks the changes, then the helper writes them (password once, cached like create). The
  activity pane shows `writing settings ⠹`, then the result; the settings are read back and the
  list refreshed. `r` reads them again, dropping changes.
- Esc with unsaved changes warns once; the second Esc drops them.
- If Timeshift's own window is open, the activity pane warns: it saves its settings over these
  when it closes.
- Keys that act on snapshots (`c`, `d`) do nothing here.

## Native backend (Phase 5)

The settings view ends with Apsis's own section, `apsis`. It's saved to Apsis's cosmic-config
as soon as it changes (not by `w`, which only writes Timeshift's file):

```
 │   apsis     backend: timeshift                │
```

- `backend`: space switches between `timeshift` (default) and `native rsync`, then lists again.
  With native on, a `dry run` row follows: `[x] dry run` (on by default). The header summary
  reads `native · rsync · 3 snapshots`.
- Native lists read the backup disk directly (no `timeshift`), through `apsis-helper`, with no
  password. Snapshots Timeshift counts as incomplete, and a native create's leftovers, show as
  list warnings in the activity pane.
- `[c]` with dry run on: the activity pane shows `native dry run… ⠹`, then the plan appears in
  the left pane (title `dry run`): the snapshot name, the folders, the whole `exclude.list`, the
  `--link-dest` snapshot, the rsync argv, `info.json`, the rename and the tag links. Nothing is
  written. Esc goes back to the list. The same plan is in `journalctl -u apsis-helper`.
- `[c]` with dry run off: a real native create (password once, cached, like Timeshift's). The
  activity pane shows `creating snapshot… ⠹` as usual.
- `[d]` still deletes through Timeshift, which removes native snapshots like its own.
- Without `apsis-helper`: `the native backend needs apsis-helper (sudo just install)`.

## Window mode

`apsis --window` shows the popup's UI (header, panes, activity, `>` line, footer) in a normal
window instead of a panel button. The app launcher's entry (`io.github.atraxsrc.Apsis.Window.desktop`,
`Exec=apsis --window`) starts it that way; the applet's own entry is `NoDisplay=true`, so it only
shows in the panel settings.

- Started outside cosmic-panel (launcher, terminal, `just run`), `apsis` opens the window even
  without `--window`: on its own the panel button would be a tiny square. "In a panel" is
  libcosmic's own test: cosmic-panel sets `COSMIC_PANEL_NAME` for its applets.
- COSMIC header bar with the title `Apsis` and a close button; no maximize or minimize.
- Opens at 720 x 520 and can be resized by dragging its edges and corners, down to 640 x 440
  (the footer hints still fit). It starts floating even with tiling on: it appears with a
  fixed size, which COSMIC floats, and becomes resizable once it's on screen. The snapshots and details
  panes fill the extra height and share the width 3:2 (details wraps and scrolls); the activity
  pane, `>` line and footer stay at the bottom. Row comments are cut only by the pane width
  (the popup also cuts them at 28 characters).
- Same keys and focus as the popup: command keys work at once, `[c]` focuses the `>` line. Esc
  backs out of a prompt or overlay first, then closes the window (and quits).
- Lists as soon as it opens.

## About view

Shown inside the popup, like help and details; Esc goes back to the list.

```
 Apsis 0.1.0
 Timeshift-style system snapshots for the COSMIC™ desktop

 license   GPL-3.0-only
 source    https://github.com/atraxsrc/apsis
```

Version, license and repository come from `Cargo.toml`. The link opens with `xdg-open`.
