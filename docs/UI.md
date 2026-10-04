# UI

Apsis 0.4 is laid out like Timeshift, in standard libcosmic widgets and COSMIC theme colours
only (the contract: `docs/APSIS-UI-PROMPT.md`). There's a panel button with a read-only popup,
and a window (`apsis --window`) where the work happens. Up to 0.3 both looked like a small
terminal; that design is in git history and DECISIONS.md.

## Panel button

- Symbolic icon `io.github.atraxsrc.Apsis-symbolic` (an orbit with its two apsides), tinted by
  the theme. The warning role colours it when the reminder is due or less than 10% of the
  backup disk is free; the destructive role under 5% (the worse wins). A disk that isn't
  connected doesn't change it.
- Tooltip, from `apsis_core::status` through `StatusView`:

  ```
  last  12h ago
  next  manual only
  disk  372G / 596G  224G free
  ```

  Overdue: `last  9d ago  (over 7 days)`. Low space: `disk  28G free  (under 10%)`. Not
  connected: `disk  not connected`. Before the first list: `Apsis`.
- Optional label beside the icon on horizontal panels, `12h · 62%` (Misc tab, off by default).
- Left click: the popup. Right click: a standard applet menu: Open Apsis, Refresh, Settings…,
  About Apsis (these three open the window, on that page), Remove or move applet…, Close.
- The panel lists in the background at start and every 6 hours while the popup is closed,
  through the helper only (no password), for the reminder.

## Panel popup (read-only)

```
 ◎ Apsis
 Last snapshot   5d ago
 Backup disk     sdb1  373G / 932G · 40% used · 559G free
 ████████████████░░░░░░░░░░░░░░░░░░░░░░░░
 Creating snapshot · 58% · 3m 12s left          (only while a job runs)
 ████████████████████████░░░░░░░░░░░░░░░░
 ──────────────────────────────────────────
 2026-09-25 11:28   before driver update
 2026-09-23 08:33
 ... (the newest 5)
 [Open Apsis]                       [Refresh]
```

- 360 px wide. Nothing is changed here: Open Apsis starts the window (or brings the open one
  forward, with an activation token), Refresh lists again, Esc closes.
- A job running anywhere (another window, one that was closed) shows its line and bar, then
  its end (`Snapshot created`, `Create stopped`, ...). No Stop here: the window has it.
- No list, no device, empty, failed: the list area says so.

## Window

```
┌ Apsis ─────────────────────────────────────────── ↻  ⓘ  ✕ ┐
│ [+ Create]  [↺ Restore]  [🗑 Delete]  [⚙ Settings]         │
│  Snapshot             Comment                              │
│▌ 2026-09-25 11:28     before driver update                 │  selected
│  2026-09-23 08:33                                          │
│  2026-09-26 14:02     Interrupted snapshot, removed by ... │  dimmed (a leftover)
│ ────────────────────────────────────────────────────────── │
│  Last snapshot   5d ago                                    │
│  Backup disk     sdb1  885G / 932G · 95% used · 47G free (under 10%)
│  ███████████████████████████████████████████████████░░░    │
│  Creating snapshot · 58% · 3m 12s left          [ Stop ]   │  while a create runs
│  ██████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░    │
└────────────────────────────────────────────────────────────┘
```

- Opens at 720 x 520, resizable down to 640 x 440 (a layout test checks the smallest size). It
  starts floating even with tiling on, then becomes resizable once it's on screen.
- Header bar: the title, Refresh and About Apsis (icons with tooltips) on the list page.
- Toolbar: **Create** (enabled with a backup device listed and connected and no job running),
  **Restore** (0.5.0: also needs exactly one snapshot selected, not a leftover; no tooltip, no
  key), **Delete** (also needs a selection), **Settings**.
- List: libcosmic list rows, date and comment, newest first, then leftovers of interrupted
  creates (dimmed, "Interrupted snapshot, removed by the next one"). Click selects one;
  Ctrl-click adds or removes; Shift-click selects a range.
- Status area: last snapshot (with the reminder's note in the warning colour), backup disk with
  its bar (accent, warning under 10% free, destructive under 5%), the running job's line and bar
  with **Stop** for a create or a restore's preparation, the last result (an error in the
  destructive colour, the helper's own words in its tooltip; after login, the last restore's
  outcome, see Restore), and the list's warnings.
- An old-format snapshot (made before 0.4.1, without ACLs and extended attributes) says so in
  its row's tooltip. No glyph or column.
- Without rows: `Reading snapshots…`, `No snapshots yet. Click Create to make one.`, `No backup
  disk chosen.` with a button to the settings, or the error with **Try again**.

### Dialogs

| dialog | content | buttons |
|---|---|---|
| Create snapshot | Comment (optional); a dimmed "Includes the system, /root and 3 filters." | Create, Cancel |
| Delete N snapshots? | their labels (`09-25 11:28 "before driver update"`), "This can't be undone." | Delete (destructive), Cancel |
| Stop the snapshot? | "What was copied so far is deleted." | Stop (destructive), Keep Going |
| Add Pattern | Pattern, and how signs work | Add, Cancel |
| Save the settings? | leaving Settings with changes | Save, Discard, Cancel |
| Restore the system? | the date and comment (muted); "Puts system files back to this snapshot and restarts twice."; Home folders radios (keep, default; restore them too, with "Files changed after {date} are lost for good." when the safety snapshot is off); the safety snapshot checkbox (on); lines for no home in the snapshot, the old format, which Apsis the snapshot holds; "Experimental · …" (muted). The body scrolls within the window so the buttons always show | Restore (destructive), Cancel |
| Can't restore this snapshot | the date and comment; the reason in the destructive colour; what to do; after a refused Restart now, "The preparation was dropped. Restore again to measure afresh." | Close |
| Stop the restore? | "The safety snapshot being made is deleted. Nothing on the system has changed." | Stop (destructive), Keep Going |
| Ready to restore | the date and comment (muted), so the row is clear before the last click; "Save your work and close your apps first." and what happens next | Restart now (suggested), Cancel restore |

A comment the helper would refuse, or a bad pattern, keeps its dialog open with the reason.
Every dialog opens with libcosmic's default focus (none), so Enter does nothing until Tab
moves focus to a button.

### Jobs

- Create: the status area says `Creating snapshot · 1m 08s` until rsync has a number, then
  `Creating snapshot · 58% · 3m 12s left` with a bar. Delete of several: `Deleting 2 of 4: <label>…`
  (one at a time through the helper; one password; stops at the first failure and says what
  was deleted in the tooltip).
- Stop: shown once the create's snapshot name is known. After the confirm: `Stopping…`, then
  `Create stopped`. Too late (the snapshot is being put in place): `Couldn't stop: ...`, and it
  ends as `Snapshot created`.
- Every window and the popup subscribe to the helper's `JobChanged` and ask `Job()` when they
  open (only if the helper runs), so a job started elsewhere shows with its progress, and the
  list refreshes when a create or delete ends. A list or settings write ending elsewhere
  doesn't refresh (unless this window was told busy), so windows never list each other forever.
- A backup disk pulled out mid-job: `Create failed: backup disk removed` (the helper checks
  `/dev/disk/by-uuid`), with rsync's error in the tooltip.

### Restore (0.5.0)

The whole flow is PLAN.md's Phase 6b; the window's part:

- **Restore** asks the helper's `CheckRestore` first (the toolbar waits), then opens "Restore
  the system?" or "Can't restore this snapshot". Confirming starts the preparation (one
  password, asked every time): `Preparing restore · checking the snapshot…`, then
  `Preparing restore · safety snapshot · 42% · 3m 00s left` with a bar and **Stop**, then
  `Preparing restore · ready` (full bar, no Stop) under the "Ready to restore" prompt.
- At the prompt, **Esc**, **Cancel restore** and closing the window all cancel: the helper
  removes the plan (a finished safety snapshot stays), and the line says `Restore cancelled`.
  **Restart now** re-checks, arms the next boot and restarts: `Restarting…`. A refusal there
  opens "Can't restore" with the dropped-plan line, and the status says `Restore stopped`; a
  plan older than 30 minutes, or one the helper no longer has, says `The preparation is too
  old…` / `…is gone. Start the restore again.`
- While a plan is ready, here or in another window, Create, Restore and Delete are off; the
  other window's line says `Restore ready in another window`. Its end refreshes the list.
- **After login** the window asks `RestoreResult` and shows the last restore on the status
  line, with the full text in a tooltip: `System restored to {date}`; `System restored ·`
  **`still boots the previous kernel`** `· see README` (warning); `Restore` **`incomplete`**
  `· system partly restored · {what to do}` (destructive) with **Restore again**, which opens
  the normal dialog for the same snapshot; `The restore didn't start · nothing was changed ·
  {what to do}`. No notification, and the panel icon and tooltip don't change.
- The boot screen (plymouth) is the helper's: "Restoring the system. Don't turn off the
  computer." and a progress bar.

### Settings

A page in the same window: back button, **Cancel** (drop changes) and **Save** (write
`config.toml`; one password), and four tabs (libcosmic tab bar).

- **Location**: radio list of the disks that can hold snapshots (path, filesystem, size,
  label); the chosen one shown as `Not connected` when it's unplugged; other Linux filesystems
  listed dimmed with why they can't (encrypted, a container).
- **Include**: "The system is always included." and checkboxes for `/root` (the root user's
  home) and `/home` (every user's home), then what a full restore does with them.
- **Filters**: a list under the heads Keep and Pattern; each row a checkbox (checked keeps the
  path, `+`; unchecked leaves it out, `-`), the sign in the accent colour, and the pattern; then Add Folder and Add File (the portal file chooser, as the user), Add Pattern,
  Remove, Move Up, Move Down (enabled for a selected row). New rows go on top. A picked folder
  becomes `<path>/***`, a file `<path>`, rsync wildcards escaped; the sign is `-`, or `+` inside
  `/root` or `/home` while that isn't included. The help line: "Checked (+) keeps a path,
  unchecked (-) leaves it out. The first line that matches wins, top to bottom. Built-in
  excludes (/proc, /dev, caches) come before these."
- **Misc**: Remind me (spin button, days, 0 = off) and Panel label (toggle). Apsis's own, per
  user, saved at once to cosmic-config.
- Converted (from Apsis 0.3) or imported (from Timeshift) settings show a card at the top with
  the conversion's notes, and stay unsaved until Save.

### Keyboard

Shortcuts work in the window; the UI doesn't list them (man page and README do): Ctrl+N
create, Delete, Ctrl+R and F5 refresh, Ctrl+, settings, Ctrl+A select all, Up and Down move
the selection, Esc closes a dialog (on the ready prompt that's Cancel restore), then leaves
Settings or About, then clears the selection. A key a text field took is left to it (except
Esc). **Restore has no key** (owner, 2026-09-30): a click, or Tab to the button and Enter.
Enter and double-click on the list open nothing.

## Backup disk presence

Every 5 s, and when the popup or window opens, Apsis looks for the disk's link in
`/dev/disk/by-uuid` (no root, no mount). Gone: the status says `not connected` and Create waits;
the list stays. Back (only on the change from gone to back): it lists again.

## About

A page in the window (libcosmic's about widget): name, icon, version and license from
`Cargo.toml`, and a link to the source.

## Tests

- `crates/apsis/src/app/tests.rs`: messages into the model (toolbar enabling, selection,
  dialogs, stop, jobs from elsewhere, settings, shortcuts, disk presence, reminder).
- `APSIS_LAYOUT_TEST=1 cargo test -p apsis fit`: the window and every settings tab fit 640 x
  440, the popup its width, and every restore state (the dialogs with their button rows, at
  most the window's height minus 16 px; the page under them) fits both 720 x 520 and 640 x
  440 (measures real text, so it depends on installed fonts). With `APSIS_SCREENSHOTS=<dir>`
  the restore states are also written as `restore-<state>-<w>x<h>.rgba`.
- `APSIS_SCREENSHOTS=<dir> cargo test -p apsis screenshots`: renders the window, a running
  create, each settings tab, a dialog and the popup (dark and light) to `.rgba` files. The
  libcosmic bars animate from empty, so a single frame shows them empty.
