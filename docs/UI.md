# UI — terminal-style popup

A panel applet. Clicking the panel icon opens a popup that looks like a tiny terminal.
It's a normal libcosmic popup, so it always floats and follows the theme (colours, radius, font scale).

## Mock

```
 ~/apsis $ ls --snapshots                     rsync · 3 snapshots
 ────────────────────────────────────────────────────────────────
 ▸ 2026-09-25 03:00   D     boot
   2026-09-24 03:00   D
   2026-09-18 12:41   O     "before kernel update"
 ────────────────────────────────────────────────────────────────
 [c]reate  [d]elete  [r]efresh  [?]help                     [esc]
 > _
```

## Rules

- Monospace font for everything. Text uses theme text colours; the selected row uses the accent
  colour as a left marker `▸` and a subtle accent background — not a hard-coded colour.
- Every `[key]` hint is also a clickable button. Every row is clickable (select), double-click = details.
- The bottom `>` line is the input line: used for the create comment and delete confirm.
  `[c]reate` and `[d]elete` are only enabled once a list has shown a snapshot device, and
  nothing else is running; delete also needs a selected snapshot.
- Width ~ 520 px logical, height fits up to ~8 rows then scrolls.

## Keys

| key | action |
|---|---|
| ↑/↓, j/k | move selection |
| c | create → input line becomes `> comment: _`, Enter runs, Esc cancels |
| d | delete selected → `> delete 2026-09-18_12-41-00? [y/N] _`; Enter with `y` deletes, anything else cancels |
| r | refresh list |
| Enter | show details (tags, comment, full name) |
| ? | help overlay |
| Esc | cancel input, then close details/help, then the popup |

## States

| state | shows |
|---|---|
| loading | `> timeshift --list` then a spinner character cycling `⠋⠙⠹⠸…` |
| empty | `no snapshots yet — press [c] to create one` |
| running | `> creating snapshot… ⠹` / `> deleting <name>… ⠹` in the input line, keys disabled except Esc (does not cancel root op) |
| result | one line above `>`: `snapshot created` / `deleted <name>` (dimmed), or `create failed: <last stderr line>` (error colour); stays until the next create/delete |
| error | `error:` + last lines of stderr, `[r]etry` |
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

## About view

Shown inside the popup, like help and details; Esc goes back to the list.

```
 Apsis 0.1.0
 Timeshift-style system snapshots for the COSMIC™ desktop

 license   GPL-3.0-only
 source    https://github.com/atraxsrc/apsis
```

Version, license and repository come from `Cargo.toml`. The link opens with `xdg-open`.
