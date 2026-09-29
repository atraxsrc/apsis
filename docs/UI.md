# UI — terminal-style popup and window

A panel applet. Clicking the panel icon opens a popup that looks like a tiny terminal: a
**read-only overview** (see "Panel popup" below). The work (create, delete, browse, restore,
settings) happens in the **window**, `apsis --window`. It's a normal libcosmic popup, so it always
floats and follows the theme (colours, radius, font scale).

Everything below the overview section (the mock, keys, states, bulk delete, the browser and the
settings view) describes the window's UI, which is the full UI the popup used to have.

## Panel popup (read-only overview)

```
 ◎ ~/apsis $ status                                  rsync · 9 snapshots
 ╭─ apsis ───────────────────────────────────────╮
 │ last   12h ago                                │
 │ next   manual only                            │
 │ sda1   372G / 596G                            │
 │ ████████████████████░░░░░░░░░░░░░░░░░░░░░░░░  │
 │ 62% used · 224G free                          │
 ╰───────────────────────────────────────────────╯
 ╭─ snapshots ───────────────────────────────────╮
 │ 2026-09-29 06:12  D   "pre-nvidia"            │
 │ 2026-09-28 18:01  O                           │
 │ ... (the newest 5)                            │
 │ +4 older                                      │
 ╰───────────────────────────────────────────────╯
 [o]pen apsis  [r]efresh                  [esc]
```

- 400 px wide (the window is 720). The popup only looks: no create, delete, restore or settings.
  The panel's `read_only` flag gates the keys; the window's code is unchanged.
- Keys: `o` (or the `[o]pen apsis` hint) starts `apsis --window` in its own process and closes
  the popup; `r` lists again; Esc closes. Every other key does nothing.
- Same text as the tooltip and the window's strip (`StatusView::strip`): `last`, `next  manual
  only`, the device and sizes, the disk bar, `NN% used · X free`, with the overdue and low-space
  phrases in the warning or destructive colour. Only the newest 5 snapshots, as plain rows (date,
  tags, a comment cut to 14 characters): no selection, no dots, no status column.
- No list yet, no device, empty, or a failed list: the pane shows the same message as the window.
- Right-click menu: `Open Apsis` (new), `Refresh`, `Settings…` and `About Apsis` (these two open
  the window on that view: `apsis --settings`, `apsis --about`), then `Remove or move applet…`
  and `Close`.
- A running create is not shown here: it runs in the window's process, and the popup lists only
  when it opens or on `r`. Showing it would need the helper to expose its running state
  (roadmap).
- The window starts from `std::env::current_exe()`, so it is the same build as the panel; a
  package upgrade while the panel runs (`... (deleted)` on the path) is handled.

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
  last result, else `idle` (dimmed). A failure shows the helper's reason. Below that, any
  warnings from the last list (`list: 2026-09-02_09-00-00: incomplete: no info.json`) in the
  theme's warning colour.
- The `>` line stays the input line (comment, `[y/N]`, and the list's spinner). The key hints
  are the footer, the last row.
- Disk line, between the panes and the activity pane: see "Disk usage" below.
- Progress of a create or a (real) restore replaces the spinner line: see "Progress" below.

## Keep last N snapshots

```
 ╭─ prune preview ─────────────────────────────────╮
 │ delete  09-21 10:00                             │
 │ delete  09-22 10:00                             │
 │ keep    09-25 10:00  newest 2                   │
 │ keep    09-24 10:00  newest 2                   │
 │ keep    09-20 10:00 "before upgrade"  comment   │
 ╰─────────────────────────────────────────────────╯
 > remove 2 old snapshots? [y/N] _
```

- Settings view, `apsis` section: `keep last 10` / `keep: all` (off, default). Apsis's own
  (cosmic-config), saved at once. `space` on/off (10), `+`/`-`/`e` 1 to 999.
- Rules (`apsis_core::retention::manual`): a snapshot with a comment is pinned: it always stays
  and doesn't count towards N. The newest N uncommented snapshots stay, whatever their tags,
  and the newest snapshot of all never goes. The rest are listed oldest first.
- Never automatic: `p` any time, and after a successful create (when its list is in) the same
  preview and question appear by themselves if something is past N. Deleting is the bulk
  delete: through the helper one at a time (one password).
- It sees what the list shows, which is every system's snapshots on the device.

## Reminder

- Settings view, `apsis` section: `remind after 7 days` (default) / `remind: off`. `space`
  on/off, `+`/`-`/`e` 1 to 365 days.
- Due when a list showed a backup device and its newest snapshot (any tag) is more than N days
  old, or there is none.
- Then the panel icon is drawn in the theme's warning colour (same icon, `warning_text_color`)
  and the tooltip's `last` line adds `(over 7 days)` (`last  none yet  (over 7 days)` with no
  snapshot). With no backup device set the reminder is off.
- So that it works without opening the popup: in applet mode Apsis lists at start and every
  6 hours while the popup is closed, through the helper (no password; no helper: nothing). A
  failed background list (disk unplugged) clears the reminder.

## Progress

```
 ╭─ activity ───────────────────────────────────────────────────────────────────╮
 │ creating snapshot · 58% · 3m 12s left  ██████████████░░░░░░░░░░░░░░░░        │
 ╰──────────────────────────────────────────────────────────────────────────────╯
```

- For creates and real restores, through `apsis-helper`: its `Progress` signal, at most about
  twice a second. The line is the same in the popup and in `--window`.
- Before the first `Progress`: the usual `creating snapshot… ⠋`.
- After the helper's first (number-less) `Progress`, until there's a real number:
  `creating snapshot · working · 1m 08s elapsed ⠹` (the spinner is the pulse). `0%` never
  counts as a number. Elapsed counts from when the create or restore started.
- With a number: `<label> · 58% · 3m 12s left`, then a bar in the theme's accent colour (the
  empty part dimmed) filling the rest of the line. The percent is rounded down. Time left and
  elapsed are to the second: `42s`, `3m 12s`, `1h 02m` (seconds dropped from an hour up). No
  time left while it's unknown (`<label> · 58%`), and for all but the last path of an
  original-mode restore (one rsync per path, the percent spread over them). A restore's label
  is `restoring`.
- The bar is the disk meter's widget (`bar`): the same monospace cells, filled in the accent
  colour here, and in the warning or destructive colour for low disk space.
- While a job runs, the strip's `last`, `next` and disk stay as the last list showed them; the
  list refreshes when it finishes.
- Where the numbers come from: rsync's `--info=progress2`. No `2.1G / 3.2G`: rsync gives bytes
  copied but no reliable total.
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
- Where the numbers come from: `apsis-helper`'s `NativeListWithUsage`, `statvfs` on
  `/run/apsis/backup` while the list has the device mounted.
- Unknown usage (e.g. no device selected, or the disk not connected): no line. No bar is ever
  drawn from a guess.
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
| p | prune: preview which old manual snapshots "keep manual" would delete (left pane), then `> remove 3 old manual snapshots? [y/N]`; `y` deletes them one by one (as a bulk delete), anything else or Esc closes the preview |
| s | settings view (see below) |
| ? | help overlay |
| Esc | cancel input, then go back from details/help/about/settings, then clear the marks, then close the popup |

## States

| state | shows |
|---|---|
| loading | `reading snapshots…` then a spinner character cycling `⠋⠙⠹⠸…` |
| empty | `no snapshots yet — press [c] to create one` |
| running | `creating snapshot… ⠹` / `deleting <label>… ⠹` / `deleting 2/4: <label>… ⠹` in the activity pane, keys disabled except Esc (does not cancel root op) |
| result | in the activity pane: `snapshot created` / `deleted <label>` / `deleted 4 snapshots` (dimmed), or `create failed: <reason>` (error colour); stays until the next create/delete |
| bulk delete stopped | `delete stopped at <label>: <reason>`, `deleted (1): <labels>`, `not deleted (3): <labels>` (error colour); the ones not deleted stay marked, the list refreshes if any was deleted |
| error | `error:` + the helper's reason, `[r]etry` |
| disk missing | `backup disk not connected (UUID 1a2b…): plug it in and press r` (list error, or the create/delete result) |
| no helper | `Apsis needs apsis-helper, which the .deb installs` |
| no device | `no backup device chosen` / `pick one in settings [s], then press [r]` |

## Bulk delete

Mark several snapshots, then `d`.

- space marks or unmarks the selected snapshot and stays on it; `J` marks or unmarks and moves
  down. Marks work in the list and the details pane, not on help, About or settings. A refresh
  drops marks of snapshots that are gone; a failed list or closing the popup drops them all.
- `d` with marks: `> delete 3 snapshots: <label>, <label>, <label>? [y/N] _`, in list order
  (newest first). Only `y` deletes. Without marks, `d` is the single delete as before.
- The snapshots go one at a time through the helper (`Delete`, one call each), so polkit asks for
  the password once (`auth_admin_keep`) and the next calls use the cached answer. Activity:
  `deleting 2/4: <label>… ⠹`. No list runs between
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

## The apsis strip (window)

In `--window`, an `apsis` pane sits above the snapshots and details, from `StatusView::strip`
(the same source as the tooltip, so they never disagree):

```
 ╭─ apsis ───────────────────────────────────────────────╮
 │ last   12h ago                sda1  372G / 596G       │
 │ next   manual only            ████████████░░░░░░░░    │
 │                               62% used · 224G free    │
 ╰───────────────────────────────────────────────────────╯
```

- Left: `last` and `next` (labels in the accent colour, values in the text colour). `next` is
  always `manual only` (no scheduler). Overdue adds `(over 7 days)` in the warning colour, and
  only that phrase.
- Right: the backup device and `used / total`, the disk bar (accent, warning or destructive by
  free space, the same widget as the popup's disk line), then `NN% used · X free`. Under 10% or
  5% free adds `(under 10%)` / `(under 5%)` in the warning / destructive colour.
- No disk: `disk  not connected` (the list failed, or no device set) or `disk  unknown` (no
  `statvfs` figures). Before the first list: `-`.
- Not shown in the settings view (its notes are sized to the smallest window without it) and
  not in the popup, which keeps its disk line until it becomes the read-only overview.
- The window's old disk line is gone; the strip replaces it. At the smallest window (640 x 440)
  the strip is 91 px and the panes keep 184 px (`the_strip_fits_the_smallest_window`).
- The pane title is dim like the other inactive panes; the accent-everywhere look of the
  mockups is the window-chrome slice.

## Rooms and the dock (window)

Four outlined cells under the activity pane, in the accent colour; the active room's cell has
the selected row's fill (`dock_cell`). `1 2 3 4` or a click jump between them; every cell is a
key. The dock is in `--window` only, and only on the snapshot views (not in the settings view
or the browser, which have their own footers). The footer gains a dim `[1-4]rooms`.

| room | key | what it shows |
|---|---|---|
| snapshots | `1` | home: the list and its details |
| create | `2` | the list stays; a `create` pane replaces details: `comment` (as typed, mirrored from the `>` line), `tag  O on-demand`, and `[enter]create` / `[esc]cancel`. `2` starts the comment prompt, as `c` does |
| schedule | `3` | `next  manual only`, then `keep` and `remind` with `[-] [+] [on]`; up/down pick a row, space turns it on or off, `+` `-` change it. Saved at once, like the settings view's rows. There is no scheduler yet, and the pane says so |
| log | `4` | what happened this session, newest first: `12:04:11  snapshot created`, a failure marked `error` (the word in the destructive colour, the rest text). At most 200 lines |

- Schedule and log replace the snapshots and details panes; the apsis strip and the activity
  pane stay. Create keeps the list.
- The log is fed by the status line: whatever a message leaves in the activity pane (created,
  deleted, failed, saved, cancelled) and a list that fails (once, not on every message).
- Digits typed at a prompt are text: room keys only work at the command line. Esc walks back:
  the prompt, then the room (to snapshots), then the marks, then the window.
- Rooms wait while a create, delete or restore runs.
- At the smallest window (640 x 440) each room leaves the panes 172 px and the dock 26 px; the
  schedule and log rooms scroll rather than push the dock out.

## Panel button

- Symbolic icon `io.github.atraxsrc.Apsis-symbolic` (an orbit with its two apsides), tinted by
  the theme. The popup header shows it in the accent colour before `~/apsis`.
- Tooltip, from `apsis_core::status` through `StatusView` (one formatter, reused by later
  phases):

  ```
  last  12h ago
  next  manual only
  disk  372G / 596G  224G free
  ```

  Overdue: `last  9d ago  (over 7 days)`. No snapshots: `last  none yet`. Reminder off: no note.
  Free space under 10% / 5% of the usable space: `disk  28G free  (under 10%)` / `(under 5%)`.
  Backup disk not connected (or the list failed): `disk  not connected`; a list without
  `statvfs` figures: `disk  unknown`. Before the first list: `Apsis`. `next` is always
  `manual only` (no scheduler).
- Icon colour, icon only: warning role for a due reminder or free < 10%, destructive role for
  free < 5% (the worse wins). No hardcoded colours.
- Optional label beside the icon, `12h · 62%` (age of the newest snapshot, share of the backup
  disk in use, `-` for an unknown side), on horizontal panels only. Settings view, `apsis`
  section: `panel label: off` (default) / `on`, toggled with `space`. Text colour only; a
  warning colours the icon, not the label.
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

## Settings view (0.2.0)

`s`, the `[s]ettings` hint, or **Settings…** in the right-click menu shows the settings in the
left pane (title `settings`, `settings (unsaved)` with changes). The right pane explains the
selected row. Needs `apsis-helper`: it reads `/etc/apsis/config.toml` (or, before it exists,
imports Timeshift's `/etc/timeshift/timeshift.json`, only reading it) and writes
`config.toml`.

The explanation wraps and is never cut off. In the popup the details pane is sized once, to
the tallest row's details (at least as high as a list), and the settings list beside it
stretches to the same height, so the popup doesn't change height while moving between rows. A
window's panes keep the window's height, so the notes are short enough to fit its smallest size
(640 x 440). A layout test (`APSIS_LAYOUT_TEST=1`) checks every row both ways. A home row lists
the three options, one line each: `excluded`, `hidden files only` (app settings, not
documents), `everything` (a full restore rolls documents back too).

```
 ╭─ settings (unsaved) ──────────────────────────╮ ╭─ details ────────────────────╮
 │ ▸ device    sdb1  ext4  931.5G  Backup        │ │ path   /dev/sdb1             │
 │   home      root       everything             │ │ type   ext4                  │
 │             user1      hidden files only      │ │ ...                          │
 │   filters   + /root/**                        │ │                              │
 │             /var/lib/libvirt/**               │ │                              │
 │             + add filter…                     │ │                              │
 │   apsis     keep: all                         │ │                              │
 │             remind after 7 days               │ │                              │
 │             panel label: off                  │ │                              │
 ╰───────────────────────────────────────────────╯ ╰──────────────────────────────╯
 [space]change  [+]  [-]  [a]dd  [x]remove  [w]rite  [r]eload                [esc]
```

- Rows: `device` (space picks the next unencrypted Linux filesystem that can hold snapshots),
  `home` per user (space cycles excluded / hidden files only / everything, Timeshift's
  patterns, kept in the filter list), the `filters` in order (home patterns dimmed; `x`
  removes a filter), `+ add filter…` (`a` anywhere), then Apsis's own `keep`, `remind` and `panel label`
  (per user, saved at once to cosmic-config, not by `w`; `+`/`-` or `e` change the number).
- **Import.** While there's no `config.toml`, the view shows what was read from Timeshift's
  settings, marked unsaved, and the activity pane lists it, in the warning colour:

  ```
  imported from /etc/timeshift/timeshift.json (not saved yet; w saves)
    device   8cecb045… (sda1, ext4)
    home     root: everything, user1: hidden files only
    filters  2
    not used schedule and counts (Apsis doesn't schedule)
  ```

  Without `timeshift.json` the view starts empty: `device  none selected`.
- `> add filter: _` and `> keep (0 = off): _` use the `>` line; Enter sets, Esc cancels. A bad
  value stays in the prompt with the reason in the activity pane.
- `w` checks the changes (a changed device must be connected and plain; filters not blank or
  repeated), then the helper writes them (password once, cached like create). The activity pane
  shows `writing settings ⠹`, then `settings written to /etc/apsis/config.toml`; the settings
  are read back and the list refreshed. `r` reads them again, dropping changes.
- Esc with unsaved changes warns once; the second Esc drops them (an import stays an import).
- Keys that act on snapshots (`c`, `d`) do nothing here.
- Without `apsis-helper`: `settings need apsis-helper (install the .deb)`.

## Window mode

`apsis --window` shows the full UI (header, apsis strip, panes, activity, `>` line, footer) in a
normal window instead of a panel button. `--settings` and `--about` open it on that view (the
panel's menu uses them). The app launcher's entry (`io.github.atraxsrc.Apsis.Window.desktop`,
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
 Apsis 0.2.0
 Simple system snapshots and file restore for the COSMIC™ desktop

 license   GPL-3.0-only
 source    https://github.com/atraxsrc/apsis
```

Version, license and repository come from `Cargo.toml`. The link opens with `xdg-open`.
