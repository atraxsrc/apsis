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
- Enter or a double-click makes the details pane active; Enter again, Esc or a click on a row
  goes back to the list.
- Activity pane: the running create/delete with a spinner (accent border while it runs), else the
  last result, else `idle` (dimmed). A failure shows Timeshift's last lines. Below that, any
  warnings from the last list (`list: E: Failed to remove directory`) in the theme's warning colour.
- The `>` line stays the input line (comment, `[y/N]`, and the `timeshift --list` spinner). The
  key hints are the footer, the last row.

## Rules

- Monospace font for everything. Text uses theme text colours; the selected row uses the accent
  colour as a left marker `▸` and a subtle accent background — not a hard-coded colour.
- Every `[key]` hint is also a clickable button. Every row is clickable (select), double-click = details.
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
| d | delete selected → `> delete 2026-09-18_12-41-00? [y/N] _`; Enter with `y` deletes, anything else cancels |
| r | refresh list |
| Enter | make the details pane active, or go back to the list |
| s | settings view (see below) |
| ? | help overlay |
| Esc | cancel input, then go back from details/help/about/settings, then close the popup |

## States

| state | shows |
|---|---|
| loading | `> timeshift --list` then a spinner character cycling `⠋⠙⠹⠸…` |
| empty | `no snapshots yet — press [c] to create one` |
| running | `creating snapshot… ⠹` / `deleting <name>… ⠹` in the activity pane, keys disabled except Esc (does not cancel root op) |
| result | in the activity pane: `snapshot created` / `deleted <name>` (dimmed), or `create failed: <last stderr line>` (error colour); stays until the next create/delete |
| error | `error:` + Timeshift's last lines (its `E:`/`W:` lines and stderr), `[r]etry` |
| disk missing | `backup disk not connected (UUID 1a2b…): plug it in and press r` (list error, or the create/delete result) |
| not installed | `timeshift not found — install it or wait for the native backend` |

## Panel button

- Symbolic icon `io.github.atraxsrc.Apsis-symbolic` (an orbit with its two apsides), tinted by
  the theme. The popup header shows it in the accent colour before `~/apsis`.
- Tooltip: `Apsis — last snapshot 3h ago` or `Apsis — no snapshots`.
- Left click: the popup. Right click: a small menu (a standard COSMIC applet menu, not the
  terminal look):

  ```
  Refresh              opens the popup and lists, like [r]
  About Apsis          opens the popup on the About view
  ─────────────────
  Panel settings…      cosmic-settings panel
  ```

  No "Quit": the panel owns the applet process. Only one of popup and menu is open at a time.
  Esc closes the menu.

## Settings view (Phase 4.5)

`s`, the `[s]ettings` hint, or **Settings…** in the right-click menu shows Timeshift's settings
in the left pane (title `settings`, `settings (unsaved)` with changes). The right pane explains
the selected row. Needs `apsis-helper`: it reads and writes `/etc/timeshift/timeshift.json`.

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

## Window mode

`apsis --window` shows the popup's UI (header, panes, activity, `>` line, footer) in a normal
window instead of a panel button. The app launcher's entry (`io.github.atraxsrc.Apsis.Window.desktop`,
`Exec=apsis --window`) starts it that way; the applet's own entry is `NoDisplay=true`, so it only
shows in the panel settings.

- Started outside cosmic-panel (launcher, terminal, `just run`), `apsis` opens the window even
  without `--window`: on its own the panel button would be a tiny square. "In a panel" is
  libcosmic's own test: cosmic-panel sets `COSMIC_PANEL_NAME` for its applets.
- COSMIC header bar with the title `Apsis` and a close button; no maximize or minimize.
- Fixed 720 x 520 (min size = max size) and not resizable, so COSMIC floats it instead of tiling.
  The snapshots and details panes fill the height; the list scrolls past what fits.
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
