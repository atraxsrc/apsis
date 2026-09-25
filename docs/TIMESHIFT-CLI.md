# Timeshift CLI contract

Verified 2026-09-25 against `timeshift --help` and real output from **Timeshift v24.01.1** (rsync mode)
on the user's machine. Redacted copies of that output are the fixtures in
`crates/apsis-core/tests/fixtures/`. Re-check this file if Timeshift is upgraded.

## Commands we use

| purpose | argv |
|---|---|
| list | `timeshift --list --scripted [--snapshot-device <dev>]` |
| create | `timeshift --create --comments <text> --scripted [--snapshot-device <dev>]` |
| delete | `timeshift --delete --snapshot <name> --scripted [--snapshot-device <dev>]` |
| devices | `timeshift --list-devices --scripted` |
| check config | `timeshift --check --scripted` - creates a scheduled snapshot if one is due, so **don't** use it for probing |

- **No `--tags` on create.** Help says `--tags {O,B,H,D,W,M}` with default `O`, but on v24.01.1
  `timeshift --create --tags O` fails with `Unknown value specified for option --tags (O)`
  (reported by the user). Omitting `--tags` gives an on-demand (`O`) snapshot. Other tags (e.g. the
  help's own example `--tags D`) are untested and not needed.
- **`--snapshot-device`**: passed whenever the device is known from a previous `--list`. Apsis passes
  the **UUID** (help note 4: a UUID can be given instead of a device name), and falls back to the
  device path only when the list had no UUID.
- `--snapshot <name>` is listed under "Restore" in `--help`, but the help's own example uses it with
  `--delete` (`timeshift --delete --snapshot '2014-10-12_16-29-08'`).
- Every command needs root. Timeshift refuses to run otherwise.

## All options (v24.01.1 `--help`)

| group | option | meaning |
|---|---|---|
| list | `--list[-snapshots]` | list snapshots |
| list | `--list-devices` | list devices |
| backup | `--check` | create snapshot if scheduled |
| backup | `--create` | create snapshot (even if not scheduled) |
| backup | `--comments <string>` | set snapshot description |
| backup | `--tags {O,B,H,D,W,M}` | add tags (default `O`; `O` itself is rejected, see above) |
| restore | `--restore` | restore snapshot (interactive without other options) |
| restore | `--snapshot <name>` | snapshot to restore (also used by `--delete`) |
| restore | `--target[-device] <device>` | target device |
| restore | `--grub[-device] <device>` | device for installing GRUB2 |
| restore | `--skip-grub` | skip GRUB2 reinstall |
| delete | `--delete` | delete snapshot |
| delete | `--delete-all` | delete all snapshots |
| global | `--snapshot-device <device>` | backup device (default: config); UUID also accepted |
| global | `--yes` | answer YES to all prompts |
| global | `--btrfs` / `--rsync` | switch mode (default: config) |
| global | `--debug` | extra debug messages |
| global | `--verbose` / `--quiet` | show (default) / hide rsync output |
| global | `--scripted` | non-interactive mode |
| global | `--help`, `--version` | |

Tags: `O` on-demand, `B` boot, `H` hourly, `D` daily, `W` weekly, `M` monthly.

## Not in scope until Phase 6

`--restore`, `--target[-device]`, `--grub[-device]`, `--skip-grub`, `--delete-all`.

## `--list` output

Configured, rsync (spacing is significant only as "whitespace"; lines carry trailing padding):

```
Mounted '/dev/sdX1' at '/run/timeshift/99999/backup'
Device : /dev/sdX1
UUID   : 00000000-0000-0000-0000-000000000000
Path   : /run/timeshift/99999/backup
Mode   : RSYNC
Status : OK
5 snapshots, 123.4 GB free

Num     Name                 Tags  Description
------------------------------------------------------------------------------
0    >  2026-09-19_09-29-57  O
...
4    >  2026-09-25_11-28-53  O     apsis test: comment with spaces
```

Not configured:

```
Device : Not Selected


** (process:99999): CRITICAL **: 11:24:41.486: gee_abstract_collection_get_size: assertion 'self != NULL' failed
No snapshots found
```

Parsing rules:

- The `Mounted '...' at '...'` line is optional; the mount path contains a PID and is not stable.
- Header lines are `Key<spaces>: value`. Known keys: `Device`, `UUID`, `Path`, `Mode`, `Status`.
  `Device : Not Selected` means no snapshot device is configured.
- `N snapshots, X GB free` is informational; the row count is what counts.
- The table starts after the `Num  Name  Tags  Description` header and the `-----` rule.
- Row: `<num> > <name> <tags> [<description>]`. `name` matches `YYYY-MM-DD_HH-MM-SS`; tags is one
  token of letters from `OBHDWM`; the description is the rest of the line, trimmed. It may contain
  spaces and colons. Rows and the header carry trailing padding.
- Rows may list several tags (e.g. `BD`); not yet seen in real output, so treat it defensively.
- GLib `** (process:N): CRITICAL **` lines can appear anywhere; ignore them.
- Timeshift's own `E: ...` / `W: ...` lines can appear anywhere, including after the table. They
  are collected as warnings and shown in the activity pane; they don't fail the list (see
  "Errors and warnings" below). A line inside the table that isn't a snapshot row is a warning
  too (`line N: not a snapshot row: ...`).
- A list fails only when Timeshift exits non-zero, or the output has neither a table nor
  `No snapshots found`.
- `No snapshots found` means an empty list.
- `--scripted` does not change the `--list` format: the user diffed `--list` against
  `--list --scripted` and only the mount PID in `/run/timeshift/<pid>/backup` differed.

## Errors and warnings

Found 2026-09-25 when the user's USB backup disk dropped off after a create (Timeshift v24.01.1).

- **Timeshift prints `E:` / `W:` lines on stdout**, not stderr. A failed run can have an empty
  stderr, so before this Apsis showed only `timeshift exited with code 1`. Apsis now takes the
  `E:`/`W:` lines from stdout plus stderr's lines, keeps the last 5 (`MAX_OUTPUT_LINES`), and
  shows those. If there are none, it shows stdout's last lines.
- **Disk missing** (`sudo timeshift --list` by the user; the disk had dropped off):

  ```
  E: Device not found: '/dev/sdX1'
  E: Failed to remove directory
  Ret=256
  ```

  Exit code non-zero (Apsis saw 1). Apsis turns `E: Device not found: '<device>'` into
  `Error::DeviceNotFound` and shows `backup disk not connected (UUID 1a2b…): plug it in and press
  r`, naming the disk by the UUID from the last good list, since the `/dev` name can differ after
  a reconnect. Fixture `list-device-not-found.txt` is rebuilt from those three quoted lines (their
  order and stream weren't captured separately). Apsis also recognises the message if Timeshift
  exits 0 with it and no table.
- **Stale mount** (after the disk was plugged back in): the list is complete and exits 0, but ends
  with an extra line after the table. It's probably the old `/run/timeshift/<pid>/backup` mount
  that couldn't be removed:

  ```
  ...
  4    >  2026-09-25_11-28-53  O     apsis test: comment with spaces

  E: Failed to remove directory
  ```

  Before this, the parser failed on it (`line 17: unexpected snapshot row`). Now it's a warning.
  Fixture: `list-rsync-stale-mount.txt` (the redacted device fixture plus that line).
- `Ret=256` appears to be Timeshift's internal status (256 = exit status 1 in `wait()` terms). Apsis
  ignores it.
- `--snapshot-device`: Apsis passes the UUID whenever a list has shown one, and keeps it when a
  later list fails. The `/dev/sdX1` in the message above came from Timeshift's own configuration
  (the user ran it without `--snapshot-device`).

## Other notes

- Config lives at `/etc/timeshift/timeshift.json` (root-owned). Apsis reads nothing from it in
  Phases 1-3.
- Known COSMIC issues with the Timeshift GTK UI (e.g. pop-os/cosmic-epoch#485, #1558) are part of why
  Apsis exists; the CLI is unaffected.
