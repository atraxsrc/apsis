# Plan

One phase at a time. Each phase ends with: `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt`,
a short summary to the user, and an entry in `DECISIONS.md` if anything was decided.

## Phase 0 — Scaffold

1. Ask the user for the **App ID** and **license** (default suggestion: GPL-3.0-only, like most
   COSMIC apps). Record both in `DECISIONS.md`.
2. `cargo generate gh:pop-os/cosmic-applet-template` into `./.scratch/` (gitignored), with name `apsis`
   and the App ID. Read what it produced.
3. Build the workspace from it:
   - root `Cargo.toml` as a workspace (`members = ["crates/*"]`, `resolver = "2"`, shared
     `[workspace.package]` and `[workspace.dependencies]`, keep the template's libcosmic git dep and
     features).
   - `crates/apsis/` ← the template's `src/`, `Cargo.toml` (binary name `apsis`).
   - `resources/`, `i18n/`, `justfile` at the root; fix paths in the justfile for the workspace.
   - `crates/apsis-core/` — empty lib crate.
4. `.scratch/` stays out of git. Nothing from the template's git history comes along.
5. **Done when:** `cargo check --workspace` passes and `just run` opens the template applet.

## Phase 1 — Core model + Timeshift parser (no UI, no root)

In `apsis-core`:
- `Snapshot { name, created: time, tags: Vec<Tag>, comment: Option<String> }`, `Tag` = O/B/H/D/W/M.
- `trait Backend` (see ARCHITECTURE.md) and `TimeshiftCli` implementing it by *building* commands.
- `parse_list(output: &str) -> Result<SnapshotList>` for `timeshift --list` output.
- Ask the user to run `sudo timeshift --list` and paste the output. Redact it (see CLAUDE.md), save as
  `tests/fixtures/list-*.txt`, include at least: an empty list, several snapshots, and comments with spaces.
- Unit tests for the parser and for command building (exact argv, no shell strings).
- **Done when:** tests pass on fixtures; nothing executes timeshift yet.

## Phase 2 — Applet UI (read-only)

- Panel button with a symbolic icon; tooltip "last snapshot: 3h ago".
- Popup = terminal-style list (see UI.md), keyboard + mouse, theme-driven.
- Listing runs `pkexec timeshift --list --scripted` via `apsis-core` on a background task.
- States: loading, empty, error (show stderr tail), list.
- **Done when:** the user can open the popup and see their real snapshots with `just run`.

## Phase 3 — Create and delete

- `[c]reate`: inline prompt for a comment → `timeshift --create --comments <c> --scripted` (no `--tags`: v24.01.1 rejects `O`, and O is the default; see TIMESHIFT-CLI.md).
- `[d]elete`: inline confirm (type `y`) → `timeshift --delete --snapshot <name> --scripted`.
- Both pass `--snapshot-device <uuid>` from the last list, and refuse to run if no list has shown
  a device.
- Progress line while running; refresh list after.
- The user tests these manually. Claude never triggers them.
- **Done when:** create/delete work end-to-end on the user's machine.

## Phase 3.5 - superfile-style layout

Look and feel only; the terminal style, keys and theme rules stay. Visual reference:
[superfile](https://github.com/yorukot/superfile) (looked at, no code copied).

- Rounded bordered panels with the title set into the top border: `snapshots`, `details`,
  `activity`.
- Details pane for the selected snapshot: full name, age, tags spelled out, full comment.
- Activity pane: the current or last operation and its result. Create/delete progress moves here
  from the `>` line.
- Footer row of key hints. The popup may grow to ~720 px wide.
- Theme colours only: borders use the theme's divider/accent colours.
- **Done when:** the three panes and the footer follow the live theme, light and dark, and every
  Phase 3 action still works from keys and mouse.

## Phase 4 — Privileged helper (removes repeated password prompts)

- `crates/apsis-helper`: small system D-Bus service (zbus) run as root on demand.
- Methods: `List`, `Create(comment)`, `Delete(name)`; each checks a polkit action
  (`list` → allowed for active local users; `create`/`delete` → `auth_admin_keep`).
- Ship `resources/` files: D-Bus system service + policy, polkit `.policy`, systemd unit.
- Applet talks to the helper; `pkexec` path kept as fallback.

## Phase 4.5 - Settings

Edit Timeshift's own config from Apsis, so `timeshift-launcher` and Apsis stay interchangeable.

- `apsis-helper` reads and writes `/etc/timeshift/timeshift.json` (root-owned). Keep Timeshift's
  exact JSON structure and field names; never invent fields, and keep fields Apsis doesn't edit
  as they were.
- Exposed settings:
  - backup device: a picker from `timeshift --list-devices`
  - mode: rsync / btrfs (read-only when Btrfs isn't available on this system)
  - include `/home` toggle
  - include/exclude filters: list, add, remove
  - schedule and retention counts: hourly, daily, weekly, monthly, boot
- polkit: a new action `io.github.atraxsrc.Apsis.configure`, `auth_admin_keep` like create/delete.
- Validate before writing: the device must exist, filters must be plausible paths, retention
  counts are non-negative integers. Never write a file that would make Timeshift refuse to start.
- The helper writes atomically (temp file, fsync, rename) and keeps one backup of the previous
  config, `timeshift.json.bak`.
- UI: a Settings view in the popup and window, reached from the right-click menu or a key, in the
  same style (theme colours, monospace, keyboard and mouse).
- **Done when:** a setting changed in Apsis shows up in Timeshift's own settings window (and the
  other way round), and Timeshift still starts and lists.

## Phase 5 — Native backend (optional, after Timeshift path is solid)

v1, rsync only (split from the original Phase 5 at the user's request):

- rsync + hardlinks (`--link-dest`) for ext4 (Pop!_OS default), in `apsis-core::native`, behind
  the same `Backend` trait as `TimeshiftCli`. Timeshift's on-disk layout exactly (folders,
  `info.json`, `exclude.list`, `rsync-log`, tag links), read from Timeshift's source, so either
  tool reads, uses and deletes the other's snapshots.
- list (reads `info.json`, no `timeshift`), create (rsync `--link-dest` against the newest
  snapshot of this system, then `info.json`), delete (the folder; only for the tests, the
  applet still deletes through Timeshift).
- Opt-in: a per-user Apsis setting in the settings view, off by default. Dry run (on by
  default) shows the rsync argv and `info.json` and writes nothing.
- Runs in `apsis-helper` (root): `NativeList`, `NativeDryRun`, `NativeCreate`.
- Tests on a loop-mounted ext4 image under `target/` (mounted by the user), not the real disk.
- **Done when:** the tests pass on the ext4 image, the user has reviewed a dry run on their
  machine, and a native snapshot shows up in `timeshift --list`.
- **Status: done 2026-09-26** (see DECISIONS.md). Left over from v1:
  - native retention (Timeshift applies it on its next run until then) - rescoped in 5.2 to
    "keep last N manual", 2026-09-28
  - ~~idle I/O priority for the native rsync (Timeshift 24.01.1 doesn't use it either)~~ -
    done in 5.2, 2026-09-28
  - Btrfs: Phase 5.1 below
  - ~~recognise Timeshift's `Ret=NNN` lines as diagnostics (they're ignored now)~~ - done
    2026-09-27, see Polish

## Phase 5.1 - Native btrfs

- btrfs: read-only subvolume snapshots of `@` / `@home` directly.

## Phase 5.2 - Manual retention, reminder, low-priority native rsync (rescoped)

**Rescoped 2026-09-28 by the user:** no native scheduler. Scheduled snapshots stay with
Timeshift's own cron for both backends, and the settings view keeps configuring it. Reason:
users mostly snapshot by hand, and Timeshift already schedules (and applies retention to
scheduled snapshots) well; a second scheduler would add a lot of machinery (timer, owner
switching, uninstall handover) for little gain. The research for it is kept below as a
reference.

- **Keep last N manual snapshots** (Apsis setting, off by default): `apsis_core::retention::
  manual` decides; `p` (and, after a create, by itself) shows a preview and deletes only after
  `y`, through the bulk delete. Commented ones pinned (kept, not counted towards N; changed 2026-09-29), other-tag ones left to Timeshift, never the
  newest. Both backends. See UI.md "Keep last N manual snapshots".
- **Reminder**: tooltip and a warning-coloured panel icon when the newest snapshot is older than
  N days (default 7, 0 = off), with a background list through the helper only. See UI.md
  "Reminder".
- **Native create at low priority**: rsync under `ionice -c 3 nice -n <to 19>`.
- Tests: retention edge cases (unit), the priority of the child processes, applet flows; native
  tests with low priority on temp dirs and (`just test-ext4`) the ext4 image.
- **Status: built 2026-09-28, committed 2026-09-29 (`90272f5`).** `just test-ext4` passes on the
  user's machine (native 14/14, restore 11/11). Waiting for the user's `just deb-install` test
  of the keep-last-N preview/confirm and the reminder.

### Reference: how Timeshift 24.01.1 schedules and applies retention (not built)

`.scratch/timeshift` at tag 24.01.1.

- A scheduled run (`timeshift --check`, `Main.vala:972-1218`) happens only if a `schedule_*`
  is on and it isn't a live session (`:946-953`). Levels are due, in order boot, hourly,
  daily, weekly, monthly, when the newest snapshot of this system with that tag
  (`SnapshotRepo.vala:390-402`) is older than system start (boot), or than `now - 1 h / 1 d /
  1 w / 1 calendar month + 1 min`.
- `create_snapshot_for_tag` (`Main.vala:1220-1271`): if any snapshot (any tag, any system) is
  newer than system start (boot) or `now - 1 h + 59 s`, it only gets the tag; else a new
  snapshot is taken. That's how one snapshot gets several tags.
- `auto_remove` (`SnapshotRepo.vala:615-713`) runs after every console `--check` and
  `--create` (`Main.vala:1201-1204`), over valid snapshots of all systems, oldest first: boot
  trimmed by count only (comments don't protect it, `:625-634`); hourly..monthly untag an
  uncommented snapshot older than N periods while more than N have the tag (`:638-688`); then
  every tagless snapshot is deleted (`remove_untagged`, `:753-772`). No "keep newest" rule.
- Every Timeshift run, even `--list`, rewrites `/etc/cron.d/timeshift-hourly` and
  `timeshift-boot` from `timeshift.json`'s flags on exit (`Main.vala:4323`,
  `cron_job_update` `:4208-4259`), so a second scheduler would have to switch those flags, not
  delete the files.

## Phase 6a - File-level restore

**Removed in 0.4.0** (owner's product decision, 2026-09-30; see "0.4.0" below and DECISIONS.md).
Kept here as history.

Open a snapshot, browse its files, copy chosen files and folders back. The same for Timeshift-made
and Apsis-native rsync snapshots (both `timeshift/snapshots/<name>/localhost/...`). Full-system
restore is 6b: nothing of it is built here. btrfs mode is refused (different layout; btrfs is
5.1), and so are encrypted backup devices, as in the native backend.

### Helper methods (apsis-helper, root)

| method | polkit | |
|---|---|---|
| `Browse(s snapshot, s path) -> (a(sstxuuussstx)b)` | `browse` | one folder: `[(name, kind, size, mtime, mode, uid, gid, owner, link target, live, live size, live mtime)]`, `truncated` |
| `Restore(s snapshot, as paths, s mode, b dry_run)` | dry run: `browse`; folder: `restore`; original: `restore-original` | returns once started; `Finished("restore", ok, text)` follows (text = the plan, or the result) |

- `kind`: `file`, `dir`, `link`, `other` (devices, fifos, sockets: listed, never restored).
  `owner`: `user:group` names from the live system, numeric if unknown. `live` compares the
  entry with the same path on the running system, never following symlinks: `missing`, `same`,
  `changed` (other type, size or mtime; for a link, other target), `present` for folders (not
  compared recursively).
- At most 10,000 entries per folder; `truncated` says there were more.
- `mode`: `folder` (default) or `original`.
- New polkit actions (amendment C): `io.github.atraxsrc.Apsis.browse`, `auth_admin_keep`
  (snapshots hold root-only files such as `/etc/shadow`; dry runs use it too);
  `io.github.atraxsrc.Apsis.restore` (folder mode), `auth_admin_keep`;
  `io.github.atraxsrc.Apsis.restore-original` (original mode), `auth_admin`, asked every time.
- Both take the single-operation lock: they mount at the same place as a native create, so a
  browse is refused while a create/delete/restore runs, and the other way round. A real
  restore is also refused while Timeshift holds its lock.
- Journal, one line per call: `restore folder 2026-09-25_03-00-01 ["/etc/fstab", "/etc/hosts",
  +3 more] for :1.42 (uid 1000): done` (paths cut to 60 characters, at most 3 shown).

### Mount

The backup device from Timeshift's settings, as `NativeList` does it (`native::open` split into
"find and mount the device" and "native config"): `mount -o ro,nosuid,nodev,noexec
/dev/disk/by-uuid/<uuid> /run/apsis/backup` for the length of one call, unmounted after. The
source is read-only and held under the lock, so it can't change between the checks and rsync.

### Path rules (`apsis_core::restore`, no root, unit-tested)

- The snapshot name matches `YYYY-MM-DD_HH-MM-SS`, and `snapshots/<name>/` and its
  `localhost/` are real folders (not symlinks).
- A path is absolute, snapshot-relative (`/etc/fstab`), not `/` itself, no NUL, no empty, `.`
  or `..` components. Checked lexically first. D-Bus strings are UTF-8, so a name that isn't
  is listed with `�` and can only be restored with its folder.
- Resolved from `localhost/` one component at a time with `lstat`: every component before the
  last must be a real folder. A symlink anywhere before the last component is refused, never
  followed.
- The last component may be a symlink: it's copied as a symlink (link text unchanged). It's
  refused if its target, read lexically from the link's folder with the snapshot's
  `localhost/` as `/`, climbs above that root (`../../../../x`). Absolute targets
  (`/usr/bin/vi`) stay inside by that rule and are kept as they are.
- Symlinks inside a selected folder are copied as links by rsync (no `-L`, `-K`, `--copy-*`),
  never followed, so they can't read anything outside the snapshot.

### Destination modes

**folder** (default): `~/Apsis-restored/<snapshot>/<original path>` of the calling user.

- uid from the bus (`GetConnectionUnixUser` on the caller), home and gid from `/etc/passwd`.
  Refused if the home is missing, not a folder, or not owned by that uid.
- Root never writes through a path the user could swap for a symlink (amendment B): every
  component of the home is opened from `/` with `openat(O_NOFOLLOW | O_DIRECTORY)`, each
  relative to the one before; the home must be owned by the caller. `Apsis-restored/` is
  opened the same way (made and given to the caller if missing) and must be owned by the
  caller. A new `<snapshot>/` is made in it with `mkdirat`, opened with `O_NOFOLLOW`, must be
  empty and root's, mode 0700 while the copy runs.
- rsync writes through the held descriptor: it's made inheritable (`FD_CLOEXEC` cleared) just
  for the rsync run, and rsync's destination is `/proc/self/fd/<n>/`. In rsync's process that
  magic link resolves to its inherited copy of the descriptor, the folder made above, wherever
  it has moved since; only root can enter it. Tested by swapping `Apsis-restored/` for a
  symlink right before rsync starts.
- Hand-over never follows a symlink: rsync's `--chown` sets the contents' owner as it copies,
  then one `fchown` and `fchmod 0755` on the top folder's descriptor. Tested with a snapshot
  symlink to a file outside the destination: its owner, mode and mtime stay as they were.
- Never overwrites: if `<snapshot>/` exists (an earlier restore), the new one is
  `<snapshot>-2/`, `-3/`, ... Plus `--ignore-existing`.
- Files are chowned to the user (`--chown=<uid>:<gid>`), setuid/setgid bits are dropped
  (`--chmod=ug-s`), `security.*` xattrs (file capabilities, SELinux labels) aren't copied, and
  no device nodes, FIFOs or sockets are made (`--no-devices --no-specials`, amendment A), so no
  user-owned copy carries root's privileges. Tested with a FIFO in the snapshot.
- One rsync for all paths, with `--relative` (`localhost/./etc/fstab`), so the original path
  is rebuilt under the folder.

**original**: back where they were, on the live system.

- Refused under `/proc`, `/sys`, `/dev`, `/run`, `/tmp`, `/boot`, for `/` itself, and for any
  path whose live location is on the backup device's filesystem (same `st_dev` as the mount).
  Warned, but allowed: `/etc`, `/usr`.
- The live parent folder must exist, and every component of it must be a real folder (no
  symlink, checked with `lstat`, e.g. a live `/var/run` link). Else refused, with the reason in
  the plan: mark the missing parent instead.
- Before replacing anything, rsync moves the current file to `<name>.apsis-before-<snapshot>`
  next to it (`--backup --suffix=.apsis-before-<snapshot>`). Only items rsync changes are
  backed up; identical ones are skipped. If a backup name already exists (a second restore of
  a file that changed again), the restore is refused, since rsync would overwrite that backup.
- One rsync per path, into its live parent (no `--relative`, so `/etc` itself keeps its owner,
  mode and mtime).
- The UI asks for `y` before running it.

### rsync

Argv, no shell, fixed `PATH`, cleared environment, `LC_ALL=C.UTF-8`, stdin null:

```
folder:   rsync -aHAX --numeric-ids '--out-format=APSIS %i %l %n%L' --relative \
            --ignore-existing --no-devices --no-specials --chmod=ug-s \
            '--filter=-x security.*' --chown=<uid>:<gid> [--dry-run] \
            /run/apsis/backup/timeshift/snapshots/<s>/localhost/./<path>... /proc/self/fd/<n>/
original: rsync -aHAX --numeric-ids '--out-format=APSIS %i %l %n%L' --backup \
            --suffix=.apsis-before-<s> [--dry-run] \
            /run/apsis/backup/timeshift/snapshots/<s>/localhost/<path> <live parent>/
```

(`-x <pattern>` is rsync's xattr filter rule; checked with rsync 3.2.7 that it drops the named
xattrs. `--out-format` is `--itemize-changes` plus the size, marked so the plan's lines are
told from rsync's other messages. A folder-mode dry run writes to `/run/apsis/restore-dry-run/`,
which doesn't exist and isn't made. No `--delete`, ever.) Exit 0 is success; 23/24 (some files unreadable or
vanished) are reported as "finished with errors" with rsync's last lines, like any other code.

### Dry run and plan

Dry run is on by default: the same argv with `--dry-run`. Its `--itemize-changes` lines give
the plan, shown like the native create plan:

```
restore from 2026-09-25_03-00-01, folder mode
to /home/<user>/Apsis-restored/2026-09-25_03-00-01
  create   /etc/NetworkManager/
  copy     /etc/NetworkManager/NetworkManager.conf   1.2 KiB
  replace  /etc/hosts   -> backup /etc/hosts.apsis-before-2026-09-25_03-00-01   (original)
2 files, 1 folder, 1.5 KiB; 1 replaced, 1 backed up
warning: /etc is live system configuration      (original)
rsync -aHAX ... (each argv)
```

Cut at 1,000 item lines (`... and 4,312 more`); the totals count everything. Items rsync
skips because they're already the same aren't listed (rsync doesn't print them). The user runs it
for real with a second key press; rsync decides again then, and backups cover anything that
changed in between.

### UI

- Snapshot list: Enter (or double-click) opens the browser in the left pane at `/`. Title is
  the breadcrumb: `2026-09-25_03-00-01:/etc/NetworkManager` (cut from the left with `…`),
  plus `· 3 marked`.
- Rows: marker, then `+` missing, `~` changed, `=` same/present (theme colours: success-ish
  for same, warning for changed, accent for missing), mark `*`, name, `/` after folders,
  `-> target` for links, size. j/k/↑/↓ move, Enter/l into a folder, Backspace/h up, space marks
  (marks are kept across folders), `R` restore, `r` reload the folder, Esc back to the list.
- Details pane: path, type, size, mtime, mode (`-rw-r--r--`), owner, link target, and the live
  comparison (`live: changed - size 1.2 KiB -> 980 B, mtime 2026-09-20 -> 2026-09-26`).
- `R`: `> restore 3 items to [f]older (~/Apsis-restored) or [o]riginal? _` → the dry run
  (activity `restore dry run… ⠹`) → the plan in the left pane (title `restore plan`) → Enter
  runs it (folder); original asks `> put 3 items back over the live system? [y/N] _` first.
  Activity `restoring… ⠹`, then the result. Esc at any step goes back to the browser.
- Details focus, which Enter had: now Tab.
- Needs `apsis-helper` (no pkexec path): `browsing needs apsis-helper (sudo just install)`.
- Same in the panel popup and the window. Theme colours only.

### Tests

- `apsis-core` (no root): path validation (`..`, `.`, empty, NUL, relative, `/`), resolution
  on temp trees (symlinked folder in the middle refused, final symlink kept, relative
  escape refused, absolute target kept), browse entries and live comparison, plan parsing from
  real `--itemize-changes` output, the refused/warned list, argv building, backup-name
  collisions.
- Real rsync on temp folders under `target/tmp/restore/`, and on the ext4 image when
  `APSIS_EXT4_MNT` is set (`just test-ext4`): folder mode (fresh folder, `-2` suffix, nothing
  overwritten, symlinks copied as links, setuid dropped), original mode against a fake live
  root (backup made, identical files skipped, collision refused), dry run writes nothing.
  Ownership tests use the tester's own uid (no root).
- Helper: interface/method names, polkit names in the policy file, journal line format.

### Files

No new installed files. Changed: `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy`
gets the three actions (`browse`, `restore`, `restore-original`). The bus policy already allows the whole interface;
only its comment changes.

**Done when:** on the user's machine a file restored in folder mode and one in original mode
(from a Timeshift snapshot and a native one) match the snapshot, the backup is next to the
original, and the journal shows each call.
- **Status: done 2026-09-26.** Tested on the user's machine: ext4 tests pass; Timeshift
  snapshots 2026-09-26_08-18-57 and 2026-09-26_08-33-30 in folder and original mode, native
  snapshot 2026-09-26_07-31-23 in folder mode. Folder mode restores are owned by the user and a
  rerun goes to `<snapshot>-2`; original mode asks the password again, keeps `root:root` and
  writes `.apsis-before-<snapshot>`; refusals and journal lines as specified. Leftovers: see
  Polish below.

## 0.4.1 - Snapshot format: ACLs and extended attributes (before 6b)

**Status: released 2026-10-01. Core done (argv, `info.json` key, format detection, tests); the
owner's four checks on apsis-test passed. The row tooltip moved to 0.5.0 (see UI).**
A small release of its own, so 0.5.0's restore starts from snapshots that hold everything a
restore needs.

**Why:** create runs rsync with `-a` only (Timeshift's flags), so snapshots lose POSIX ACLs and
extended attributes, including file capabilities (`security.capability`, e.g. on `ping`). A
restore can't bring back what the snapshot doesn't have.

- **Create argv** gets `-A -X --numeric-ids` (the rest unchanged: `-aii --recursive --verbose
  --delete --force --stats --sparse --delete-excluded --info=progress2 --link-dest ...`).
  - `-A`: POSIX ACLs (`system.posix_acl_*`), e.g. on `/var/log/journal`.
  - `-X`: extended attributes, including `security.capability`.
  - `--numeric-ids`: owners by number, never mapped through a user database. It changes
    nothing on a local copy on the same system, but it's explicit, and it's what a recovery
    from a live USB (a different `/etc/passwd`) needs. The restore uses it too.
  - **`-H` stays off.** Timeshift doesn't use it. With `--link-dest`, the cost is a
    table of every multiply-linked file on each run (memory and time on a full system), for a
    small gain: a handful of files in `/usr` that are hard links to each other come back as
    separate copies with the same content. dpkg and the programs don't care. It can be added
    later if a real case needs it.
- **`info.json`** gets one more key, written last: `"apsis-rsync-flags" : "-aAX --numeric-ids"`
  (a plain string). Missing (Timeshift's snapshots, Apsis 0.4.0 and older) means the old
  format. The byte-exact writer test changes to expect the key at the end. Check 3 confirms that
  Timeshift 24.01.1 still lists and restores a snapshot with the extra key; json-glib reads
  members by name, so an extra one should be ignored.
- **`--link-dest` and the change:** rsync only hard-links a file to the previous snapshot when
  its ACLs and xattrs match too. The first new-format snapshot copies again the few files that
  have ACLs or xattrs. Every later one links as before. CHANGELOG line.
- **Backup filesystem:** the selectable ones (ext4, xfs, btrfs, ...) all store ACLs and
  xattrs. If one doesn't, rsync fails with "Operation not supported" and the create fails as
  now. No new check.
- **`/boot/efi` keeps being copied** on create, and is **not** added to any create exclude.
  A Timeshift restore runs rsync `--delete` without `-x`, so a snapshot with an empty
  `boot/efi/` would wipe the live ESP if someone restores it with Timeshift. Only Apsis's
  restore skips `boot/efi` (6b.2). The same goes for anything else Timeshift itself would
  expect to find in a snapshot.
- **UI:** old-format snapshots are marked in the Restore dialog (the main place) and in the
  list row's tooltip, "Older format: made without ACLs and extended attributes". No glyph or
  column, no details area (owner, 2026-09-30; 6b.8).
  **Both come with 0.5.0, none in 0.4.1** (owner, 2026-10-01): `List` sends a snapshot as
  `(sss)` and a 0.4.0 panel checks every field, so carrying the format in the list is a
  breaking wire change. How the list carries it is settled in 6b's helper slice (6b.13 step
  3), and the tooltip is built in its UI slice (step 4). 0.4.1 changes no interface.
- **Tests (no root):** argv (exact, with `-A -X --numeric-ids`, without `-H`); `info.json`
  writer and reader with and without the key; format detection; real rsync on temp trees
  keeps a `user.*` xattr and an ACL (both settable without root on the test's own files, where
  the filesystem supports them; the test skips with a message where it doesn't).
- **On apsis-test** (owner runs):
  1. A create with the new flags: rsync exit 0, and `sudo grep -ciE 'not supported|failed'
     <snapshot>/rsync-log` is 0. The vfat ESP under `/boot/efi` has no ACLs or xattrs. The
     check is that rsync says nothing about that.
  2. `sudo getcap <backup>/timeshift/snapshots/<new>/localhost/usr/bin/ping` shows the
     capability; `sudo getfacl` on a file with an ACL matches the live one.
  3. `sudo timeshift --list` shows the new snapshot, and Timeshift's restore dialog opens on
     it (cancel there).
  4. `du -sh` of the second new-format snapshot shows `--link-dest` linking again.

## Phase 6b - Full-system restore (release 0.5.0)

**Status: design approved with changes (owner, 2026-09-30); this version has them. 0.4.1 is
released; the core slice (6b.13 step 1) started 2026-10-01 on branch `restore-6b-core`.**
Core so far: the filter and protect list, home detection (`apsis_core::restore::filter`), the
restore's rsync argv (`apsis_core::restore::argv`), the refusals of 6b.7 except Busy
(`apsis_core::restore::refusal`), the Apsis in a snapshot (`apsis_core::restore::apsis`),
space parsing and the two space refusals (`apsis_core::restore::space`). Format detection is 0.4.1's `Info::is_old_format`, unchanged.

Goal: pick a snapshot, click Restore, and after a restart the system is back to that state.
One person at the keyboard. The copy is rsync over `/` with excludes, like Timeshift's. When
it runs (the next boot), what it skips (the ESP) and the boot refresh and its check are
**Apsis's own design, not Timeshift's behaviour** (DECISIONS.md, 2026-09-30). The README says
so, and never "restores like Timeshift". Marked **experimental** in the README and the dialog.

### 6b.0 Facts this design rests on

- Test machine `apsis-test`: Pop!_OS 24.04, UEFI, systemd-boot + kernelstub, ext4 root on a
  plain partition, cryptswap with a random key, `/recovery` (vfat, Pop's recovery), ESP (vfat)
  at `/boot/efi`.
- kernelstub boots the kernel and initrd from **copies** on the ESP
  (`\EFI\Pop_OS-<root-uuid>\vmlinuz.efi`, `initrd.img`; entries `loader/entries/Pop_OS-current.conf`
  and `Pop_OS-oldkern.conf`). Restoring `/` rolls back `/boot` and `/usr/lib/modules` but not
  the ESP, so the ESP kernel can end up with no matching modules. **This is the central risk.**
- **What snapshots hold under `boot/efi` and `recovery`** (from the code; confirmed on
  apsis-test by check 0.1): the exclude builder gives each non-standard fstab mount point a
  `<mount>/*` line, and `/boot` is a standard prefix (`native/exclude.rs`, `STANDARD_PREFIXES`,
  as in Timeshift). So `/recovery/*` is excluded (the real fixture has it), and the snapshot
  holds only the empty `recovery/` folder. `/boot/efi` gets **no** exclude, and create
  doesn't use `-x`, so **snapshots hold a full copy of the ESP files** under
  `localhost/boot/efi/`. That stays so (0.4.1); the restore skips it (6b.2).

### 6b.1 When to write: at the next boot (decided)

Compared: A, a live rsync over the running system followed by an immediate restart (what
Timeshift does); B, prepare now and apply early in the next boot. **B, decided 2026-09-30.**

| | A. Live rsync, then restart | B. Prepare now, apply in the next boot |
|---|---|---|
| What runs during the writing | The desktop session, the applet, journald, NetworkManager, the user's apps, all writing while rsync `--delete`s under them | Only early-boot services (`system-update.target`): no desktop, no user session |
| Consistency | Files written during the copy race rsync; a program started mid-copy loads mixed libraries | A quiet filesystem; the result is the snapshot plus the excludes |
| Progress | In the status area throughout | Preparation in the status area; the copy on the boot screen |
| Stop | No clear line while the user sits at a half-rewritten desktop | Everything before the restart can be stopped; nothing after |
| Copy interrupted | A half-restored system, nothing picks it up again | Retried at the next boot (6b.10) |
| Restarts | One | Two, both automatic after one click |

B uses systemd's offline updates (`systemd.offline-updates(7)`, the mechanism PackageKit uses):
a `/system-update` symlink makes the next boot go to `system-update.target` instead of the
desktop, with local filesystems (`/`, `/boot/efi`), udev and cryptswap up. **Condition:**
check 0.3 on apsis-test shows a dummy unit runs there, the desktop stays down, and the next
boot is normal. If it fails, this design comes back for review with A. It does not switch
silently.

### 6b.2 Protected paths and the restore filter (`apsis_core::restore::filter`)

rsync reads rules top to bottom, first match wins, anchored at the transfer root (`/`). The
restore **never** uses `--delete-excluded`, so every excluded path is also protected from
`--delete`: excluded means left exactly as it is on the live system. The list is written to
`/var/lib/apsis/restore/restore.filter` when preparing and used as is at apply time.

**Protect list** (group 1, always first):

| path | what it is |
|---|---|
| `/system-update` | The offline-update link (6b.6) |
| `/etc/systemd/system/apsis-restore.service` and `/etc/systemd/system/system-update.target.wants/apsis-restore.service` | The restore unit and its wants link |
| `/var/lib/apsis/***` | The helper copy that runs the restore, `request.json`, `state.json` (the attempt counter and "written" flag), `restore.filter`, `rsync-log`, the ESP backup (6b.6), `result.json` |
| `/etc/apsis/***` | Apsis's config (the backup disk choice) |

None of these paths belongs to the Apsis package. The .deb ships `/usr/bin/apsis`,
`/usr/libexec/apsis-helper`, the D-Bus, polkit, desktop, icon, metainfo and man files. Those
are **not** protected and come back exactly as the snapshot has them, together with the
snapshot's `/var/lib/dpkg`. So dpkg's record and the packaged files always agree after a
restore, whatever Apsis version the snapshot holds. A test checks the protect list against the
package's file list, so nothing packaged can slip into it. What that means per case (the dialog
says it, from the snapshot's `var/lib/dpkg/status`):

| the snapshot has | after the restore | the dialog says |
|---|---|---|
| this Apsis version | nothing to reconcile | nothing |
| no Apsis | Apsis is gone: dpkg says not installed, and its files are gone. Left behind, owned by no package: `/etc/apsis/` and `/var/lib/apsis/restore/` (the result and log). Reinstalling Apsis picks them up and shows the result | "Apsis wasn't installed yet when this snapshot was made, so it will be gone after the restore. Install it again to see how the restore went." |
| Apsis 0.4.x | the 0.4.x applet and helper. They read the protected config (v2 since 0.4.0) and don't know `/var/lib/apsis/`, so no result line until 0.5.0 is installed again | "This snapshot has Apsis 0.4.x. Install Apsis 0.5 again afterwards to restore again." |
| Apsis 0.3.x or older | the old applet and helper, which refuse the protected v2 config ("version 2, this Apsis reads 1") | "This snapshot has an older Apsis (0.3.x) that can't read the current settings. Install Apsis 0.5 again afterwards." |

The unit, the helper copy and `/system-update` are removed when the apply ends, whatever the
outcome (6b.6 steps 8 and 9). `/etc/apsis/` and `/var/lib/apsis/restore/{result.json,
rsync-log}` stay. The .deb's `postrm purge` also removes `/var/lib/apsis/` and a leftover unit
file.

**The whole filter:**

| # | rule(s) | why |
|---|---|---|
| 1 | the protect list above | A restore must not delete what's running it, its state, or the live Apsis config |
| 2 | `- /boot/efi/***` | The ESP (vfat). Never written by rsync. The snapshot's copy of it is ignored. Only kernelstub writes the ESP, as on every kernel update, plus Apsis's own put-back (6b.6) |
| 3 | `- /recovery/***` | Pop's recovery partition (vfat). Never touched: it's the way back (6b.11) |
| 4 | `- /etc/fstab`, `- /etc/crypttab` | They describe the disks as they are now (cryptswap's partition, the ESP, other disks). The initramfs is rebuilt against them |
| 5 | `- <mount point>` (the plain anchored path, no `/***`: a mount point can be a file, and rsync never enters an excluded folder; owner, 2026-10-01) for every live mount point except `/` (and `/home` when home is restored), from `/proc/self/mountinfo` | Other disks, the backup disk wherever it's mounted (`/run/apsis/backup`, `/media/<user>/...`), `/boot/efi`, `/recovery`, a separate `/home`. rsync would otherwise go into them and `--delete` there |
| 6 | `- /dev/***`, `- /proc/***`, `- /sys/***`, `- /run/***`, `- /tmp/***`, `- /mnt/***`, `- /media/***`, `- /lost+found`, `- /swapfile` | Pseudo and runtime filesystems, temporary files, other disks' mount places. Mostly also caught by 5 and the snapshot's list; stated once so it doesn't depend on either |
| 7 | `- /timeshift/***` | A backup kept on the system disk itself (Timeshift's layout) |
| 8 | `- /var/log/journal/***` | journald writes there during the apply; keeping it keeps the restore's own log (`journalctl -b -1 -u apsis-restore`) |
| 9 | Keep home: `- /home/***` | Home folders stay as they are (6b.3) |
| 10 | Pass 1 only: `- /usr/lib/modules/<running>/***`, `- /boot/vmlinuz-<running>`, `- /boot/initrd.img-<running>`, `- /boot/config-<running>`, `- /boot/System.map-<running>` | The **protected kernel**: the one the ESP boots now keeps its files until the ESP is refreshed and checked, so the ESP always boots a kernel with modules. Removed at the end only if the snapshot doesn't have it and the check passed (6b.6) |
| 11 | The snapshot's own `exclude.list`, line for line | What the snapshot didn't save (caches, `/root` or `/home` if not included, the user's `-` filters) stays as it is instead of being deleted |

Pop!_OS has a merged `/usr` (`/lib` is a symlink to `usr/lib`), so rule 10 names
`/usr/lib/modules`. `/root` needs no rule: if the snapshot has it, it's restored; if not, rule
11 keeps the live one.

### 6b.3 Home folders

The dialog offers the choice only if the snapshot holds home files (its `exclude.list` has a
`+ /home/...` line: Apsis's `+ /home/**`, or Timeshift's per-user `+ /home/<user>/**`).
Otherwise it says "Home folders aren't in this snapshot, so they stay as they are." and home
is kept.

- **Keep my files as they are now** (default). **Keep means `/home` is fully excluded:**
  rule 9 `- /home/***` matches `/home` itself and everything under it. rsync never compares,
  writes or deletes anything there, not even `/home`'s own owner, mode or time, and a
  separate `/home` partition isn't entered at all (rule 5). Settings in home may be newer than
  the restored apps expect. Apps cope with that far better than with lost documents.
- **Restore them too**: `/home` becomes what the snapshot holds. Files created or changed after
  the snapshot are **deleted or put back to their old version**. The exceptions are parts the
  snapshot didn't include (caches, the user's `-` filters, homes of users the snapshot didn't
  cover), which rule 11 keeps. With the safety snapshot on, it includes `/home` this one time,
  whatever the Include setting says, so the lost files are on the backup disk. Homes of users
  added after the snapshot keep their folders, but their accounts are gone (`/etc/passwd` is
  restored). The README says so.

### 6b.4 Safety snapshot and disk space

- **Safety snapshot, on by default**: a checkbox, "Take a snapshot of the current system first
  (recommended)". A normal create (0.4.x code, new format from 0.4.1), comment `Before
  restoring <snapshot date>`. It includes `/home` when home is restored, else it follows the
  Include setting.
- **Space**, from rsync `--dry-run --stats` ("Total transferred file size"), read-only, before
  anything is copied:
  - backup disk: the safety snapshot's size + 1 GiB free;
  - system disk: the restore's transfer size + 1 GiB (or 2% of the disk, whichever is more)
    free on `/`. This is an upper bound: rsync also frees space as it deletes.
  - **every destination partition** (owner, 2026-10-01): when home is restored and `/home` is
    a separate mount, the part of the transfer that lands under `/home` is checked against
    the free space of `/home`'s filesystem (same margin), and only the rest against `/`. Not
    a refusal by itself: a separate `/home` is restored like any other (6b.2 rule 5), and only
    a short partition refuses, with the system-disk line.
- **Checked again at "Restart now"** (the free space can change while the prompt waits): a
  `statvfs` of `/` (and of a separate `/home` being restored) against the needs recorded in
  `request.json`. Everything is checked **before the
  restart**. Short: refused with **one line**, and nothing is armed:
  - "Not enough space on the backup disk for a safety snapshot (needs 14 GB, 9 GB free)."
  - "Not enough space on the system disk to restore (needs 6 GB, 3 GB free)."
- Order: refusals (6b.7), both dry runs and space, the safety snapshot, write the plan. The
  slow part comes after every check that could refuse.

### 6b.5 Stop, and a restart that can't be undone

| phase | where | stop? |
|---|---|---|
| Checks, dry runs, safety snapshot, writing the plan to `/var/lib/apsis/restore/` | desktop, status area | **Yes**: the Stop button, as for a create. A safety snapshot being made is removed like a stopped create; a finished one is kept, and the status says so ("Restore stopped. The safety snapshot was kept.") |
| Ready: the "Ready to restore" prompt | desktop | **Yes**: "Cancel restore", Esc or closing the window removes the plan (a finished safety snapshot stays). Nothing is armed until "Restart now", so a normal restart does **not** restore. There's no "Restart later" and no waiting state: anything changed between the safety snapshot and a later restart would be in no snapshot, and the restore would wipe it silently (owner, 2026-09-30) |
| After "Restart now" | boot screen | **No.** The prompt said "Once the computer restarts, the restore can't be stopped. Don't turn it off until it's done." The boot screen says "Restoring the system. Don't turn off the computer." |

The ready prompt is modal. A plan left on it for more than 30 minutes is too old: Restart now
then says "The preparation is too old. Start the restore again." and cleans up. While the prompt
is up, Create, Delete, Settings Save and another Restore get Busy, so the snapshot can't be
deleted from under it. A plan whose window went away without an answer (crash, logout) is
removed at the helper's next call.

### 6b.6 The apply, and the boot files

Everything runs from `apsis-restore.service` in offline-update mode, as root, with the helper's
fixed `PATH` and cleared environment. Each step is logged to the journal and the console;
progress and messages go to plymouth (`plymouth system-update --progress=N`, `plymouth
display-message`) when it runs.

`state.json` holds `attempts` (copies that started) and `written` (anything under `/` was ever
written by this restore), both fsynced before the step that depends on them.

1. **Arm check.** `/system-update` must point to `/var/lib/apsis/restore`, and `request.json`
   and `state.json` must be there. Otherwise remove the link and boot normally (a stray link
   isn't Apsis's to act on beyond that).
2. **Backup disk.** Wait up to 60 s for `/dev/disk/by-uuid/<uuid>` (USB disks are slow at
   boot), mount it read-only (`ro,nosuid,nodev,noexec`) at `/run/apsis/backup`, then the delete's
   path checks (`O_NOFOLLOW` walk, `info.json` a regular file, nothing mounted inside) and the
   refusals of 6b.7 once more. **A separate `/home`** (owner, 2026-10-01): when home is
   restored and `/home` was a separate mount when the plan was made, `/home` must be mounted
   now, and its filesystem UUID must be the one in `request.json`. Otherwise rsync would
   write the home files onto the system disk under `/home`, or into another disk. Any of
   these fails: **never started** (6b.10), before rsync runs.
3. **Pass 1.** `attempts += 1`, `written = true`, fsync; then:
   ```
   rsync -a -A -X --numeric-ids --delete --force --sparse --stats --info=progress2 \
     --log-file=/var/lib/apsis/restore/rsync-log \
     --exclude-from=/var/lib/apsis/restore/restore.filter \
     /run/apsis/backup/timeshift/snapshots/<name>/localhost/ /
   ```
   `-A -X` only for a new-format snapshot (0.4.1). An old-format one is restored with `-a
   --numeric-ids` and the rest, without `-A -X`. With `-X`, rsync would strip the live xattrs
   (file capabilities) even from unchanged files, to match a snapshot that never stored them.
   Never `--delete-excluded`, `-L` or `--link-dest`. Exit 0 or 24: go on. 23 (some files
   couldn't be written or deleted): go on; the result is "restored with problems" and names
   the log. Anything else: **copy broke** (6b.10).
4. **ESP backup.** Copy the ESP's `EFI/Pop_OS-<root-uuid>/{vmlinuz.efi,initrd.img}` and
   `loader/entries/Pop_OS-{current,oldkern}.conf` to `/var/lib/apsis/restore/esp-backup/`
   (on `/`, protected), fsync, and byte-compare each copy with the original. A copy that
   doesn't match: stop before touching the boot files, and go to the **boot files failed**
   path below with nothing to put back. This happens before `update-initramfs`, because Pop's
   post-update hook runs kernelstub itself.
5. **Boot files**, on the restored `/`. No chroot is needed: the running system *is* the target,
   and the tools are the restored system's own:
   ```
   update-initramfs -u -k all      # initrds for the restored kernels, against the live crypttab
   kernelstub --verbose            # copies /boot/vmlinuz and /boot/initrd.img to the ESP and
                                   # writes the entries from /etc/kernelstub/configuration
   ```
   kernelstub is called explicitly, not only through the hook.
6. **Check, byte for byte**: the ESP's `vmlinuz.efi` and `initrd.img` equal the files
   `/boot/vmlinuz` and `/boot/initrd.img` point to; `/usr/lib/modules/<that version>/` exists;
   `Pop_OS-current.conf` exists. Both commands exited 0.
   - **Passes:** go on.
   - **Fails** (or a command failed): **boot files failed**. Put the four backed-up files back
     on the ESP (each written to a temporary name in the same folder, fsynced, renamed), then
     byte-compare them with the backup. The protected kernel stays (step 7 is skipped), so the
     ESP boots the kernel it booted before, whose modules and `/boot` files rule 10 kept. Then
     step 8, result `boot-kept`: the system is the snapshot's, but it still starts the newer
     kernel. Not retried (a second kernelstub run would do the same). If putting back also
     fails to compare: result `boot-broken`, and the boot screen and result point to the
     recovery steps (6b.11).
7. **Protected kernel, cleanup** (only after a passed check): if the snapshot doesn't have the
   running kernel's version, remove the rule 10 files (the modules folder with
   `prune::remove_at`, one filesystem, no symlinks followed). The system now matches the
   snapshot. A power cut here is harmless: the ESP already boots the restored kernel.
8. **End.** Remove `/system-update`, the unit and its wants link, the helper copy and
   `esp-backup/`. Write `result.json` (`done`, `problems` or `boot-kept`, the snapshot, the
   safety snapshot, the time), `sync`, restart.

The unit (text in `apsis_core`, tested), written to `/etc/systemd/system/` on arm and **not**
shipped in `/usr` (a snapshot older than Apsis would delete it mid-restore, and a retry needs
it):

```ini
[Unit]
Description=Apsis: restore the system from a snapshot
DefaultDependencies=no
Requires=sysinit.target
After=sysinit.target system-update-pre.target local-fs.target
Before=system-update.target shutdown.target
ConditionPathIsSymbolicLink=/system-update
OnFailure=reboot.target

[Service]
Type=oneshot
ExecStart=/var/lib/apsis/restore/apsis-helper --apply-restore
StandardOutput=journal+console
```

The helper binary is copied to `/var/lib/apsis/restore/` on arm, for the same reason. No
`daemon-reload` is needed: the unit is read at the next boot.

### 6b.7 Refusals (in the dialog, again when preparing, again at apply)

Each opens the "Can't restore this snapshot" dialog instead: the reason in one plain line, one
line on what to do, and Close (wording in 6b.8's string table). The lines below are the gist:

| check | line |
|---|---|
| no `/sys/firmware/efi` | "Restore needs a computer that starts in UEFI mode." |
| no `/etc/kernelstub/configuration` or `kernelstub` (GRUB, other distros) | "Restore only works on Pop!_OS with systemd-boot for now." |
| ESP not mounted at `/boot/efi` as vfat, or no `EFI/Pop_OS-<root-uuid>/` on it | "The boot partition isn't where Pop!_OS keeps it." |
| root filesystem isn't ext4 | "Restore doesn't support a <fstype> system disk yet." |
| root on `/dev/mapper/*`, dm-crypt, LVM (lsblk `TYPE` not `part`) | "Restore doesn't support an encrypted or LVM system disk yet." |
| `/boot`, `/usr` or `/var` a separate mount | "Restore doesn't support a system split over several partitions yet." |
| **root UUID**: the snapshot's `info.json` `sys-uuid` differs from the UUID of the filesystem mounted at `/` (`findmnt`), or the snapshot's `etc/kernelstub/configuration` names another `root=UUID=` | "This snapshot is from another installation." |
| snapshot isn't rsync, has no `localhost/` or `exclude.list` | "This snapshot can't be restored: <reason>." |
| snapshot's `/boot/vmlinuz` has no `/usr/lib/modules/<version>/` in the snapshot, or the snapshot has no `update-initramfs` or `kernelstub` | "This snapshot's kernel files are incomplete, so it can't be restored safely." |
| `/system-update` already exists (a pending system update) | "A system update is waiting for a restart. Restart first, then restore." |
| not enough space (6b.4) | the space line |
| Timeshift's lock is held; another Apsis job runs | the existing Busy wording |

The checks are pure functions on text that has already been read (mountinfo, lsblk JSON,
`info.json`, the kernelstub configuration, a folder listing), so the tests don't need root.
Not refusals, but dialog lines: the Apsis version reconciliation (6b.2) and the old format
(0.4.1).

### 6b.8 6b UI

**Reference: the throwaway preview** on branch `preview-6b-ui` (never merged, debug builds
only): `./target/debug/apsis --window --preview <state>`, one state per command (`restore`,
`restore-home-no-safety`, `restore-old-format`, `restore-no-apsis`, `restore-apsis-0.4`,
`restore-apsis-0.3`, `restore-refused`, `preparing`, `ready`, `boot-screen`, `result-done`,
`result-boot-kept`, `result-failed`). Reviewed by the owner on 2026-09-30. The mockups below
are drawn from those screens. The UI slice rebuilds them in the real code; the preview's
layout test comes along (see Tests).

**Rule:** 0.4.0's look is the baseline. No new visual style: each element reuses a 0.4.0
pattern, named in the table under each mockup. A dialog may be larger than 0.4.0's when that
keeps the flow clearer; the restore dialog is, so every choice sits in one place.

#### Toolbar

```
┌ Apsis ─────────────────────────────────────────────── ↻  ⓘ  ✕ ┐
│ [+ Create]  [↺ Restore]  [🗑 Delete]  [⚙ Settings]              │
```

| element | 0.4.0 pattern |
|---|---|
| Restore, between Create and Delete | toolbar button: standard, leading symbolic icon (`document-revert-symbolic`), no tooltip; the contract's order |

- Restore is enabled with exactly one snapshot selected (not a leftover row), the backup disk
  connected, and no job running.
- **Restore has no keyboard shortcut** (owner, 2026-09-30): it's reached by a click, or by
  Tab to the button and then Enter. 0.4.0's keys and its rule stay as they were: keys are
  listed only in the README and man page, and tooltips don't name them.
- There's only one kind of restore. File restore went in 0.4.0, so there's no choice step.
- Enter and double-click on the list do nothing, as in 0.4.0.

#### The restore dialog

At 720 x 520 (`restore`):

```
        ┌──────────────────────────────────────────────────────────┐
        │ Restore the system?                                      │
        │ 2026-09-25 11:28 · before driver update          (muted) │
        │ ┌──────────────────────────────────────────────────────┐ │
        │ │ Puts system files back to this snapshot and restarts │ │ ▲ scrolls
        │ │ twice.                                               │ │ │ when the
        │ │ Home folders                                         │ │ │ window is
        │ │ ┌──────────────────────────────────────────────────┐ │ │ │ too short
        │ │ │ (•) Keep my files as they are now                │ │ │ │
        │ │ │ ( ) Restore them too                             │ │ │ │
        │ │ └──────────────────────────────────────────────────┘ │ │ │
        │ │ ┌──────────────────────────────────────────────────┐ │ │ │
        │ │ │ [✓] Take a snapshot of the current system first  │ │ │ │
        │ │ │     (recommended)                                │ │ │ │
        │ │ └──────────────────────────────────────────────────┘ │ │ │
        │ │ Experimental · if it won't start, see "If a restore  │ │ │
        │ │ goes wrong" (README)                         (muted) │ │ ▼
        │ └──────────────────────────────────────────────────────┘ │
        │                                     [Cancel] [Restore]   │  always shown
        └──────────────────────────────────────────────────────────┘
```

"Restore them too" chosen **and** the safety snapshot off (`restore-home-no-safety`):

```
        │ │ │ ( ) Keep my files as they are now                │ │
        │ │ │ (•) Restore them too                             │ │
        │ │ │     Files changed after 2026-09-25 11:28 are     │ │  (item description)
        │ │ │     lost for good.                               │ │
        │ │ │ [ ] Take a snapshot of the current system first  │ │
```

Extra line above the experimental line, in body text, one per case:

```
restore-old-format   Older format: this snapshot was made without ACLs and extended
                     attributes. A few system files may come back without them.
restore-no-apsis     Apsis wasn't installed yet when this snapshot was made, so it will be
                     gone after the restore. Install it again to see how the restore went.
restore-apsis-0.4    This snapshot has Apsis 0.4.0. Install Apsis 0.5 again afterwards to
                     restore again.
restore-apsis-0.3    This snapshot has an older Apsis (0.3.1) that can't read the current
                     settings. Install Apsis 0.5 again afterwards.
```

| element | 0.4.0 pattern |
|---|---|
| The dialog, title, Cancel + destructive Restore | Delete dialog (`widget::dialog`, `button::destructive`) |
| Muted line | dimmed text (Create dialog's includes line) |
| Home folders radios | Location tab: `settings::section` with `item::radio` |
| Safety snapshot | Include tab: `settings::item::checkbox` |
| "lost for good" | a settings item's description |
| Old-format and Apsis lines | body text |

- **Title** "Restore the system?", on one line. It says plainly that the whole system is restored.
- **Muted line**: the date as the list shows it, then ` · ` and the comment if there is one.
  Not the folder name.
- **The body scrolls, and the buttons are always shown.** The body's height is capped at the
  window height minus 216 px, the height of everything else in the dialog plus an 8 px margin
  top and bottom. The dialog is centred over the whole window, header included. The window
  tracks its height from resize events. At 720 x 520 the normal dialog doesn't scroll; at
  640 x 440 it does. A layout test checks every state at both sizes (Tests).
- **Old format**: this dialog is the main place it's shown. The list row's tooltip also
  says "Older format: made without ACLs and extended attributes". There's no glyph or column in
  the list, and no details area.
- `/root` isn't mentioned in the dialog. The README explains it (restored with the system if
  the snapshot has it).
- **Focus and Enter** (owner, 2026-09-30: no forced focus): the dialogs open with libcosmic's
  default focus, which is none. In libcosmic, Enter only presses the button that has keyboard
  focus, so Enter in a freshly opened dialog does nothing. Restore starts only with a click,
  or by moving focus to it with Tab and then pressing Enter. The password prompt is the second
  guard. The same holds for every restore dialog.
- Esc closes the dialog (0.4.0).

#### Can't restore (`restore-refused`)

```
        ┌──────────────────────────────────────────────────────────┐
        │ Can't restore this snapshot                              │
        │ 2026-09-25 11:28 · before driver update          (muted) │
        │                                                          │
        │ It was made on another installation of the system, so    │
        │ its files don't fit this one.                            │
        │ Pick a snapshot made on this installation.               │
        │                                                  [Close] │
        └──────────────────────────────────────────────────────────┘
```

The reason in the error colour (as a bad pattern in Add Pattern), then one line on what to do
in plain text. Each refusal in 6b.7 has such a pair (string table).

#### Preparing (`preparing`)

```
│ Last snapshot   26h ago                                            │
│ Backup disk     sdb1  347G / 868G · 41% used · 521G free           │
│ ████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░             │
│ Preparing restore · safety snapshot · 42% · 3 min left    [ Stop ] │
│ ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░             │
```

0.4.0's job line (text, destructive Stop, a determinate bar). Stop asks "Stop the restore?"
(0.4.0's Stop dialog).

#### Ready to restore (`ready`)

```
        ┌──────────────────────────────────────────────────────────┐
        │ Ready to restore                                         │
        │ Save your work and close your apps first.                │
        │                                                          │
        │ The computer restarts, restores the system to 2026-09-25 │
        │ 11:28, and restarts once more. Once it restarts, the     │
        │ restore can't be stopped. Don't turn off the computer    │
        │ until it's done.                                         │
        │                           [Cancel restore] [Restart now] │
        └──────────────────────────────────────────────────────────┘
  status area under it:  Preparing restore · ready   (full bar, no Stop)
```

- Only two buttons. **Nothing is armed until "Restart now"** (6b.5): there's no "Restart later"
  and no waiting state.
- **Closing the pop-up counts as Cancel restore**: Esc, Cancel restore and closing the window
  all cancel and clean up. The plan is removed, and a finished safety snapshot is kept
  ("Restore cancelled. The safety snapshot was kept.", dimmed, as 0.4.0's "Create stopped").
- It opens with libcosmic's default focus (none), so Enter does nothing until focus is moved
  with Tab. Restart now needs a click, or Tab to it and then Enter.

#### Boot screen (`boot-screen` prints it)

Plymouth's message and progress (`plymouth display-message`, `system-update --progress`), or
console text without plymouth:

```
Restoring the system to 2026-09-25 11:28 · 42%
Don't turn off the computer.
  … Setting up the boot files…
  … System restored. Restarting…

copy broke:      The restore was interrupted. Restarting to try again (attempt 2 of 3).
never started:   The restore didn't start: the backup disk wasn't found. Nothing was changed.
                 Starting normally.
after attempt 3: The restore couldn't finish after 3 tries. The system may be partly restored.
                 Starting it now. Apsis explains after you log in.
boot files:      The system is restored, but its boot files didn't check out.
                 Keeping the kernel it started with. Restarting…
```

#### After login

The result shows only in the window's status line, where 0.4.0 shows a job's result, with the
full text in its tooltip (0.4.0 has no Log since the rooms went). No desktop notification, and
no change to the panel icon's colour or its tooltip (owner, 2026-09-30):

| result | status line (role on the marked phrase only) |
|---|---|
| done | `System restored to 2026-09-25 11:28` (plain) |
| boot-kept | `System restored · `**`still boots the previous kernel`**` · see README` (warning) |
| failed | `Restore `**`incomplete`**` · system partly restored · reconnect the backup disk`  [Restore again] (destructive) |
| not started | `The restore didn't start · nothing was changed · <what to do>` (plain) |

```
│ Backup disk     sdb1  347G / 868G · 41% used · 521G free           │
│ ████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░             │
│ System restored · still boots the previous kernel · see README     │   boot-kept
│ Restore incomplete · system partly restored · reconnect the backup disk  [Restore again] │   failed
```

- The failed line's last part names what to do and varies with the reason: "reconnect the
  backup disk", "free space on the system disk", "see README" (string table).
- The last part of a line wraps instead of being cut off.
- **Restore again** opens the normal restore dialog for the same snapshot, with the normal
  defaults (home kept, safety snapshot on). (6b.10)

#### Strings (new, user-facing)

0.4.0's copy style: sentence case, ` · ` between parts, `…` for "more follows", no em or en
dashes, no rsync or paths on screen (the helper's own words only in tooltips).

| where | string |
|---|---|
| toolbar | `Restore` |
| row tooltip | `Older format: made without ACLs and extended attributes` |
| dialog title | `Restore the system?` |
| dialog muted | `{date} · {comment}` / `{date}` |
| dialog | `Puts system files back to this snapshot and restarts twice.` |
| dialog section | `Home folders` |
| radio | `Keep my files as they are now` |
| radio | `Restore them too` |
| radio description | `Files changed after {date} are lost for good.` |
| checkbox | `Take a snapshot of the current system first (recommended)` |
| dialog, no home in snapshot | `Home folders aren't in this snapshot, so they stay as they are.` |
| dialog | `Older format: this snapshot was made without ACLs and extended attributes. A few system files may come back without them.` |
| dialog | `Apsis wasn't installed yet when this snapshot was made, so it will be gone after the restore. Install it again to see how the restore went.` |
| dialog | `This snapshot has Apsis {version}. Install Apsis 0.5 again afterwards to restore again.` |
| dialog | `This snapshot has an older Apsis ({version}) that can't read the current settings. Install Apsis 0.5 again afterwards.` |
| dialog muted | `Experimental · if it won't start, see "If a restore goes wrong" (README)` |
| buttons | `Cancel`, `Restore` |
| refusal title | `Can't restore this snapshot` |
| refusal | `It was made on another installation of the system, so its files don't fit this one.` / `Pick a snapshot made on this installation.` |
| refusal | `This computer doesn't start in UEFI mode, which restore needs.` / `Restore can't be used on this computer.` |
| refusal | `Restore works only on Pop!_OS with systemd-boot for now.` / `Restore can't be used on this computer yet.` |
| refusal | `The boot partition isn't where Pop!_OS keeps it.` / `Restore can't be used on this computer.` |
| refusal | `Restore doesn't support a {fstype} system disk yet.` / `Restore can't be used on this computer yet.` |
| refusal | `Restore doesn't support an encrypted or LVM system disk yet.` / `Restore can't be used on this computer yet.` |
| refusal | `Restore doesn't support a system split over several partitions yet.` / `Restore can't be used on this computer yet.` |
| refusal | `This snapshot's kernel files are incomplete.` / `Pick another snapshot.` |
| refusal | `This snapshot can't be read: {reason}.` / `Pick another snapshot.` |
| refusal | `A system update is waiting for a restart.` / `Restart first, then restore.` |
| refusal | `Not enough space on the backup disk for a safety snapshot (needs {n}, {free} free).` / `Delete old snapshots, or turn off the safety snapshot.` |
| refusal | `Not enough space on the system disk to restore (needs {n}, {free} free).` / `Free some space, then try again.` |
| button | `Close` |
| status | `Preparing restore · checking the snapshot…` |
| status | `Preparing restore · checking disk space…` |
| status | `Preparing restore · safety snapshot · {percent} · {time} left` |
| status | `Preparing restore · ready` |
| stop dialog | `Stop the restore?` / `The safety snapshot being made is deleted. Nothing on the system has changed.` / `Stop`, `Keep Going` |
| status | `Restore stopped` / `Restore stopped. The safety snapshot was kept.` |
| ready title | `Ready to restore` |
| ready | `Save your work and close your apps first.` |
| ready | `The computer restarts, restores the system to {date}, and restarts once more. Once it restarts, the restore can't be stopped. Don't turn off the computer until it's done.` |
| buttons | `Cancel restore`, `Restart now` |
| status | `Restore cancelled. The safety snapshot was kept.` / `Restore cancelled` |
| ready, too old | `The preparation is too old. Start the restore again.` |
| boot screen | the six lines under "Boot screen" |
| result | `System restored to {date}` |
| result | `System restored · still boots the previous kernel · see README` |
| result | `Restore incomplete · system partly restored · {what to do}` |
| what to do | `reconnect the backup disk` / `free space on the system disk` / `see README` |
| result | `The restore didn't start · nothing was changed · {what to do}` |
| button | `Restore again` |
| result tooltips | the helper's reason, then: `Restored from snapshot {name} ("{comment}"). Home folders were kept. Safety snapshot: {date}.`; `Restored to {date}. The new boot files didn't check out, so the ones it had were put back: it runs on the kernel from before the restore ({version}). The next kernel update sets this right. See "If a restore goes wrong" in Apsis's README.`; `Stopped after 3 tries: {reason}. Reconnect it and click Restore again, or restore the safety snapshot from {date}.` |

### 6b.9 Helper

New methods on `Helper2`. They're additions, so the interface version stays. All go through
the one lock (a ready plan counts as held for everything but its own restart and cancel):

| method | polkit | |
|---|---|---|
| `CheckRestore(s snapshot) -> (b ok, s refusal, b has_home, b has_root, b old_format, s apsis_note)` | `list`, not interactive | Mounts read-only like `List`; the 6b.7 checks; fills the dialog |
| `Restore(s snapshot, b restore_home, b safety_snapshot)` | **`restore`, `auth_admin`, asked every time** | Returns once started. Job kind `restore`, with `JobChanged` progress and `Finished("restore", ok, msg)`; ok means "ready" |
| `RestartToRestore(s snapshot)` | none for the uid that started the plan, else `restore` | Re-checks space and freshness, arms (unit, helper copy, `state.json`, `/system-update`, `sync`), then logind `Reboot(false)` over the system bus (zbus, no new crate) |
| `CancelRestore()` | none for the starter's uid, else `restore` | Removes the plan (not armed yet) |
| `RestoreResult() -> (s state, s snapshot, s message, x when)` | `list` | `ready`, `done`, `problems`, `boot-kept`, `boot-broken`, `not-started`, `failed` or `""`. The window asks when it opens (6b.8) |
| `Stop(s snapshot)` (existing) | as now | Also stops a restore's preparation |

- New polkit action `io.github.atraxsrc.Apsis.restore`: `auth_admin` for active, inactive and
  any (no `_keep`). That makes six actions; the resource test counts them.
- Input checks as for delete: the name matches `YYYY-MM-DD_HH-MM-SS`, is in a fresh list and
  isn't a leftover, and `snapshots/<name>/localhost/` is reached with `O_NOFOLLOW` from the
  mount. The helper runs every 6b.7 check itself; the applet isn't trusted.
- The starter's uid (`GetConnectionUnixUser`) is kept in `request.json`, so it survives the
  helper's idle exit.
- `/var/lib/apsis/restore/` (root, 0700): `request.json` (snapshot, backup UUID, home choice,
  format, safety snapshot name, root UUID, running kernel, space needed per destination
  partition, the separate `/home`'s filesystem UUID when home is restored and `/home` is its
  own mount (else none), starter uid, prepared-at), `restore.filter`, `state.json`, `rsync-log`, `esp-backup/`, `result.json`, and
  the helper copy while armed. serde_json, already a dependency.
- `apsis-helper --apply-restore` is the offline entry point. It uses no D-Bus.
- Journal: `restore "2026-09-25_11-28-00" keep-home safety for :1.42: ready`, `armed;
  restarting`; in the offline boot, each step, the attempt number, rsync's summary, both boot
  commands' output, the ESP backup and check results.
- No new crates: rsync, update-initramfs, kernelstub, plymouth and logind are called like
  today's tools (fixed argv, fixed `PATH`).

### 6b.10 Failure states

| state | when | what happens | counted as an attempt? |
|---|---|---|---|
| **never started** | Step 1 or 2 fails: the backup disk isn't there after 60 s, the mount fails, a path check or refusal fails, the snapshot is gone, a separate `/home` being restored isn't mounted or has another UUID than the plan's | Nothing is written in this boot. Remove `/system-update`, the unit and the helper copy; write `result.json` `not-started` with the reason; boot normally. The boot screen says "The restore didn't start: the backup disk wasn't found. Starting normally." After login: the result line (6b.8) | **No** |
| **copy broke** | Pass 1 exits with anything but 0, 23 or 24 (disk pulled out, I/O error, disk full), or the power goes during pass 1 | `/system-update` stays. The boot screen says "The restore was interrupted. Restarting to try again (attempt 2 of 3)." and restarts; the next boot runs from step 1 (rsync picks up where the files differ) | **Yes** |
| **boot files failed** | Step 4 or 6 | The ESP files are put back, the protected kernel is kept, `boot-kept` (or `boot-broken`), end (6b.6) | Not retried |

"Never started" after an earlier broken copy (the disk was pulled, and is still missing at the
next boot) also removes the link and boots normally. The result line then says the restore
didn't finish (the `written` flag), not that nothing changed, and offers Restore again.

**After attempt 3 breaks**, exactly:
- Machine state: `/` is partly restored. Some files are the snapshot's, the rest are as they
  were. Which ones depends on where rsync stopped: rsync goes through the tree in order and
  deletes as it goes. The package database may not match the files. `/home` is untouched with
  "keep". The ESP is **untouched** (steps 4 to 7 never ran), so it still boots the kernel it
  booted before, and that kernel's modules and `/boot` files are there (rule 10). `/etc/fstab`,
  `/etc/crypttab`, `/etc/apsis/`, the journal and `/recovery` are as they were. The safety
  snapshot, if taken, is on the backup disk.
- Apsis removes `/system-update`, the unit and the helper copy, writes `result.json` `failed`
  with the last rsync error, and boots normally.
- The boot screen says: "The restore couldn't finish after 3 tries. The system may be partly
  restored. Starting it now; Apsis will explain after you log in."
- After login, if the desktop comes up (likely, since the booted kernel has its modules): in
  the window's status line, "Restore incomplete · system partly restored · reconnect the backup
  disk" (the last part follows the reason) with [Restore again] (6b.8). Restoring the safety snapshot
  instead goes back to where the user started. If the desktop doesn't come up, see 6b.11.
- **Restore again** opens the normal restore dialog for the same snapshot, with the normal
  defaults (home kept, safety snapshot on; owner, 2026-09-30). It runs the whole restore again
  (checks, preparation, ready prompt), and rsync finishes the copy from wherever it stopped.

A power cut in the other steps: during 4 to 6, the next boot sees `written` and runs from
step 1. Pass 1 then finds almost nothing to copy, and steps 4 to 6 run again: the ESP backup
is taken again only if the ESP still matches the protected kernel, otherwise the earlier backup
is kept. During 7 or 8: harmless, and the next boot finishes the cleanup.

### 6b.11 If a restore breaks booting

The recovery partition and the ESP's `Pop_OS-oldkern` entry are never touched by a restore. The
README gets a section, "If a restore goes wrong". A copy with this machine's UUIDs and the
snapshot name filled in is written to the backup disk (`timeshift/apsis-restore-RECOVER.txt`)
while preparing. It holds only UUIDs and the snapshot name: no user names, no emails.

1. At power-on, hold Space for the systemd-boot menu and pick **Pop!_OS Recovery**, or boot a
   Pop!_OS live USB of the same version.
2. In a terminal:
   ```
   sudo mount /dev/disk/by-uuid/<root-uuid> /mnt
   sudo mount /dev/disk/by-uuid/<esp-uuid> /mnt/boot/efi
   sudo mkdir -p /media/backup && sudo mount -o ro /dev/disk/by-uuid/<backup-uuid> /media/backup
   # finish the same restore (or pick the safety snapshot to go back):
   sudo rsync -a -A -X --numeric-ids --delete --force --sparse \
     --exclude-from=/mnt/var/lib/apsis/restore/restore.filter \
     /media/backup/timeshift/snapshots/<name>/localhost/ /mnt/
   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done
   sudo chroot /mnt update-initramfs -u -k all
   sudo chroot /mnt kernelstub --verbose
   sudo rm -f /mnt/system-update
   ```
   (`-A -X` only for a new-format snapshot; RECOVER.txt has the right line.) The filter's
   rules are anchored at the transfer root, so they work unchanged against `/mnt/`. Its rule 5
   lines name the installed system's mount points, which are right for `/mnt` too. `--rbind
   /sys` brings `efivars` for kernelstub. `--numeric-ids` matters here: the live system's user
   database isn't the installed one.
3. Restart.

Timeshift from a live USB can also read these snapshots, but its boot refresh isn't
kernelstub's, so the README gives the steps above.

### 6b.12 Tests

No root (run by Claude):
- **Filter**: the exact list for keep and restore home; the protect list first; rule 5 from
  mountinfo fixtures (ESP vfat, `/recovery`, a data disk, the backup disk under `/media`,
  `/run/apsis/backup`, a separate `/home` kept or restored, a bind mount); rule 10 only in pass
  1 and for the running version; the real Timeshift `exclude.list` appended unchanged.
- **Protect list vs the package**: no protected path is one the .deb installs (the list from
  the justfile's install recipe / deb metadata).
- **Apsis in the snapshot**: `var/lib/dpkg/status` fixtures (none, 0.3.x, 0.4.x, this version)
  give the right dialog line.
- **argv**: exact, new and old format; never `--delete-excluded`, `-L`, `--copy-links`,
  `--link-dest`, `-H`.
- **Refusals**: every row of 6b.7 from fixtures, including the root UUID from `info.json` and
  from the kernelstub configuration.
- **Home detection** from `exclude.list` (Apsis v2 with and without `+ /home/**`, Timeshift
  per-user lines, `/root` only).
- **Space**: parsing `--stats`; both thresholds; the re-check at restart; a separate `/home`
  being restored is checked on its own filesystem.
- **Apply state machine** (fake runner and fake ESP, temp root): never started does not count
  and removes the link (also for a separate `/home` that's missing or has another UUID, with
  rsync never run); copy broke counts and keeps it; the third break gives up with `failed`;
  23 vs 11; ESP backup, a failed check puts the files back and keeps the protected kernel
  (`boot-kept`); a put-back that doesn't compare gives `boot-broken`; cleanup only after a
  passed check and only when the snapshot lacks the kernel; a power cut at each step
  (re-entering from step 1 with the saved state) ends in the same result.
- **Real rsync** on temp trees (as the tester, no root): a fake snapshot over a fake live root
  with the real filter: changed files replaced, new system files removed, home kept (including
  `/home`'s own mode) or restored, the protect list and other protected paths untouched,
  paths the snapshot's `exclude.list` left out kept; an old-format snapshot doesn't strip a
  live `user.*` xattr from an unchanged file.
- **Unit text** and its install path; **RECOVER.txt** has only UUIDs and the snapshot name.
- **Helper**: introspection (new methods), policy file (6 actions, `restore` is `auth_admin`
  everywhere), Busy while a plan is ready, the starter exemption for restart and cancel, the
  expired plan.
- **Applet** (messages into the model): Restore enabled only with one real snapshot; no key
  opens the dialog; Enter and double-click on the list open nothing; the dialogs open with no
  forced focus; home radios default to keep and are
  hidden without home; the safety snapshot is on by default; "lost for good" only with
  "Restore them too" and no safety snapshot; the old-format and Apsis-version lines; the
  refusal dialog; preparing lines with Stop; the ready prompt has two buttons and Esc
  cancels; each result line with its role on the phrase only; Restore again opens the normal
  dialog for the same snapshot with the normal defaults; the panel icon and tooltip don't
  change.
- **Layout** (`APSIS_LAYOUT_TEST=1`, from the preview's `preview_states_fit_both_window_sizes`):
  every restore state at 720 x 520 and 640 x 440. Each dialog, buttons included, is at most
  the window's height minus 16 px, and the page under it fits. With `APSIS_SCREENSHOTS`, each
  state is written as `preview-<state>-<w>x<h>.png` (the `image` crate already in the tree, as
  a dev-dependency; no new crate).

**Harness on apsis-test** (SSH; root steps are the owner's, Claude writes the exact commands):
- **Baseline first.** Take the baseline snapshot only after apsis-test has, in its system files:
  the Wi-Fi connection, the `sudoers.d` entry the SSH checks use, and the test-only polkit rule
  that lets the SSH session call the helper. Every snapshot the tests restore must be taken
  after that. An earlier one would roll them back, and the restore would cut the SSH link it's
  being watched through. The sudoers and polkit files live only on apsis-test: never in the
  repo, never in the .deb, and removed when testing ends. Their exact content is the owner's
  (it names the test account).
- A small read-only `tools/restore-check.sh`, run with sudo after each restore: ESP vs `/boot`
  hashes, modules for `uname -r`, `dpkg --audit`, `/system-update` gone, the unit's journal,
  `result.json`. A VM harness (OVMF, a Pop!_OS install) isn't worth it for 0.5.0: it needs root
  and KVM, and the risk is Pop's boot chain on real hardware, which apsis-test is.

Checks:
- **0.1** Facts: `sudo ls -la <backup>/timeshift/snapshots/<name>/localhost/boot/efi
  .../localhost/recovery`; `ls /etc/initramfs/post-update.d/`; `cat
  /etc/kernelstub/configuration`; `ls /usr/lib/systemd/system-generators/ | grep update`;
  `plymouth --help | grep system-update`.
- **0.2** The recovery partition boots (hold Space, Pop!_OS Recovery).
- **0.3** Offline-mode spike, **before the helper slice**: a dummy unit that only logs, armed by
  hand with a `/system-update` link. It runs, the desktop doesn't start, the unit removes the
  link and restarts, and the next boot is normal. This is the condition on 6b.1.
- 1. Keep home, safety on: after the snapshot, add `/etc/apsis-test-marker`, a file in `~`, and
  `sudo apt install cowsay`. Restore: the marker and cowsay are gone, the `~` file is kept and
  `/home`'s mode is unchanged, `dpkg --audit` is clean, `swapon --show` shows cryptswap, `getcap
  /usr/bin/ping` is right (new-format snapshot), the journal shows each step, and the safety
  snapshot is listed.
- 2. **Kernel rollback** (the central risk): snapshot, kernel update, restart on the new kernel,
  restore. `uname -r` is the snapshot's kernel, the ESP hashes match `/boot`, and the new
  kernel's modules are gone.
- 3. Restore home too: the `~` file is gone, and it's in the safety snapshot.
- 4. Stop during the safety snapshot; Cancel at the ready prompt; after a cancel, a normal
  restart doesn't restore. Fill the system disk between ready and Restart now: refused with
  the one line.
- 5. Power cut at about 30% on the boot screen: attempt 2 finishes it.
- 6. Backup disk unplugged before Restart now's reboot: never started, normal boot, the
  message after login. Unplugged at about 30%: copy broke. Replugged: attempt 2 finishes.
  Left out: "didn't finish", and Restore again finishes it.
- 7. Boot files failed, simulated by the owner (e.g. a `kernelstub` that exits 1 on the test
  machine, put back afterwards): the ESP files are back, the newer kernel boots, `boot-kept` shown.
- 8. Recovery drill: from the recovery partition, the README steps with the safety snapshot.
- 9. Snapshot with Apsis 0.4.x: the dialog line; afterwards 0.4.x runs with the protected
  config; reinstalling 0.5 shows the result.
- 10. `pkaction --verbose --action-id io.github.atraxsrc.Apsis.restore`: `auth_admin`.

### 6b.13 Order of work (after the final look)

0. **0.4.1** (above), released on its own, with its apsis-test checks.
1. **Core** (no root): filter and protect list, refusals, home and format detection, Apsis
   version reading, space parsing, plan and state files, argv, ESP backup and check, the apply
   state machine with a fake runner, real-rsync temp-tree tests.
2. The owner runs checks 0.1 to 0.3 and sets up the baseline. Revise here if 0.1 or 0.3
   surprise.
3. **Helper**: methods, polkit action, the unit, `--apply-restore`, plymouth, logind reboot,
   journal. Also decide how the list carries a snapshot's format for the row tooltip (moved
   here from 0.4.1; step 4 builds the tooltip).
4. **UI**, rebuilt from the preview (6b.8) in the real code: the toolbar's Restore (no key, no
   tooltip), the dialog, refusals, preparing status, ready prompt, results in the status line. The preview branch stays unmerged.
5. Docs: README (experimental; "If a restore goes wrong"; how it differs from Timeshift, never
   "restores like Timeshift"), man page (the key), UI.md, ARCHITECTURE.md, CHANGELOG,
   DECISIONS.md. Then the owner's checks 1 to 10. Version 0.5.0.

Each slice ends with `cargo test --workspace`, clippy `-D warnings`, `cargo fmt` and a summary.

### 6b.14 Answers (owner, 2026-09-30) and what's left

Answers: 1 next-boot apply, if check 0.3 passes. 2 one dialog (date, name, comment, one summary
line), no typing the name. 3 safety snapshot on by default; space checked before the restart,
one-line refusal. 4 home kept by default, "keep" = `/home` fully excluded. 5 Enter and
double-click unchanged; restore gets its own key. 6 Pop!_OS with kernelstub only; the snapshot's
root UUID must be the current one. 7 live `fstab` and `crypttab` kept; `/etc/apsis/` protected.
Additions a to f are in 6b.2, 6b.10, 6b.6, 0.4.1, 6b.12 and DECISIONS.md.

UI review (owner, 2026-09-30, from the preview): no details area (old format in the dialog and
the row tooltip); one Restore, no choice step; no key for Restore (click or Tab), and
0.4.0's "keys only in README and man page" unchanged; armed
only at Restart now, no "Restart later" and no waiting state, closing the pop-up counts as Cancel
restore and cleans up; dialogs open with libcosmic's default focus (none), so Enter does nothing
until Tab; results in the window's status line only (no notification, no panel change);
boot-kept warning and failed destructive on the phrase only, done plain; Restore again opens
the normal dialog with normal defaults; the refusal's reason in the error colour. All in 6b.5, 6b.8, 6b.10 and DECISIONS.md.

Nothing open.

## Polish

- ~~Browser: space marks and moves the cursor down, so the details pane shows the next row and
  the mark looks like it failed. Keep the cursor on the marked row (or use a separate
  mark-and-move key), highlight marked rows more clearly, and show `marked` in the details
  pane.~~ - done 2026-09-26: space marks in place, `J` marks and moves down, marked rows get
  a faint accent background, details show `restore   marked`.
- ~~`just vendor` deletes `vendor/` without writing `vendor.tar`, so `just build-vendored`
  fails.~~ - done 2026-09-26: it writes `vendor.tar` (`vendor/` with versioned folders, plus
  `.cargo/config.toml`), and `build-vendored` unpacks it, builds `--frozen`, and removes
  `vendor/` and `.cargo/` again, even on failure.
- Bulk delete in the snapshot list, like Timeshift: space marks or unmarks in place, `J` marks and
  moves down (as in the browser); marked rows get an accent `*` and a faint accent background, the
  title `· N marked`. `d` with marks asks once, listing the names (`y` deletes); without marks it's
  the single delete. One password prompt through the helper (`auth_admin_keep`), then one
  snapshot at a time, `deleting 2/4: <label>…` in the activity pane, list refreshed after. Stops at
  the first failure and says which were deleted and which weren't. Refused while busy; Esc clears
  the marks. - done 2026-09-27, tested by the user. Afterwards, at the user's request: delete
  prompts, progress and results name snapshots as `09-27 09:12 "comment"` (the date only without
  a comment) instead of the raw name; the details pane and journal keep the raw name.
- Right-click menu: add `Close` (closes the menu, like Esc), and rename `Panel settings…` to
  `Remove or move applet…` (still opens `cosmic-settings panel`). No Quit: the panel owns the
  process. - done 2026-09-27, tested by the user.
- Parser: `Ret=NNN` lines are diagnostics like `E:`/`W:` (a list warning, not `not a snapshot
  row`), and part of a failure's output. Fixture `list-rsync-stale-mount-ret.txt`. - done
  2026-09-27.
- Docs + small UI (2026-09-27, not committed yet): README "Settings explained", man page
  `docs/apsis.1` installed gzipped by `just install` and the .deb (clears lintian's
  `no-manual-page`), settings details that wrap and are never cut off in the popup or a narrow
  window, with a line per home option; the popup keeps one height in the settings view
  (sized to the tallest row's details). The layout test runs only with `APSIS_LAYOUT_TEST=1`.

- Disk usage line (superfile "Processes"-style) under the panes: `disk  sdX1  ████░░░░  448G
  used · 483G free · 9 snapshots`, accent / warning (< 10% free) / destructive (< 5%) from the
  theme, bar width from the space left. The helper's new `ListWithUsage` and
  `NativeListWithUsage` add `statvfs` numbers (`List`/`NativeList` unchanged); Timeshift's
  `X GB free` line is the free-only fallback, also for pkexec. Unknown: no line. Tooltip adds
  `· 483G free`. Refreshed after create, delete, bulk delete and restore. - done 2026-09-28,
  tested by the user. Then (same round): when no mount of the disk exists, a brief read-only
  mount for the `statvfs`; the free line only if that fails.
- Progress and time left for create (Timeshift and native) and real restores:
  `creating ██████░░░░ 58% ~3 min left` in the activity pane, from the helper's new
  `Progress` signal (Timeshift's `% complete` line, rsync `--info=progress2`); `estimating…`
  until there's a number; a bare spinner through pkexec or an older helper. - done 2026-09-28,
  not committed, waiting for the user's test.
- `just deb-install` for testing on the user's machine (builds the .deb, reinstalls it with
  apt, stops the helper). - done 2026-09-28.

## 0.2.0 Standalone (design approved 2026-09-29, built, tested by the user)

- **Status: built 2026-09-29; the user's 8 real-machine checks passed the same day (old schedule
  removed, install, import and `w`, list/create at idle priority/delete/bulk delete with
  Timeshift not running, tag links, real bind and loop mounts inside a snapshot, keep-last-N,
  restore, disk unplugged). Release 0.2.0 prepared, not committed.** Approved with decisions 1-5 as recommended and
  the user's delete conditions (see DECISIONS.md). Differences from the text below, found while
  building:
  - **config.toml has one ordered `filters` list** (Timeshift's format, home patterns inside)
    instead of `exclude` + a `[home]` table: rsync takes the first matching filter, so splitting
    the home patterns out would reorder them against the other filters and change what a
    snapshot holds. The settings view still shows one home row per user, read from the list.
  - `WriteConfig` takes the config as `(sas)` (device UUID, filters), not TOML text.
  - Delete also refuses a snapshot with anything mounted inside it (`/proc/self/mountinfo`),
    before deleting anything: a bind mount has the same device number, so the no-follow,
    same-filesystem walk alone would empty it.

**Direction (the user's, 2026-09-29):** Apsis becomes a simple, standalone snapshot and restore
program. Manual only: no scheduling, no automatic retention; a snapshot is deleted only when
the user deletes it (one, marked ones, or the keep-last-N preview after `y`). The native rsync
backend is the only backend; nothing runs `timeshift`. The Phase 5.2 schedule is parked on
branch `phase-5.2-schedule` (`a2a0629`, not merged, not pushed). No new features: this removes
code and moves what stays onto Apsis's own config.

### What stays on disk

The backup device keeps Timeshift's rsync layout, unchanged (Phase 5): `timeshift/snapshots/
<name>/{localhost/,info.json,exclude.list,rsync-log}`, the `snapshots-<tag>/` links, and
Apsis's `timeshift/apsis-staging/`. Existing snapshots (Timeshift's and native ones) keep
listing, restoring and deleting. New snapshots stay tagged `ondemand`, so an installed Timeshift
still reads them, and its own retention never removes them (it never removes `ondemand`).

### Config: `/etc/apsis/config.toml`

Root-owned, 0644, written only by `apsis-helper` (atomically, temp + rename, `.bak` kept, like
the Phase 4.5 writer), with polkit `configure`. The TOML crate already in `Cargo.lock`
(`toml` 0.5.11) reads and writes it; no new download.

```toml
# Written by apsis-helper; change it in Apsis's settings.
version = 1
backup_device_uuid = "8cecb045-975d-49d0-bd57-1ec9f6eb77b5"
exclude = ["/var/lib/libvirt/**", "*.mp3"]

[home]        # per user: "excluded" (default), "hidden" (dot-files only), "all"
root = "excluded"
user1 = "all"
```

- A user not listed is `excluded`, as Timeshift's default (`Main.vala:751-781`). ecryptfs homes
  are always excluded (Timeshift's own patterns for them stay in the exclude builder).
- The rsync filters are built exactly as today: `exclude` plus Timeshift's home patterns
  (`<home>/**`, `+ <home>/.**`, `+ <home>/**`) go in as the "user filters" of
  `native::exclude::for_backup`, which adds Timeshift's defaults, fstab mounts and so on. So a
  snapshot made after the switch excludes exactly what one made before it did, and
  `--link-dest` keeps hard-linking.
- Checks before writing (as now): the device connected, unencrypted, a Linux filesystem; filters
  not blank, no control characters, no duplicates; home values one of the three.

**Import, once.** While `config.toml` doesn't exist and `/etc/timeshift/timeshift.json` does,
`ReadConfig` returns a config built from it (read only): `backup_device_uuid`, the `exclude`
list split into home modes (the existing `HomeState` reading) and other filters. The settings
view says so (`imported from Timeshift, w saves it`) and the activity pane lists what was taken
and what wasn't:

```
imported from /etc/timeshift/timeshift.json (not saved yet; w saves):
  device   8cecb045… (sda1, ext4)
  home     user1: everything, root: excluded
  filters  4
  not used schedule and counts (Apsis doesn't schedule), btrfs mode
```

List and create use that imported config until it's saved (so the first run works without a
`w`); once `config.toml` exists, `timeshift.json` is never read again. Timeshift in btrfs mode:
the device is still imported, with a note that btrfs snapshots aren't shown. No `timeshift.json`
and no config: `no backup device: pick one in settings [s]`.

### Backend

- **One backend, `native::NativeRsync`,** through the helper only. Removed: `TimeshiftCli`, the
  `--list` parser, `PkexecRunner` and the pkexec fallback (they run `timeshift`), and the
  applet's backend choice. Without `apsis-helper` the popup says `Apsis needs apsis-helper
  (install the .deb)` and does nothing else.
- **List** (with the disk bar from `statvfs` while mounted), **create** (low priority, already on
  main since `90272f5`: `ionice -c 3 nice -n <to 19>` around rsync; progress from
  `--info=progress2`), **delete** and bulk delete, **browse/restore** (6a): as now.
- **Delete becomes native** (it went through Timeshift). Recommended: bring
  `native::prune::remove_tree` over from the parked branch: it walks with
  `openat(O_NOFOLLOW | O_DIRECTORY)` and stops at another filesystem, where the current native
  `delete` uses `std::fs::remove_dir_all`, which doesn't follow symlinks but does empty a mount
  inside the tree. Only `snapshots/<name>` with a valid name, a real folder (not a symlink), and
  a name the fresh list has.
- **Timeshift's lock is no longer checked** (as asked). Consequence: if Timeshift is still
  installed and runs on the same disk at the same moment, both write there. Apsis builds in
  `apsis-staging/` and only renames into `snapshots/`, and Timeshift never removes `ondemand`
  snapshots, so neither loses the other's snapshots; they'd only compete for disk and I/O.
- **Keep last N** (main's `retention::manual`, opt-in, preview + `y`): the Timeshift rule goes
  ("a snapshot with another tag is left to Timeshift's retention"). N counts every uncommented
  snapshot in the list, whatever its tags; commented ones stay pinned and uncounted; the newest
  is never deleted. `Kept::OtherTags` is removed. **Remind** stays as it is.

### Helper methods (interface stays `io.github.atraxsrc.Apsis.Helper1`)

| method | now | 0.2.0 |
|---|---|---|
| `List`, `ListWithUsage` | `timeshift --list` | **removed** |
| `Create(s)` | `timeshift --create` | **removed** |
| `NativeList` | native, no usage | **removed** (the applet calls the `WithUsage` form) |
| `NativeListWithUsage` | native + `statvfs` | kept |
| `NativeCreate(s)` | native, low priority | kept |
| `NativeDryRun(s)` | native plan | **removed** with the dry-run setting (decision 2) |
| `Delete(s)` | `timeshift --delete` | **changed**: native delete (safe walk), same signature and polkit `delete`, `Finished("delete", ..)` as now |
| `ReadSettings`, `WriteSettings` | `timeshift.json` | **removed** |
| `ReadConfig() -> (s, s, a(ssb), as)` | - | **replaces ReadSettings**: `(config.toml text or the import, lsblk JSON, users, import notes)`; empty notes = nothing imported. polkit `list` |
| `WriteConfig(s expected, s config) -> s` | - | **replaces WriteSettings**: writes `config.toml` if it still reads `expected` (`""` = doesn't exist yet), after the checks above. polkit `configure` |
| `Browse`, `Restore`, `Progress`, `Finished` | | kept |

An older applet still running after the upgrade gets `UnknownMethod` for `List`/`ReadSettings`
and shows an error until it's re-added to the panel (`just deb-install` already says so).

**polkit actions: all seven kept, none added or changed** (`list`, `create`, `delete`,
`configure`, `browse`, `restore`, `restore-original`). `create` now only covers `NativeCreate`,
`configure` only `WriteConfig`.

### Applet

- **Settings view**: `device`, `home` (one row per user), `filters` (+ add), then Apsis's own
  `keep manual` and `remind`. Removed: `mode` (rsync/btrfs), `@home`, the five schedule rows and
  counts, `backend`, `dry run`. `w` writes `config.toml`; `keep manual` and `remind` are still
  saved at once (cosmic-config). The "close Timeshift's own window first" warning goes.
- `[c]` always creates for real (after the comment prompt), with progress.
- Errors: Timeshift's `E:` lines, `Ret=`, "timeshift not installed", the `timeshift --list`
  spinner text and the `Failed { code, output }` mapping go; native errors show as they are.
- Header: `rsync · 3 snapshots` (no `native ·`).
- cosmic-config: `native_backend` and `native_dry_run` are no longer read (left in old configs,
  harmless).

### Files and tests

Removed:
- `apsis-core/src/timeshift.rs` (`TimeshiftCli`, `Runner` moves to `native/runner.rs` or
  `lib.rs`), `parse.rs`, `pkexec.rs`; in `settings.rs` everything that edits `timeshift.json`
  (`Config::edit`, `write`, the string-field rules); in `progress.rs` Timeshift's `% complete`
  parser; in `usage.rs` Timeshift's `X GB free` line; in `error.rs` `UnrecognisedOutput`,
  `NotInstalled`, `Failed { code, output }`, `NoSnapshotDevice` (list-first rule),
  `SettingsChanged` becomes `ConfigChanged`.
- `apsis-helper`: `state.rs`'s `TimeshiftCli` (the lock and idle exit stay), `settings.rs`'s
  `Files` writer for `timeshift.json` (the atomic writer stays, for `config.toml`), `native.rs`'s
  `TIMESHIFT_LOCK`/`timeshift_running`, `usage.rs`'s `ListWithUsage` path.
- Tests: `tests/parse_list.rs`, `tests/timeshift_cli.rs`, `tests/pkexec.rs`, the Timeshift
  cases in `tests/progress.rs` and `tests/settings.rs`; fixtures `list-*.txt` (6) and
  `create-rsync-progress.txt`. The applet's Timeshift/pkexec tests.
- `docs/TIMESHIFT-CLI.md` (history keeps it; Phase 5's references to Timeshift's source stay in
  DECISIONS.md).

Added or changed:
- `apsis-core/src/config.rs`: `Config` (TOML read/write, checks), `import(timeshift_json,
  users) -> (Config, notes)`, `user_filters()` for the exclude builder. Tests: round trip,
  defaults, bad values, import from the real `config-rsync.json` fixture (kept) giving the same
  rsync filter list as today (`exclude.list` fixture), btrfs note.
- `native/prune.rs` from the branch (with its tests), `Backend::delete` using it.
- `retention::manual` without the tag rule (tests updated).
- Helper: `ReadConfig`/`WriteConfig`, native `Delete`; interface test counts 7 methods.

### .deb and install

- `recommends = "timeshift"` goes (it was a Recommends, not a Depends); `rsync`, `dbus`,
  `polkitd` stay. No maintainer script changes; no files added or removed from the package
  (the helper, its unit, bus files and policy stay).
- `postrm purge` removes `/etc/apsis/` (Apsis's config) - new.

### Root-level and destructive, for review

- **Delete is Apsis's own, as root**: `rm` of `snapshots/<name>` through the safe walk, for
  single, bulk and keep-last-N deletes. Timeshift did this before.
- **New root-owned file** `/etc/apsis/config.toml` (+ `.bak`), written by the helper; removed on
  purge.
- **Timeshift's lock is ignored** (see Backend).
- **`timeshift.json` is only read, once, for the import**; Apsis never writes it again.
- **Dropping `Recommends: timeshift`**: if apt installed Timeshift automatically because of it,
  `apt autoremove` would later offer to remove Timeshift (the package; not its snapshots or
  settings). Check with `apt-mark showauto | grep timeshift`; `sudo apt-mark manual timeshift`
  keeps it.

### Decisions for you

1. Delete through `remove_tree` from the parked branch (recommended), or keep
   `remove_dir_all`?
2. Drop the dry-run setting and `NativeDryRun` (recommended: with one backend, a default that
   makes `c` not create is confusing), or keep it (off by default)?
3. Keep last N: count every uncommented snapshot regardless of tags (recommended), or only
   `ondemand` ones? Snapshots of another system on the same disk would count too (the list
   doesn't tell systems apart; rare).
4. Restores at idle priority too, or only creates (recommended: a restore is something the
   user waits for)?
5. Home mode per user as now (recommended; it's what's already built and imported), or one
   mode for all users?

### Done when

`cargo test`, clippy, fmt; `just test-ext4` (native, restore); on the user's machine: first run
shows the import and `w` saves `/etc/apsis/config.toml`; list, create (idle, progress), delete,
bulk delete, keep-last-N and restore work with Timeshift uninstalled (or at least never run: a
check that nothing executes `timeshift`); existing Timeshift snapshots still list and restore.
Then README, man page and metainfo for 0.2.0.

## Phase 7 — Release

- README screenshots, metainfo, `just vendor` tarball, tag `v0.1.0` (user pushes).
- Draft the cosmic-project-collection entry for `applets.ron`; the user opens the PR.
- **Status: prep done 2026-09-26** (libcosmic pinned by `Cargo.lock`, CI, SECURITY.md, README, metainfo,
  CHANGELOG, collection drafts). Left for the user: see the checklist in `docs/RELEASE.md`
  (vendor tarball, tag, release, collection PR).

## UI polish (contract: `docs/APSIS-UI-PROMPT.md`; decisions in `DECISIONS.md`, 2026-09-29)

The panel popup is a read-only overview; the window does the work. One slice at a time.

1. **Status model** - done: `ApsisStatus`, panel tooltip (`last`, `next  manual only`, `disk`),
   icon colour, optional panel label.
2. Strip in the window (time | disk), from `StatusView` - done.
3. Activity bar in the window: percent and time left - done.
4. Popup becomes the read-only overview, plus `open apsis` - done.
5. Window chrome: the four rooms; create beside the list - done.
6. Copy pass, then theme audit (no hardcoded colours) - done 2026-09-29, see `DECISIONS.md`.

Roadmap, not scheduled: a scheduler (`next` in time, the last/next timeline), undock.

## 0.4.0 - The Timeshift model (released 2026-09-30)

**Status: released as v0.4.0 (2026-09-30).** 0.4.0 works on apsis-test; edge-case checks 3-6
not run, left to issue reports.

**Product (owner, 2026-09-30):** Apsis does exactly this, nothing else:
1. snapshot the system; two include choices, `/root` and `/home` (either, both, neither); the
   system is always included;
2. a simple include/exclude filter list (like Timeshift's Filters tab);
3. delete snapshots;
4. restore the whole system (0.5.0, Phase 6b).

**UI direction (owner, 2026-09-30; overrides the earlier UI decisions and the old
`APSIS-UI-PROMPT.md`):** take Timeshift's UI and ease of use over to Apsis. Start simple; add
features only when users ask. Standard libcosmic widgets and COSMIC theme colours only; no
terminal styling, no key-hint footer, no dock or rooms, no prompt line. Keyboard shortcuts may
keep working but are documented only in the man page and README. Kept: the panel applet
(read-only popup and tooltip) and the single window. The new contract is
`docs/APSIS-UI-PROMPT.md` (rewritten short; the old one is in git history).

### Order of work

One slice at a time; each ends with `cargo test --workspace`, clippy `-D warnings`, `cargo
fmt` and a summary. No commits (the owner commits).

1. **Removals:** H; A (file-level restore); keep-last-N and its pruning; the window's rooms,
   dock, schedule and log rooms. The terminal look stays until slice 3 replaces it, so the app
   builds and runs between slices.
   **Done 2026-09-30** (not committed): tests, clippy and fmt pass; see DECISIONS.md.
2. **Helper:** config v2 and the converter (B); `Helper2` with clean names; `Job` and
   `JobChanged` (D); `Stop` (E); leftovers, including delete of a leftover (F);
   `DeviceRemoved` (G); the polkit file (I). The applet is moved onto `Helper2` in the same
   slice, keeping its current look, so it keeps working.
   **Done 2026-09-30** (not committed); see DECISIONS.md.
3. **UI**, in three parts: (a) the main window (toolbar, list, status area, dialogs, stop);
   (b) settings (tabs); (c) the panel popup in standard widgets, with the running job.
   **Done 2026-09-30** (not committed); see DECISIONS.md.
4. Docs, version 0.4.0, CHANGELOG, then the owner's checks on the HP.
   **Done 2026-09-30**, released as v0.4.0 with new screenshots (`docs/1.png` to `4.png`).
   0.4.0 works on apsis-test; edge-case checks 3-6 not run, left to issue reports.

### A. File-level restore goes

Removed, whole:
- `apsis-core`: `restore/` (`mod`, `browse`, `dest`, `path`, `plan`, `runner`), `tests/restore.rs`,
  `Error::Restore`, `RESTORE_HEADER`, `WireListing`/`listing_to_wire`, the client's browse and
  restore calls, `OP_RESTORE`, `METHOD_BROWSE`, `METHOD_RESTORE`, the three restore action
  names, `Progress` part-splitting for multi-path restores. From 0.3.1:
  `Listing::is_excluded_home` (the browser's "home not included" message) and
  `Restored::from_result`.
- `apsis-helper`: `restore.rs`, the `Browse` and `Restore` methods, `unix_user` (only restore
  used it), `native::mount_backup` (only restore used it), `logged_path(s)`.
- Applet: `browser.rs`, the browser, plan and result views, `R`, the 0.3.1 `[o]pen folder`
  key and `xdg-open` call for it, the `restoring` progress label, their i18n strings. Enter and
  double-click on a snapshot open nothing until 0.5.0 (restore).
- polkit: `browse`, `restore`, `restore-original`.
- `justfile`: `test-ext4` runs `--test native` only.
- **Keep last N** (answer 5): `apsis_core::retention` and its tests, the prune preview, `p`,
  the settings row, the automatic preview after a create. Its cosmic-config field is no longer
  read (left in old configs, harmless). The **reminder** and the optional **panel label** stay.
- **Window chrome** (answer to the UI direction): rooms and the dock (`Room`, `1 2 3 4`), the
  schedule and log rooms, the apsis strip pane, the details pane, the prompt line, the key-hint
  footer, the help overlay, the pane widget and the settings view's layout test helper
  (`tallest.rs`). Removed in slice 1 where they're self-contained (rooms, dock, schedule, log,
  help, keep-N); the rest is replaced in slice 3.
- Docs: UI.md's browser section, ARCHITECTURE.md's restore section, README, man page,
  metainfo, desktop `Comment=`, crate descriptions, the .deb's extended description.

Left alone on users' machines: anything already in `~/Apsis-restored/` and any
`*.apsis-before-<snapshot>` file next to an original. They're the user's files now; the
CHANGELOG and README say they can be deleted by hand. Apsis never touches them again.

### B. Settings

#### Config v2 (`/etc/apsis/config.toml`)

```toml
# Written by apsis-helper; change it in Apsis's settings.
# filters: first match wins, top to bottom; "- " leaves out, "+ " keeps.
# Built-in excludes (/proc, /dev, caches, ...) always come first.
version = 2
backup_device_uuid = "8cecb045-975d-49d0-bd57-1ec9f6eb77b5"
include_root = true
include_home = true
filters = [
    "+ /home/user1/Videos/keep/***",
    "- /home/*/Videos/***",
    "- *.iso",
]
```

Changes from v1:
- `version = 2`.
- New `include_root`, `include_home` (booleans).
- `filters`: every entry starts with `+ ` or `- ` (v1 had a bare pattern for an exclude).
  Explicit signs also fix a v1 trap: rsync reads a bare line starting with `#` or `;` in an
  exclude file as a comment.
- No per-user home patterns: those become the two booleans or plain filter rows (below).
- `backup_device_uuid` unchanged.

Checks (core `config::validate`, again in the helper): the device rule as today; each filter
`+ ` or `- ` then a body that isn't blank, no control characters, at most 4096 bytes, not
repeated. No semantic checks (Timeshift has none either).

Wire: `(sbbas)` = (device UUID, include_root, include_home, filters).

#### Loading old configs

- `Config::parse` reads version 2 as is. **Version 1** (0.2.0 to 0.3.x) and the **Timeshift
  import** (`timeshift.json`'s `exclude`, which is the v1 format) go through one converter,
  `config::from_v1(filters, users, home_entries) -> (Config, notes)`. The result is used for
  list and create at once (like today's import), shown in settings as unsaved, and written only
  on `w`; the written file's `.bak` then holds the v1 text.
- **The conversion doesn't change what a snapshot holds.** It simplifies only when it can prove
  the result is the same, else it keeps explicit rows. One exception, from the builder's
  parent-folder lines (below): a `+` filter that a later `-` filter used to hide (so it did
  nothing) starts to work. Timeshift's editor appends, so its lists rarely have that; the
  CHANGELOG says so. The rules:
  1. Every bare pattern gets `- `; `+ ` patterns stay; order is kept.
  2. `/root`: root's home state (Timeshift's reading, as today) `everything` -> `include_root
     = true`, its pattern dropped. `excluded` -> `false`, dropped (the built-in `/root/**`
     covers it). `hidden files only` -> `false`, and `+ /root/.**` stays as a row in place.
  3. `/home`: `include_home = true`, all home patterns dropped, only if (a) every non-system
     user with a home under `/home/` is `everything`, (b) every folder in `/home` except
     `lost+found` and `.ecryptfs` is one of those homes (the helper reads `/home`; a shared
     `/home/data` was left out before and would come in), and (c) no filter after a dropped
     `+ <home>/**` could match inside that home (anything not anchored with `/`, or anchored
     under that home). Otherwise `include_home = false` and each include stays as a row in
     place: `+ /home/user1/**`, `+ /home/user2/.**` (this is how **hidden files only**
     migrates: it becomes a visible `+ <home>/.**` row the user can keep or remove). A plain
     `/home/<user>/**` exclude is dropped (the built-in `/home/*/**` covers it).
  4. ecryptfs users: their `/home/.ecryptfs/<name>/***` patterns follow rule 3 as if they were
     the home. Decrypted contents stay out, as today (the builder adds those first).
  5. A user whose home is outside `/root` and `/home` and was `excluded` gets `- <home>/**`,
     so it stays out (0.4.0 has no per-user step; such a home is otherwise system files).

  The converter needs the users (`/etc/passwd`) and the entries of `/home`, so the helper
  passes them wherever it loads the config (`native::open` passes no users today, since they
  only fed the notes).
- Notes, shown at the top of the settings page (a dimmed box) until saved:

  ```
  settings from Apsis 0.3 (not saved yet; w saves)
    /root   excluded
    /home   included (every home was "everything")
    kept as filters  + /home/user2/.**  (user2: hidden files only)
  ```

- Existing snapshots: untouched. They list, delete and `--link-dest` as before.
- Downgrade: 0.3.x refuses a v2 file (`version 2 (this Apsis reads 1)`). Documented fix in
  the CHANGELOG, for the user to run: `sudo cp /etc/apsis/config.toml.bak
  /etc/apsis/config.toml`.
- Fresh config (no `config.toml`, no `timeshift.json`): device none, filters empty, include
  defaults `/root` on, `/home` off (answer 1).
- `ReadConfig` no longer sends the users list (the UI has no home rows): `(s(sbbas)sas)` =
  (file text, config in effect, lsblk JSON, notes).

#### rsync filter list (`native::exclude::for_backup`)

Same Timeshift 24.01.1 order, with the user part replaced:

1. Built-in defaults, fstab mounts, default extras (unchanged).
2. Decrypted ecryptfs contents (unchanged).
3. The user's filters, in order, as written (`+ x` / `- x`). For each `+` filter that is an
   absolute path, its parent folders are added just before it as `+ <dir>/` (folder only, not
   contents), so rsync reaches a kept folder inside a left-out one (`+
   /home/user1/Videos/keep/***` with `/home` off needs `+ /home/user1/Videos/` first; rsync
   never looks inside an excluded folder). Only in `exclude.list`, not in the config. Not
   Timeshift's behaviour; noted in DECISIONS.
4. `+ /root/**` if `include_root`, `+ /home/**` if `include_home`.
5. The built-in `/root/**`, `/home/*/**`, then `/timeshift/*` (unchanged).

Removed: the per-user "exclude homes without an include" step (steps 4 and 5 cover it; see
rule 5 above for homes elsewhere). So a user's `- /home/*/Videos/***` beats `/home` on, and a
user's `+ ...` beats `/home` off; a `+` can't bring back a built-in exclude (they come first).

#### Settings (Timeshift's tabs)

A page in the same window (single window): the Settings toolbar button opens it, a back
button returns to the list. Three tabs (libcosmic `tab_bar` / segmented button): **Location**,
**Include**, **Filters**. Changes are kept until **Save** (the helper writes `config.toml`,
polkit `configure`, one password, cached); **Cancel** drops them. Leaving with unsaved
changes asks once (Save / Discard / Cancel dialog).

```
┌ Apsis ───────────────────────────────────────────────────── ✕ ┐
│ ← Settings                                     [Cancel] [Save]│
│  ( Location )   Include    Filters                            │
│ ───────────────────────────────────────────────────────────── │
│  Select the disk where snapshots are saved.                   │
│                                                               │
│  ◉  sdb1   ext4    931.5 GB   Backup                          │
│  ○  sdc1   ext4    120.0 GB   USB                             │
│  ○  sda3   btrfs   476.9 GB   (system disk)                   │
│     sdd1   ntfs     64.0 GB   can't hold snapshots (greyed)   │
│                                                               │
│  Snapshots are saved in timeshift/snapshots on this disk.     │
└───────────────────────────────────────────────────────────────┘

│  Location   ( Include )   Filters                             │
│  The system is always included.                               │
│                                                               │
│  [✓] /root   the root user's home folder                      │
│  [ ] /home   every user's home folder                         │
│                                                               │
│  Home folders hold documents and settings. A full restore     │
│  puts included ones back as they were in the snapshot.        │

│  Location    Include   ( Filters )                            │
│  ┌─────┬────────────────────────────────────────────────┐     │
│  │  +  │ /home/user1/Videos/keep/***                    │ ◀ selected
│  │  -  │ /home/*/Videos/***                             │     │
│  │  -  │ *.iso                                          │     │
│  └─────┴────────────────────────────────────────────────┘     │
│  [Add Folder] [Add File] [Add Pattern] [Remove] [Move Up] [Move Down] │
│  + includes, - excludes. The first matching line wins, top    │
│  to bottom. Built-in excludes (/proc, /dev, caches) come      │
│  first.                                                       │
```

- Location: a radio list of devices from `ReadConfig`'s lsblk (as the 0.3 device row, now
  all at once); unusable ones shown dimmed with the reason, not selectable. The saved device
  can be unplugged and still shown (`not connected`).
- Include: two libcosmic checkboxes (standard widgets are allowed now). Fresh installs:
  `/root` on, `/home` off (answer 1).
- Filters: a list; each row a `+`/`-` toggle (a small segmented button or dropdown, `+ Include`
  / `- Exclude`) and the pattern. Buttons: **Add Folder**, **Add File** (the xdg portal picker,
  libcosmic `dialog::file_chooser`, as the user), **Add Pattern** (a dialog with a text field
  and the +/- choice), **Remove**, **Move Up**, **Move Down** (answer 6; the last four act on
  the selected row, dimmed without one). New rows go at the top and are selected.
- Picker patterns as designed before: folder `<path>/***`, file `<path>`, rsync wildcards
  (`*`, `?`, `[`, `\`) backslash-escaped; sign `-`, except `+` inside `/root` or `/home` while
  that include is off; `/run/user/<uid>/doc/` paths refused. `/root` can't be browsed by the
  user; its paths are typed with Add Pattern.
- A bad pattern: the Add Pattern dialog stays open with the reason under the field.
- The reminder and panel label (kept, answer 5): a fourth tab, **Misc** (open question 1).
- Removed: per-user home rows, `HomeState` cycling, `is_home_pattern`, the users in
  `ReadConfig`, `w`/`r`/`space` as the way to edit.

### Main window (Timeshift's)

```
┌ Apsis ─────────────────────────────────────────────────── ☰  ✕ ┐
│ [+ Create]  [↺ Restore]  [🗑 Delete]  [⚙ Settings]              │
├────────────────────────────────────────────────────────────────┤
│  Snapshot             Comment                                  │
│  2026-09-30 14:02     before driver update                     │
│▌ 2026-09-29 06:12                                              │ ◀ selected
│  2026-09-28 18:01     pre-nvidia                               │
│  2026-09-27 14:02     Interrupted snapshot, removed by the     │ (dimmed)
│                       next one                                 │
├────────────────────────────────────────────────────────────────┤
│  Last snapshot   12 hours ago                                  │
│  Backup disk     sdb1   372 GB used · 224 GB free              │
│                  ████████████████████░░░░░░░░░░░░              │
└────────────────────────────────────────────────────────────────┘

while a job runs, the status area:
├────────────────────────────────────────────────────────────────┤
│  Creating snapshot · 58% · 3 min left                  [ Stop ] │
│  ███████████████████████░░░░░░░░░░░░░░░░░                       │
└────────────────────────────────────────────────────────────────┘
```

(The toolbar icons are the theme's symbolic icons, not characters; the mock only marks them.)

- **Toolbar:** labelled buttons with symbolic icons, in a libcosmic header or a row under it.
  **Create** is enabled when a backup device is set and connected and no job runs. **Delete**
  needs a selection and no job. **Settings** always. **Restore** isn't shown until 0.5.0 (open question 2). The header's menu (☰) has Refresh and About Apsis (libcosmic's about widget).
- **List:** columns Snapshot (date and time, local) and Comment (cut with `…`, full in the
  tooltip). Newest first. Click selects; Ctrl/Shift-click selects several (multi-select
  replaces space/`J` marks). Double-click does nothing in 0.4.0 (0.5.0: restore). libcosmic's
  `table` widget if the pinned revision has it, else a column of selectable rows; checked at
  the start of slice 3a. States in the list area: loading (spinner), empty ("No snapshots
  yet. Click Create to make one."), no device ("No backup disk chosen. Choose one in
  Settings." with a button), disk not connected, no helper, error (the reason and a Retry
  button).
- **Leftover rows** (answer 2): dimmed, one per leftover, "Interrupted snapshot, removed by the
  next one" and the time it started. Selectable; Delete removes it through the normal delete
  path (F).
- **Create:** a dialog, "Create snapshot", one optional Comment field, a dimmed line with what
  it includes (C), [Cancel] [Create]. Then polkit asks for the password.
- **Delete:** a dialog, "Delete 2 snapshots?", their labels (`09-30 14:02 "before driver
  update"`), "This can't be undone.", [Cancel] [Delete] (destructive button style). Several go
  one at a time through the helper, as the bulk delete does now; the status area says
  "Deleting 2 of 4…". It stops at the first failure and says which were deleted.
- **Status area** (bottom): idle, "Last snapshot" (age, with the reminder's warning colour and
  "(over 7 days)" when due) and "Backup disk" (device, used, free, a libcosmic progress bar;
  warning / destructive colour under 10% / 5% free). "Not connected" when the disk is gone.
  While a job runs (this window's or another's, D): its label, percent and time left, a
  progress bar, and **Stop** for a create (E). After a job: one line with the result ("Snapshot
  created", "Create stopped", "Create failed: backup disk removed", with the helper's reason in
  the tooltip) until the next action.
- **Keyboard** (man page and README only, not shown in the UI): Ctrl+N create, Delete delete,
  Ctrl+R / F5 refresh, Ctrl+, settings, Ctrl+A select all, Esc closes a dialog or the settings
  page. The single-letter commands (`c`, `d`, `x`, `s`, ...) go with the prompt line.
- Colours: theme only (accent for selection and bars as libcosmic draws them; warning and
  destructive roles for the reminder and low space). No hard-coded colours, no monospace.
- Window size: opens at about 720 x 520, minimum 640 x 440 as now; a layout test checks the
  toolbar, a few rows and the status area fit the minimum.

### C. What a snapshot includes (Create dialog)

The 0.3.1 `home  included / hidden files only / excluded` line becomes one dimmed line in the
Create dialog, from the config in effect (`ReadConfig` when the dialog opens):

```
Includes the system, /root and 3 filters.
Includes the system, /root, /home and 1 filter.
Includes the system only.
```

No line if `ReadConfig` fails. `HomeState::summary`, `ConfigInfo::home_state_of` and the
`$HOME` matching go.

### D. Job status

The helper keeps the current job next to its one lock, in `State` (one `Mutex<Job>` beside the
existing `running` flag; `State::begin(kind)` sets it, `Running`'s drop clears it). Nothing new
takes the lock.

- `Job() -> (sssxdx)`: `(kind, state, snapshot, started, percent, eta_seconds)`. `kind`:
  `create`, `delete`, `list`, `configure`, or `""` when idle. `state`: `running` or
  `stopping`. `snapshot`: the name being made or deleted (`""` until a create has planned
  it). `started`: Unix seconds. `percent` / `eta_seconds`: `-1` while unknown. polkit `list`,
  not interactive.
- `JobChanged((sssxdx))`: broadcast (no destination), on start, when the name is known, on
  each progress step (at most every 500 ms, the end always), on `stopping`, and once at the end
  with `state` = `done`, `failed` or `stopped` (then `Job()` says idle).
- Privacy: the broadcast carries no comment, caller, error text or path; any local process can
  see that a create runs, its snapshot name (a time) and its progress. The error text still
  goes only to the starter, in `Finished`.
- `Progress` is folded into `JobChanged` (answer 4): one path for the window that started
  the job and for everyone else. `Finished(s op, b ok, s message)` stays, sent only to the
  starter.
- Applet, window and popup alike: subscribe to `JobChanged` at start (a match rule; it doesn't
  start the helper). On open, ask `NameHasOwner` on the system bus first and call `Job()` only
  if the helper runs, so opening the popup never starts a root process just to hear "idle". A
  window opened mid-job shows it in the status area ("Creating snapshot · 42% · 3 min left",
  "Deleting 09-27 09:12…"), Create and Delete wait, and it lists when the job ends. The 0.3.1
  busy poll (list again every 3 s) is replaced by this; `kind` `list`/`configure` still reads
  `busy` (they last a second).
- The helper leaving the bus mid-job (crash): `NameOwnerChanged` ends it as `failed`, then a
  list.
- Popup: one line and a progress bar while a job runs or has just ended ("Creating snapshot ·
  42%", then "Snapshot created", "Create failed" or "Create stopped", dimmed) until it closes.
  Read-only: no Stop there; "Open Apsis" goes to the window, which has it.

### E. Stop (create only)

`Stop(s snapshot)`: stops the running create if its snapshot is `snapshot` (so a confirm typed
for one job can't stop the next). Refused: nothing running, not a create, another name, or too
late (see commit point). Returns once stopping has begun; the create's own `Finished` follows.

Core (`apsis-core`, no root, testable):
- `native::Cancel` (`Arc`), a small state machine: `Armed` -> `Stopping` or `Committed`.
  `request()` moves `Armed` to `Stopping`; `commit()` moves `Armed` to `Committed` right before
  the rename into `snapshots/`. Whichever comes first wins, so a stop can never race the
  rename: after `Committed`, `Stop` answers "finishing, can't be stopped now" and the snapshot
  is kept.
- `QuietRunner` starts rsync in its own process group (`CommandExt::process_group(0)`; ionice
  and nice exec into rsync, so the group is rsync and the children it forks) and registers the
  group with the `Cancel`.
- Kill: `SIGTERM` to the group (`rustix::process::kill_process_group`), then `SIGKILL` to the
  group after 10 s if the leader hasn't exited. The pid can't be reused under us: the waiting
  thread first waits with `waitid(WEXITED | WNOWAIT)` (leader exited but not reaped, so its
  pid and group id stay taken), marks the group dead under the `Cancel`'s lock, and only then
  reaps. The killer takes the same lock and skips a dead group. rustix 1.1 has both calls
  (checked); no new crate.
- `execute` checks the `Cancel` before rsync, after rsync, and commits before the rename. A
  stopped create returns `Error::Stopped`.
- Staging removal, for a stop and for every failed create (today it's `fs::remove_dir_all`):
  the delete's rules. The name is a snapshot name; `timeshift/`, `apsis-staging/` and `<name>/`
  opened with `openat(O_NOFOLLOW | O_DIRECTORY)` from the mount; nothing mounted at or below it
  (`mounts_under`); removed with `prune::remove_at` (no symlink followed, stops at another
  filesystem); then `apsis-staging/` itself if empty. Logged with the path.

Helper:
- The lock stays held from the stop request until the staging folder is gone; meanwhile
  `Job()` says `stopping`, every other call gets `Busy`.
- Journal: `stop "2026-09-30_14-02-11" for :1.42`, `SIGTERM to process group 12345`, `SIGKILL
  after 10 s` (if needed), `removed .../apsis-staging/2026-09-30_14-02-11`, `stopped`.
- polkit (answer 3): the **uid** that started the job may stop it without a prompt (the helper
  keeps the starter's uid from `GetConnectionUnixUser`, so `unix_user` stays when restore
  goes; the stopper's uid is asked the same way). Any other uid needs the new `stop` action,
  `auth_admin_keep`. The uid is never broadcast.
- A process stuck in the kernel (disk gone, `D` state) may ignore even `SIGKILL` for a while;
  the job stays `stopping` until it goes, and the UI keeps saying `stopping…`.
- `Finished("create", false, "stopped")` (new `STOPPED_HEADER` in `encode_error`).

UI (window only; the popup has none):
- **Stop** button at the end of the status area's progress line, while a create runs, once its
  snapshot name is known (under a second in). Also in a window that didn't start it.
- Confirm dialog: "Stop the snapshot? What was copied so far is deleted." [Keep Going]
  [Stop] (destructive style).
- Status: "Stopping…" with an indeterminate bar, then "Create stopped". Too late: "It can't be
  stopped now: the snapshot is being finished", and it ends as "Snapshot created".
- Deletes: no stop (a half-deleted snapshot is worse than either end). A bulk delete stops at
  the first failure, as now.

**Restore in 0.5.0:** stoppable while it prepares (checks, the dry run, anything staged on the
backup side), not once it writes to the live system: a half-restored system is worse than
either end. 6b already plans "stage, then apply at reboot"; the apply has no stop, and the UI
shows none then.

### F. Leftovers from interrupted creates

A create killed with the helper (power loss, crash, `systemctl stop`) leaves
`timeshift/apsis-staging/<name>/`. Today the list only warns.

- **Automatic** (answer 2): at the start of every create, holding the lock,
  after the read-write mount and before rsync, each entry in `apsis-staging/` is removed if its
  name is a snapshot name and it's a real folder, with the staging removal rules from E. Anything
  else there is left and logged. Journal: `removed leftover .../apsis-staging/
  2026-09-29_14-02-11 (an interrupted create, started 2026-09-29 14:02:11)`. Space comes back
  before the new copy starts.
- **Visible**: `List` returns the leftover names (a new `as` in the list tuple, replacing the
  0.3.0 warning string). The window's list shows one dimmed row per leftover (main window
  mock). The time is the interrupted create's start (from the name); when it was cut off isn't
  recorded.
- **Delete on that row** (answer 2): the normal `Delete(s name)`, polkit `delete`. The helper
  lists afresh; a name in `snapshots/` is a snapshot delete as now, a name among the leftovers
  (and not in `snapshots/`) is removed from `apsis-staging/` with the staging removal rules
  (E). Journal: `delete leftover "2026-09-29_14-02-11" for :1.42: removed .../apsis-staging/
  2026-09-29_14-02-11`. No new method.

### G. "Backup disk removed" from the helper

- New `Error::DeviceRemoved { device, reason }`. When a create or delete fails, the helper
  checks `/dev/disk/by-uuid/<uuid>` (the device it mounted); gone -> this error instead of the
  raw one. Encoded `device removed: <uuid>: <reason>` in `Finished`.
- Applet: "Create failed: backup disk removed" / "Delete failed: backup disk removed" in the
  status area; the reason (rsync's `Input/output error (os error 5)`) in its tooltip. The 0.3.1 inference
  (`CreateFailure::of`) goes.
- The unmount after a yanked disk: if `umount` fails, `umount --lazy`, so the next plug-in
  mounts cleanly. Logged.

### H. rsync argv

The second `--delete-excluded` (after `--exclude-from`) is removed. rsync's behaviour doesn't
change (it's a flag, not an order-dependent rule). Tests and the doc comment ("twice, as in
Timeshift") updated.

### Helper interface after 0.4.0

Removing and changing methods is a breaking change, so the interface becomes
`io.github.atraxsrc.Apsis.Helper2` (names.rs: "a breaking change gets a new interface"); bus
name, object path and unit stay. With the new version, the Timeshift-era `Native` prefixes go
(answer 4).

| method / signal | polkit | |
|---|---|---|
| `List() -> ((sssa(sss)asas)a{st})` | `list` | was `NativeListWithUsage`; adds leftovers |
| `Create(s comment)` | `create` | was `NativeCreate`; removes leftovers first; `Finished("create", ..)` |
| `Delete(s name)` | `delete` | a snapshot, or a leftover (F); `Finished("delete", ..)` |
| `Stop(s snapshot)` | `stop`, not asked of the starter's uid | new |
| `Job() -> (sssxdx)` | `list`, not interactive | new |
| `ReadConfig() -> (s(sbbas)sas)` | `list` | no users; v2 config |
| `WriteConfig(s expected, (sbbas) config) -> s` | `configure` | v2 config |
| `JobChanged((sssxdx))` | - | new, broadcast |
| `Finished(s op, b ok, s message)` | - | unchanged, to the starter |
| errors | | `NotAuthorized`, `Busy`, `InvalidInput`, `Failed`, `DeviceNotFound`, `Changed` (under `Helper2.Error`) |

Removed: `NativeListWithUsage`, `NativeCreate`, `Browse`, `Restore`, `Progress`. The bus policy
file names `Helper2`. A 0.3.x panel still running after the upgrade gets `UnknownInterface`
until it's re-added (log out and in); `just deb-install` already prints that.

### I. polkit actions after 0.4.0

| action | used by | allow_active | allow_inactive, allow_any |
|---|---|---|---|
| `io.github.atraxsrc.Apsis.list` | `List`, `Job`, `ReadConfig` | `yes` | `auth_admin` |
| `io.github.atraxsrc.Apsis.create` | `Create` | `auth_admin_keep` | `auth_admin` |
| `io.github.atraxsrc.Apsis.delete` | `Delete` (snapshots and leftovers) | `auth_admin_keep` | `auth_admin` |
| `io.github.atraxsrc.Apsis.stop` (new) | `Stop`, except from the starter's uid | `auth_admin_keep` | `auth_admin` |
| `io.github.atraxsrc.Apsis.configure` | `WriteConfig` | `auth_admin_keep` | `auth_admin` |

Removed: `browse`, `restore`, `restore-original`. 0.5.0 would add one for the full restore,
`auth_admin` (asked every time), like `restore-original` was. Five actions; the resource test
counts them.

### Root-level and destructive, for review

- **Kill**: the helper (root) sends `SIGTERM`/`SIGKILL` to rsync's process group, only the
  group it started, only while its leader is known unreaped (E).
- **New root delete path**: `apsis-staging/<name>` on stop, on failure and at the start of a
  create (leftovers), with the snapshot delete's rules. `snapshots/` is never touched by it.
- **Staging failures now use the safe walk** instead of `remove_dir_all`.
- **Config rewrite** to v2 on `w` (`.bak` keeps v1). Before `w`, the converted config is used
  for creates; the conversion keeps snapshot contents the same (B).
- **Broadcast signal** readable by any local process (D, privacy line).
- **`umount --lazy`** after a failed unmount of a removed disk (G).
- **What a snapshot holds** changes only when the user flips an include or edits filters, and
  in one conversion case: a `+` filter a later `-` hid now works (B, parent-folder lines).

### J. Tests

Unit and integration tests, no root (run by Claude):
- Config: v2 round trip; v2 checks (sign required, blank, control characters, length,
  duplicates); version 3 refused; v1 conversion: root all / excluded / hidden, all users
  everything -> `include_home`, mixed, hidden, a shared folder in `/home` (stays explicit), a
  filter after a dropped include (stays explicit), ecryptfs, a home outside `/home`, bare ->
  `- `, a `#pattern`, a `+` hidden by a later `-` (the documented exception); the import from
  the `config-rsync.json` fixture.
- Filter builder: order (defaults, ecryptfs, user filters with parent `+ dir/` lines, includes,
  built-ins); `exclude.list` for the converted fixture equals today's in what it keeps (checked
  by running rsync `--dry-run` with both lists on a temp tree with `/root`, `/home/<users>`,
  `/home/shared` and hidden files, comparing the file lists).
- Picker patterns: folder `/***`, file, escaping of `* ? [ \`, the default sign, refused portal
  path, relative path refused.
- argv: one `--delete-excluded`.
- Stop, on temp folders with real processes: a group of `sh -c 'sleep 60 & sleep 60'` dies on
  `SIGTERM`; one that ignores `SIGTERM` (`trap '' TERM`) dies on `SIGKILL` after a test-sized
  timeout; stop before spawn; stop after commit refused; no signal after the group is reaped.
- Staging removal on temp trees (also through `Delete` of a leftover): symlinked
  `apsis-staging/` refused, a symlink inside not
  followed, a non-snapshot name left alone, a fake mountinfo entry refuses it; leftovers listed
  and removed by the next create (fake runner).
- Helper: `Helper2` introspection (7 methods, 2 signals, the new types); names; policy file (5
  actions, defaults as in I); `State` job transitions (begin, name, progress, stopping, end);
  `JobChanged` has no comment; the starter exemption (a pure `stop_needs_auth`); `DeviceRemoved`
  encode/decode; the by-uuid check with a temp folder standing in for `/dev/disk/by-uuid`.
- Applet (messages into the model, no display): toolbar enablement (no device, no selection,
  job running), multi-select and the delete dialog's labels, Create dialog and its includes
  line, settings tabs (device pick, include toggles, +/- flip, add/remove/move up/down, new
  rows at the top, unsaved/Save/Discard), picker-to-pattern, job messages (running, stopping,
  done, failed, stopped, helper gone) in window and popup, the Stop dialog and too-late
  copy, leftover rows and their delete, disk-removed copy, the shortcuts; layout test at
  640 x 440 for the main window and each settings tab (`APSIS_LAYOUT_TEST=1`).
- `just test-ext4` (the user mounts the image): create, stop mid-rsync, leftover removal, on
  ext4.

On the HP (`apsis-test` over SSH; root steps are the user's to run):
1. Upgrade with `just deb-install` over 0.3.1 with a v1 config (one with a hidden-files user
   if possible): notes shown, list and create work before `w`, `w` writes v2, `.bak` is v1.
2. Create with `/root` and `/home` each on and off: the new snapshot's `exclude.list`, the
   folders present, `--link-dest` still hard-linking (`du -sh` of the new snapshot).
3. Stop mid-create from the starting window and from a reopened window (both no password,
   same uid), and from another user's session (password): journal lines, `pgrep -a rsync`
   empty, staging gone, list works at once.
4. Stop escalation: `sudo kill -STOP <rsync pid>`, then stop in the UI: `SIGKILL` after 10 s.
5. Leftover: start a create, `sudo systemctl kill apsis-helper`; the list shows the dimmed
   row; Delete on it removes it (journal); a second leftover is removed by the next create.
6. Unplug the disk mid-create: `create failed · backup disk removed`; replug lists; the mount
   point is clean (`findmnt /run/apsis/backup` empty).
7. Job status: create from the window, open the popup (progress line); close the window mid-job
   and reopen it (shows the job); `gdbus monitor --system --dest io.github.atraxsrc.Apsis.Helper`
   as the user sees `JobChanged` (the bus lets an unprivileged process receive the broadcast).
8. `pkaction --verbose --action-id io.github.atraxsrc.Apsis.stop`; `browse`, `restore`,
   `restore-original` gone.
9. Pickers open (portal) and add the right patterns; a typed `/root` pattern; the window,
   settings tabs and popup follow a light and a dark theme and an accent change live.
10. `~/Apsis-restored/` and any `.apsis-before-*` files are untouched.

### Docs

UI.md rewritten for the new window, settings and popup (browser, rooms, prompt line and
key-hint sections out); ARCHITECTURE.md (interface, config, privilege model, restore section
out); README and man page (the keyboard shortcuts live only there); metainfo `<summary>`
"System snapshot and restore" (AppStream requires one; answer 7); desktop entries without a
tagline `Comment=`; crate descriptions and the .deb's short description "System snapshot and
restore" (Debian requires one); "Timeshift-style" removed everywhere, including CLAUDE.md's
listing line (answer 7; the collection draft in RELEASE.md too); CHANGELOG (downgrade note,
restored-files note, keep-last-N removed); SECURITY.md (0.4.x); `docs/APSIS-UI-PROMPT.md`
rewritten 2026-09-30 as the short Timeshift-style contract. Screenshots retaken by the owner.

### Answers (owner, 2026-09-30)

1. Defaults: `/root` on, `/home` off for fresh installs. The 0.5.0 restore confirm must say
   whether home folders are rolled back or left untouched (added to Phase 6b).
2. Leftovers: removed automatically at the next create, and shown as a dimmed row; Delete on
   that row removes it through the normal delete path.
3. Stop: the uid that started the job stops it without a prompt; anyone else needs
   `auth_admin_keep` (`stop`).
4. `Helper2` with clean method names.
5. Keep-last-N and its pruning are removed. The reminder and the optional panel label stay.
6. Filters: Move Up / Move Down buttons; new rows at the top.
7. No tagline; the app is "Apsis". Where a summary is required (AppStream, Debian): "System
   snapshot and restore". "Timeshift-style" goes everywhere.

Also kept, as proposed: the Timeshift import on first run (through the converter), and the
pickers through libcosmic's `xdg-portal` feature (ashpd is in `Cargo.lock`; if cargo needs a
download, I ask first).

### Open questions (answered 2026-09-30, all as recommended)

1. The reminder and panel label are set on a fourth settings tab, **Misc**: "Remind me when
   the newest snapshot is older than [7] days" (0 = off) and "Show last snapshot age and disk
   use next to the panel icon".
2. The Restore button isn't shown until 0.5.0 builds it.
3. The popup keeps its content (last snapshot, disk bar, newest 5, the running job, Open Apsis
   and Refresh) in standard libcosmic widgets; the `~/apsis $ status` header and bordered
   monospace panes go.

### Done when

`cargo test --workspace`, clippy, fmt, `just test-ext4`; the HP checks 1 to 10 pass; nothing in
the tree mentions file restore, `Apsis-restored` or home modes except history (DECISIONS.md,
CHANGELOG, this file's 6a); docs updated; version 0.4.0.

Outcome (2026-09-30): 0.4.0 works on apsis-test; edge-case checks 3-6 not run, left to issue
reports.
