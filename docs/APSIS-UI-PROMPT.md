# Apsis — standing UI / TUI prompt

Use this file as the contract for every polishing phase.
Paste the **Session kickoff** block at the start of a Claude Code turn.
The rest stays loaded (copy into `CLAUDE.md` or point `CLAUDE.md` at this file).

Agent rules in the repo `CLAUDE.md` still win: no sudo, no pkexec, no
timeshift/helper write ops, no invented app id, fixtures scrubbed,
`docs/DECISIONS.md` append-only.

---

## Session kickoff (paste each turn)

```text
Read CLAUDE.md, docs/PLAN.md, docs/ARCHITECTURE.md, docs/UI.md,
docs/DECISIONS.md, and docs/APSIS-UI-PROMPT.md (this file).

Current phase: <PHASE NAME FROM PLAN.md>
Current task: <ONE SENTENCE>

Work one slice. Do not redesign the app. Follow the visual and
product rules in APSIS-UI-PROMPT.md.
Out of scope unless the task says otherwise: restore, btrfs,
encrypted disks, new crates, new colors.

After the slice: cargo test --workspace && cargo clippy --workspace -- -D warnings
&& cargo fmt --all. Then stop and summarize files touched.
```

---

## 1. What Apsis is

Apsis is a native **libcosmic** program for Pop!_OS / COSMIC.

It looks like a small terminal console. It is not a GTK clone of
Timeshift and not a rainbow dashboard.

Timeshift is optional compatibility. The happy path is Apsis native
rsync + polkit helper + a backup disk. If Timeshift is missing, list /
create / delete / status must still work.

**The product on screen is two paired numbers:**

| Axis | Question | Idle copy |
| --- | --- | --- |
| Time | Last snapshot? Next one? | `last  12h ago` · `next  in 6h (daily)` |
| Disk | Backup volume fill | `sda1  372G / 596G` · `224G free` |

That pair is named **apsis**. Snapshots, create, schedule, and log are
rooms under it.

Full-system restore is not part of polishing phases that use this
prompt. File-level restore already exists and stays experimental;
do not restyle or expand it unless the phase says so.

---

## 2. Visual language (non-negotiable)

Copy the current applet screenshot, not the Catppuccin mockups.

### Two colors, plus theme roles

| Token | Source | Use |
| --- | --- | --- |
| Background | Cosmic `bg_color()` | window, pane fill |
| Text | Cosmic `on_bg_color()` | dates, comments, values |
| Accent | Cosmic accent / `accent_text_color()` | borders, pane titles, labels, bars, tabs, key hints |
| Selection | accent at ~20–30% opacity | current row, active tab |
| Muted | text at ~55% | `rsync · 9 snapshots`, idle, secondary figures |
| Warning | Cosmic `warning` role | last snap older than N days; free < 10% |
| Destructive | Cosmic `destructive` role | free < 5% |

Warning / destructive apply to **the icon or the one affected phrase**.
They are not badges, not row status columns, not painted green / red.

### Forbidden

- Hardcoded hex (no `#89b4fa`, no Mocha palette, no “cyan” literals)
- Green OK / red WARN / yellow dots on snapshot rows
- Multi-color tag chips — `O B H D W M` stay text color
- Health chrome (“all systems nominal”)
- Material cards, large filled pills, traffic-light window dots
- A second accent “for branding”

When Cosmic theme changes, Apsis changes with it. Live.

### Chrome

- Monospace body
- Thin accent pane borders
- Pane titles in accent (`snapshots`, `details`, `activity`, `apsis`)
- Prompt line may stay: `~/apsis $ ls --snapshots`
- Footer is key hints in accent: `[c]reate [d]elete [j k] [esc]`
- Corner radius and spacing from Cosmic theme tokens

It should still read as the same app as `docs/screenshot.png`.

---

## 3. Information architecture

Four rooms. Nothing else is a main room.

| Room | Job | Notes |
| --- | --- | --- |
| **Snapshots** | list, select, delete, open details | Always available. Home. |
| **Create** | comment + optional tag + one action | Must not replace the list in `--window` |
| **Schedule** | enable tags + keep N | Edits policy; strip only shows soonest due |
| **Log** | helper output, last error | Visible while a job runs if the user opens it |

Settings / first-run / filters stay an advanced screen (`s` or a
settings pane title), not a fifth dock item, unless PLAN already
put them there.

Details is a **pane**, not a room. It sits beside snapshots.

Activity is a **pane**, not a room. It sits under the split.

Apsis (time + disk) is a **pane**, not a room. It sits above the split.

---

## 4. Layout D — window

Default for `apsis --window`.

```
┌ Apsis ────────────────────────────────────────────── x ┐
│ ~/apsis $ ls --snapshots          rsync · 9 snapshots  │
├ apsis ─────────────────────────────────────────────────┤
│ last   12h ago              sda1   372G / 596G         │
│ next   in 6h  (daily)       disk   [████████░░░░] 62%  │
│                             224G free                  │
├ snapshots ──────────────┬ details ─────────────────────┤
│ 2026-09-29 06:12  D  …  │ path                         │
│ 2026-09-28 18:01  O  …  │ type                         │
│                         │ size                         │
│                         │ comment                      │
├ activity ───────────────┴──────────────────────────────┤
│ idle                                                   │
│   OR  creating snapshot · 42% · 3m 12s left  [████░░]  │
├────────────────────────────────────────────────────────┤
│ [c]reate [d]elete [j k]move [1-4]rooms [s]ettings [esc]│
│ [snapshots]   [create]   [schedule]   [log]            │
└────────────────────────────────────────────────────────┘
```

### Dock / tabs

- Four outlined cells, accent border, text in accent
- Active cell uses the **same fill as the selected list row**
- Not mauve pills, not icons-as-identity
- Keys `1 2 3 4` jump the four rooms
- In `--window`, switching to Create / Schedule / Log should **not**
  destroy the snapshots list if width allows (split or stacked rooms).
  If width is tight, rooms replace the split but the apsis + activity
  panes stay mounted.

### Undock (later polish only)

Detail or Log may grow an `undock` affordance that opens a second
Cosmic window on the same message bus. Do not build an inner WM.
Do not implement undock unless the phase names it.

---

## 5. Layout — panel popup

The popup is the same language, narrower. Do not force the four-tab
dock if it steals list height.

Order:

1. Prompt / title
2. **apsis** strip (time | disk). Bar allowed. No `this snap ~1.8G`
3. Snapshot list (as many rows as fit; four is enough if cramped)
4. Details only if there is width; otherwise details on the selected
   row’s second line or on Enter
5. **activity**
6. Footer keys

Existing settings view stays reachable with `s`.

> **Update 2026-09-29 (supersedes the list and the line above):** the popup is a read-only
> overview. It shows the apsis strip (last / due / disk bar), a read-only list of the newest few
> snapshots, the activity line (progress of jobs started in the window), and one clear way to
> open the window: Enter, or a click on the title or a row (opens the window with that snapshot
> selected). Esc closes it. No settings (`s`), no create, delete, mark or restore in the popup.
> Setup and error states (disk not connected, first run, create failed) are one muted line plus
> `open apsis`. The right-click menu keeps Refresh, Close and "Remove or move applet...";
> Settings and About live in the window.

---

## 6. Panel button (closed)

Label, one line:

```
12h · 62%
```

Meaning: age of newest snapshot · used percent of **backup** disk.

Unknowns: `— · —` or omit the missing side (`12h · —`).

Long tooltip:

```
last  12h ago
next  in 6h  (daily)
disk  372G / 596G   224G free
```

Icon uses warning / destructive from stale-snap and low-free rules
already in the product. No extra glyph, no second color.

---

## 7. The apsis strip

### Copy

Time column:

```
last   12h ago
next   in 6h (daily)
```

Disk column:

```
sda1   372G / 596G
disk   [████████░░░░] 62%
       224G free
```

Low space, still copy:

```
disk   28G free   (under 10%)
```

Missing device:

```
disk   not mounted
```

No schedules enabled:

```
next   manual only
```

`--window` only, muted, hide in popup:

```
this snap ~1.8G   (from last increment)
```

### Timing rules

- `last` = age of the newest snapshot. Same clock as today’s tooltip.
  Format `Xm` / `Xh` / `Xd ago`. No wall-clock in the strip.
- `next` = soonest **enabled** schedule tag in this order only:
  `boot → hourly → daily → weekly → monthly`
  Copy `in 6h (daily)`.
- No calendar in the strip. Schedule room owns policy.
- Time and disk fail independently. A missing disk does not blank `next`.

### Disk rules

- Always the **backup device**, never `/`.
- Bar = accent fill on a dim track (same widget as activity).
- No “healthy” color.
- Human sizes: `G` / `T`; one decimal if value < 10, else integer.
- Warning if free < 10% of total. Destructive if free < 5%.
- Stale snap: last age > configured N days (existing reminder).

### Interaction

The strip is not a navigation target. Click focuses snapshots.
`s` opens schedule. `c` opens create.

---

## 8. Activity pane (progress + time left)

This is where jobs speak. Not the list rows. Not details.

| State | Copy |
| --- | --- |
| Idle | `idle` |
| Running | `creating snapshot · 42% · 3m 12s left` |
| Running, no total | `working · 1m 08s elapsed` + indeterminate accent pulse |
| Done | `created <snapshot-name>` then return to `idle` |
| Failed | `create failed · open log` |

Optional extra on the running line if it fits: `2.1G / 3.2G`.

Rules:

- Percent only from helper progress (Timeshift `%` or rsync
  `--info=progress2`). Never fake 0–100.
- ETA from the same stream; if unknown, show elapsed only.
- While busy, apsis **disk** used may tick; `last` / `next` stay frozen
  until list refresh after `Finished`.
- One bar widget shared with the disk meter (accent on dim track).
- Do not invent cancel in a polish phase unless PLAN already has it.

---

## 9. Snapshot list and details

List columns, keep them boring:

```
DATE              TAG  COMMENT
2026-09-29 06:12  D    pre-nvidia
```

- One selected row, accent-dim fill
- Tags as letters only
- No size-OK column, no status lights
- Delete is `[d]elete` plus existing y/N confirm — not a red button
- Mouse: click select, double-click reserved (browse/details),
  wheel scroll, right-click only if a menu already exists
- Keys: `j k` / arrows, `g G` or Home/End, `/` filter if it exists,
  `c` create, `d` delete, `r` refresh, `s` settings, `?` help, `Esc`

Details pane: label in accent, value in text.

```
path
type
size
comment
```

Nothing else required for polish.

---

## 10. Create / schedule / log (when those rooms are open)

### Create

- Comment field
- Optional tag chips as **text toggles**, not colored pills
- One primary action: create
- Dry-run / plan if native backend already shows one
- List remains visible in `--window`
- Progress goes to **activity**, not a modal that hides the list

### Schedule

- Enable + keep N for boot / hourly / daily / weekly / monthly
- Writing settings uses the existing helper path
- Changing schedule updates `next` on the apsis strip after save

### Log

- Monospace helper / rsync / timeshift output
- Last error at the top or clearly marked with text, not a red block
- Can be the fourth room or an expansion of activity later; do not
  duplicate both unless PLAN says so

---

## 11. Mouse and keyboard (keep both)

Keyboard remains first-class. Every click target has a key.

| Input | Result |
| --- | --- |
| Click row | select |
| Double-click row | existing primary open (details / browse) |
| Click tab / dock cell | switch room |
| Click boxed `[c]reate` | same as `c` |
| Wheel | scroll focused pane |
| `1 2 3 4` | snapshots / create / schedule / log |
| `Tab` / `Shift+Tab` | cycle focus among panes |

Do not add gestures that have no keyboard twin.

> **Update 2026-09-29:** in the panel popup only these apply: `Enter` (and a click on the title
> or a row) opens the window, `Esc` closes. Click on a popup row opens the window with that
> snapshot selected; its keyboard twin is `Enter` (opens the window on the newest snapshot).
> The table above is the window's.

---

## 12. Architecture (do not rip up)

```
crates/apsis-core     models, Backend, status math, no libcosmic
crates/apsis          libcosmic applet + window
apsis-helper          polkit + D-Bus + one lock + journald
```

UI never runs as root. Helper is the only privileged process.

For this prompt’s first implementation slice, add to **apsis-core**:

```rust
pub struct ApsisStatus {
    pub last: Option<Duration>,
    pub next: Option<NextSnapshot>,
    pub disk: DiskStatus,
}

pub struct NextSnapshot {
    pub eta: Duration,
    pub tag: Tag, // Boot, Hourly, Daily, Weekly, Monthly
}

pub enum DiskStatus {
    Mounted { device: String, used: u64, total: u64 },
    NotMounted,
    Unknown,
}
```

Methods on `ApsisStatus` (unit-tested, no root):

- `label_short() -> "12h · 62%"`
- `used_pct() -> Option<u8>`
- `free_bytes() -> Option<u64>`
- `disk_warning()` / `disk_critical()`
- `stale_snap(threshold: Duration) -> bool`

**One widget** renders panel label, tooltip, popup strip, and window
strip. Do not fork four layouts.

Settings source of truth stays whatever PLAN currently says. Do not
migrate off Timeshift JSON in a UI-only phase.

---

## 13. Phases this prompt is for

Work only the phase named in the kickoff. Do not jump ahead.

Suggested order (align with `docs/PLAN.md`; do not invent ids if
PLAN already numbers them):

1. **Status model** — `ApsisStatus` + tests + panel label + tooltip
2. **Strip in list view** — window + popup render of time | disk
3. **Activity bar** — % + ETA / elapsed in the existing activity pane
4. **Layout D chrome** — four rooms dock in `--window` only; popup
   stays compact
5. **Create beside list** — `--window` does not replace snapshots
   while creating
6. **Copy pass** — empty / not-mounted / manual-only / low-disk lines
7. **Theme audit** — zero hardcoded colors; warning/destructive only
   on icon or the affected phrase
8. **Undock** (optional, late) — real Cosmic window for details or log

Restore, native-default backend, Timeshift-optional first-run are
**product phases**, not UI polish. They have their own briefs. This
file still constrains how those phases may look.

---

## 14. Definition of done (UI polish)

A phase is done when:

- The UI still looks like the current Apsis screenshot family
- Accent + text only, live Cosmic theme
- Panel shows `12h · 62%` (or `—` sides)
- Tooltip shows last / next / disk
- `--window` has apsis above the split and activity below it
- Busy create shows `%` and time left in activity
- No new crates, no restore work, no sudo from the agent
- `cargo test --workspace`, clippy `-D warnings`, `cargo fmt`
- `docs/UI.md` updated; `docs/DECISIONS.md` appended

---

## 15. Explicit do-not list

- Do not rebrand layouts A/B/C as the default
- Do not implement an inner tiling WM
- Do not require Timeshift for rendering status (if list + disk
  info can come from native / helper, use that)
- Do not show root filesystem usage
- Do not one-click delete or create without existing confirms
- Do not add full-system restore to the dock
- Do not write `/etc/timeshift/timeshift.json` from a UI-only slice
- Do not run `just deb-install` or any root command; print it for
  the user

---

## 16. Voice and copy

Lowercase pane titles. Short words.

Good: `last  12h ago` · `disk  not mounted` · `create failed · open log`
Bad: `Last Snapshot Age` · `Backup Target Utilization` · `ERROR!!!`

Key hints use the existing bracket style: `[c]reate [d]elete`.

---

## 17. When a future phase conflicts with this file

Append the conflict to `docs/DECISIONS.md` with date and why.
Do not silently rewrite this prompt mid-phase.
If Cosmic theme tokens cannot express something, stop and ask;
do not invent a third color.
```
