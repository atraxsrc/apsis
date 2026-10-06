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

**Status: design approved with changes (owner, 2026-09-30); this version has them. 0.4.1 and
0.4.2 are released (0.4.2 on 2026-10-01, check 13 passed; main merged into `restore-6b-core`
the same day); the core slice (6b.13 step 1) is done on `restore-6b-core`, the core-only parts
of the helper slice too. Next: check 0.4 (6b.12), then the helper slice build (6b.13 step 3,
from item 2).**
Core so far: the filter and protect list, home detection (`apsis_core::restore::filter`), the
restore's rsync argv (`apsis_core::restore::argv`), the refusals of 6b.7 except Busy
(`apsis_core::restore::refusal`), the Apsis in a snapshot (`apsis_core::restore::apsis`),
space parsing and the two space refusals (`apsis_core::restore::space`), the plan and state
files: `request.json` (`apsis_core::restore::plan`), `state.json` and `result.json`
(`apsis_core::restore::state`), with their shared format, version rule and atomic write
(`apsis_core::restore::file`), the ESP backup with its manifest, the check after the boot
refresh, the put-back and the ESP space refusal (`apsis_core::restore::esp`; follow-up the
same day: the seven-file set, the previous pair, the check before arming), the apply state
machine over a runner trait, tested with a fake runner (`apsis_core::restore::apply`). Format
detection is 0.4.1's `Info::is_old_format`, unchanged.
The last part of the core slice, the real-rsync temp-tree tests, is done (2026-10-01,
`crates/apsis-core/tests/restore.rs`): **the core slice is complete.** Step 2 of 6b.13 (the
owner's checks 0.1 to 0.3) passed on apsis-test, 2026-10-01. Next is step 3, the helper.

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
  and `Pop_OS-oldkern.conf`). The owner's machine (2026-10-01; apsis-test to confirm) also has
  `cmdline` there, and the oldkern entry's own pair, `vmlinuz-previous.efi` and
  `initrd.img-previous`, which are the files `/boot/vmlinuz.old` and `/boot/initrd.img.old`
  link to (the full set: 6b.6 step 4). Restoring `/` rolls back `/boot` and `/usr/lib/modules` but not
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
| `/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf` | The drop-in that keeps Pop's release upgrade from running in the restore's boot (6b.6, "pop-upgrade-init"). Written on arm, removed with the unit. **Protected so it's still there in a retry boot** after a copy that restored a snapshot without it (helper slice, 2026-10-01) |
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
| Apsis 0.4.x | the 0.4.x applet and helper. They read the protected config (v2 since 0.4.0) and don't know `/var/lib/apsis/`, so no result line until 0.5.0 is installed again | "This snapshot has Apsis 0.4.x. Install this version of Apsis again afterwards to restore again." |
| Apsis 0.3.x or older | the old applet and helper, which refuse the protected v2 config ("version 2, this Apsis reads 1") | "This snapshot has an older Apsis (0.3.x) that can't read the current settings. Install this version of Apsis again afterwards." |

The unit, the drop-in, the helper copy and `/system-update` are removed when the apply ends,
whatever the outcome (6b.6 steps 8 and 9). `/etc/apsis/` and `/var/lib/apsis/restore/{result.json,
rsync-log}` stay. The .deb's `postrm purge` also removes `/var/lib/apsis/`, a leftover unit
file and a leftover drop-in (and its folder when empty).

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
  restart**. **The ESP's space is checked again there too** (owner, 2026-10-01), like the
  other refusals of 6b.7 and unlike the two lines below: **nothing of it is stored in
  `request.json`**. What it needs (`esp::esp_needs`: what each of the two kernels and two
  initrds grows by, a put-back's temporary copy of the largest, 16 MiB) is worked out again
  from the ESP's files and the snapshot's `/boot`, and the free space is a live `statvfs` of
  `/boot/efi`. Tested with the owner's real sizes (kernels 17,273,344 and 17,056,256 bytes,
  initrds 214,307,600 and 212,169,876, a 1020M ESP with 361M free: 220 MiB needed).
  Short: refused with **one line**, and nothing is armed:
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
removed at once: the helper stays alive while a plan is ready and watches the starter's bus
name (6b.9; changed 2026-10-01 from "at the helper's next call"). Reads (`List`, `Job`,
`ReadConfig`, `CheckRestore`, `RestoreResult`) aren't refused meanwhile.

**Armed, but no restart** (owner, 2026-10-01): arming also starts a transient systemd timer in
the current boot. If the computer hasn't restarted 10 minutes after "Restart now" (the reboot
was blocked or failed), the timer disarms: it undoes the arm (`/system-update`, the unit and
its wants link, the helper copy, `state.json`), removes `request.json`, writes a journal line,
and the window shows the restore as cancelled ("Restore cancelled", as for Cancel restore). A
transient timer is gone at the restart, so it never runs in the restore's own boot. This is
what keeps an armed plan from being applied at some later, unrelated restart; the apply itself
doesn't look at the plan's age (6b.6).

**`/system-update` is the single commit point** (owner, 2026-10-01). With the link, the next
boot restores; without it, nothing does, whatever else is on disk.
- **Arm creates it last**: the unit and its wants link, the pop-upgrade-init drop-in (6b.6),
  the helper copy and `state.json` are written and synced first, then the link.
- **Disarm removes it first**, then the rest. A disarm cut short after that leaves only
  leftovers.
- **Leftovers without the link** (a unit, a helper copy, `state.json`, `request.json`) arm
  nothing. The next arm or the helper's next start cleans them.
- **The disarm service has `Conflicts=shutdown.target`**, so it can't run once a restart has
  begun: systemd stops it, or never starts it, when shutdown is queued. A restart that's
  under way keeps its link.

**Decided 2026-10-04, not built yet** (the triage; DECISIONS.md, "triage of checks 1 to 9").
Until it is built, the text above is what the code does.
- **`restore.filter` and `restore.note` are the preparation's working files.** The
  preparation writes `restore.note` beside `restore.filter`, with the recovery note's text
  as it goes onto the backup disk (6b.11). The two share one lifetime: removed on Stop, on
  Cancel, on a refusal (while preparing or at "Restart now"), and after a finished restore
  or a disarm, next to `request.json` in the apply's `clean_up` and `give_up`, in
  `disarm()` and in `clean_at_start()`. Never through `remove_arm_files`:
  `clean_leftovers` calls that right before the arm and would delete the files the arm is
  about to copy. A `Retry` and a `LinkStuck` end remove nothing. **Built (step 7a).**
- **The arm keeps a pair**, before anything else of the arm is written: `restore.filter`
  is copied to `last-restore.filter` and `restore.note` to `last-restore.note`, all in the
  state folder, each copy under a temporary name and renamed. If either copy fails the arm
  is refused. Then `rsync-log` is cleared: at the arm, no longer at the preparation. "Last
  restore" means "last arm": a disarmed arm has replaced the pair too. All of these files
  are under `/var/lib/apsis/***` (6b.2's protect list). **Built (step 7a).**
- **A refused "Restart now" removes the plan through `remove_plan`**, so the journal says
  `plan removed (refused ...)` and `refused:`, not `plan removed: request.json` and
  `failed:` (check 4). **Built (step 7a).**
- **The helper takes its job lock around the arm** (6b.9), so a package script can't land
  inside one. **Built (row 5)**: "Restart now" takes the plan where it is
  (`State::take_ready`: the plan is marked in its slot and the job lock on disk is taken, in
  one step) and holds both through the re-checks, the arm, the link and the timer, or
  through the undo and the plan's removal after a refusal. The plan leaves its slot, and the
  lock is released, at its end: `done` right before logind's `Reboot`, `stopped` after the
  removal.
- **`disarm()` syncs after it removes the link**, for the timer and for the package script.
  A power cut seconds after a disarm must not bring the link back. **Built (step 7b).**
- **A second preparation while a restore is armed is refused before it touches anything**
  (the armed-state gap, found 2026-10-04; owner: fixed in 0.5.0). An armed plan is no longer
  "ready" in the helper, so a second `Restore` got as far as the preparation, whose first
  step clears the state folder: the armed plan, its filter, `state.json` and the helper copy
  went, and only then did the dialog's check refuse because of the link. A restart that
  stalled would have found the link with nothing behind it. Now the preparation looks at
  `/system-update` and `/etc/system-update` first: Apsis's own link is
  `Refusal::RestoreArmed` ("A restart to restore is already waiting." / "Restart the
  computer, or wait for it to time out."), anything else at either name is the existing
  `PendingUpdate`. Nothing is written or removed, the log included. The dialog's check
  (`CheckRestore`, `refusal::check`) makes the same split, read-only, in the pending
  update's place in 6b.7's order, so a click on Restore while armed gets the same words
  before any password. `Restore` itself refuses the same way before the password and the
  job lock (owner, 2026-10-04, "B1"): after the name check it reads the link, and once more
  after the password (the dialog can stay open while another window arms), both before the
  job begins. Refused there, it is the method's error (`restore refused: restore-armed`),
  no job is announced, and no other window shows anything; the calling window opens the
  "Can't restore this snapshot" dialog and waits for no job end. The preparation's own look
  stays as the third line, under the lock (a refusal there is a job that ends `failed`).
  At apply the link is the restore's own and is never refused as armed.
  **Built (armed-gap fix).**
- **No snapshot is deleted while a restore is armed** (the armed-gap audit's row 12; owner,
  2026-10-04). `Delete` and `DeleteMany` are refused while Apsis's own `/system-update` is
  in place (`arm::is_armed`), whichever snapshots they name: the armed plan's snapshot and
  its safety snapshot must be there at the restart, armed lasts minutes at most, and one
  rule for every delete is the simpler one to state. The link is looked at before the
  password and once more after it, both before the job lock: no job is announced, nothing
  is mounted, and a delete of several is refused once, as a whole. Another tool's link
  refuses nothing here. On the wire it is `Error::RestoreArmed` (`restore armed`, a
  `Failed`); the window's line is "Not deleted: a restart to restore is waiting." with the
  tooltip "Restart the computer, or wait for it to time out. Nothing was deleted." The
  seconds between "Restart now" taking the plan and the link being made are not covered
  (below, step 11: since then a delete in those seconds is `Busy`). **Built (row 12).**
- **Knowingly left** (owner): in the seconds between "Restart now" and the reboot, a
  package operation on Apsis cancels the arm, and only apt's output says so.
- **Knowingly left** (owner, 2026-10-04; the armed-gap audit's row 1): `Create` is not
  refused while a restore is armed. A reboot during it leaves an interrupted create's
  folder, which the next create removes; it touches neither the plan's snapshot, nor the
  safety snapshot, nor the state folder.
- **Knowingly left until step 11** (owner, 2026-10-04): "Restart now" takes the plan out of
  "ready" before its checks and the arm, which take seconds (a mount, sizes). In those
  seconds no plan is ready and no link exists, so a second `Restore` is let in and its
  first step clears the plan that is being armed. The arm then fails at keeping the pair
  and the plan is dropped with an error: nothing ends up armed without a plan. The job
  lock around the arm (above, 6b.9) closes it. **Closed (step 11, row 5)**: in those
  seconds the plan is still in its slot, so a second `Restore`, a `Delete` or `DeleteMany`,
  a `Create` and a `WriteConfig` are `Busy` (the window's "busy" line; owner: kept as
  `Busy`), lists and `CheckRestore` go on, and `Job()` shows the restore at 100%. A second
  "Restart now" and a cancel in those seconds are `Busy` too. `CancelRestore` and the
  starter-gone path hold the plan and the lock the same way while the plan's files go
  (owner, "Q8").

### 6b.6 The apply, and the boot files

Everything runs from `apsis-restore.service` in offline-update mode, as root, with the helper's
fixed `PATH` and cleared environment. Each step is logged to the journal and the console;
progress and messages go to plymouth (`plymouth system-update --progress=N`, `plymouth
display-message`) when it runs.

`state.json` holds `attempts` (copies that started), `written` (anything under `/` was ever
written by this restore), `step` (`armed`, `copy`, `boot-files`, `boot-refresh`, `end`),
`problems` (the copy ended with exit 23) and `boots` (offline boots in which the apply
began). It's saved, and so fsynced, before each step that depends on it:

| saved | before |
|---|---|
| `boots + 1`: **the apply's first write in every offline boot** | everything else. Only two things run before it: the check that `/system-update` is Apsis's link, and reading `state.json` |
| `attempts + 1`, `written`, step `copy` | pass 1 (step 3) |
| step `boot-files`, `problems` | the ESP backup (step 4). The boot files are still untouched |
| step `boot-refresh` | the boot refresh and everything after it (steps 5 to 7). From here the boot files may be changed |
| step `end` (after `result.json`) | the cleanup (step 8) |

A state that can't be saved stops the apply before the step: no copy without its attempt on
disk (`not-started`, or `failed` if an earlier copy wrote), and no boot refresh without
`boot-files` on disk (`boot-kept`, the ESP untouched). **A boot after a power cut goes on from
the saved step** (6b.10).

1. **Arm check.** `/system-update` must point to `/var/lib/apsis/restore`, and `request.json`
   and `state.json` must be there. **A link that points anywhere else is another tool's
   update, and is left exactly where it is** (changed 2026-10-01: `systemd.offline-updates(7)`
   says a service that finds a link to another location "must exit without error"; removing
   it would cancel that tool's update). Apsis then removes only its own leftover unit files,
   doesn't restart, and exits cleanly. With Apsis's link and an unreadable plan or state:
   6b.10. **No wall-clock age check** (owner, 2026-10-01): the
   apply never compares `prepared_at` with the clock. The clock in early boot can be hours off
   (a hardware clock in local time), and the 30 minutes are checked at "Restart now" (6b.5),
   with the disarm timer covering an arm that no restart followed.
2. **Backup disk.** Wait up to 60 s for `/dev/disk/by-uuid/<uuid>` (USB disks are slow at
   boot): `udevadm wait --timeout=60 /dev/disk/by-uuid/<uuid>`, fixed argv (check 0.3 found
   the disk there at once; a slower disk isn't promised to be). A wait that times out is
   **never started**. Then mount it read-only (`ro,nosuid,nodev,noexec`) at `/run/apsis/backup`, then the delete's
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
   Never `--delete-excluded`, `--ignore-errors`, `-L` or `--link-dest`. Before every copy
   the snapshot itself is checked (6b.10, "Exit 23"). Exit 0 or 24: go on. 23 (some files
   couldn't be written or deleted): go on; the result is "restored with problems" and names
   the log. **23 with rsync's "skipping file deletion" line**, **23 with the snapshot gone
   when it's looked at again right after the copy** (check 6, 2026-10-03), anything else, or
   no exit code (a signal): **copy broke** (6b.10).
   The copy step returns only after a `syncfs` of each filesystem it wrote to (`/`, and a
   separate `/home` that's restored): rsync doesn't sync. Only then is step `boot-files`
   saved, so a copy that `state.json` records as ended is on disk and is **never run again**,
   also after a power cut. A boot that finds `attempts` at 3
   and step `copy` (the power went during the third copy) gives up with `failed`; there's no
   fourth copy.
4. **ESP backup.** Copy the ESP file set to `/var/lib/apsis/restore/esp-backup/` (on `/`,
   protected), fsync, and byte-compare each copy with the original. **The file set** is one
   list in one place (`apsis_core::restore::esp`, `SET`): the only files Apsis reads, backs
   up or writes on the ESP. Names as on the owner's machine (Pop!_OS 24.04, kernelstub;
   2026-10-01):

   | file | where | |
   |---|---|---|
   | `vmlinuz.efi` | `EFI/Pop_OS-<root-uuid>/` | required |
   | `initrd.img` | `EFI/Pop_OS-<root-uuid>/` | required |
   | `cmdline` | `EFI/Pop_OS-<root-uuid>/` | required |
   | `Pop_OS-current.conf` | `loader/entries/` | required |
   | `vmlinuz-previous.efi` | `EFI/Pop_OS-<root-uuid>/` | optional |
   | `initrd.img-previous` | `EFI/Pop_OS-<root-uuid>/` | optional |
   | `Pop_OS-oldkern.conf` | `loader/entries/` | optional |

   (`vmlinuz-previous.efi`, but `initrd.img-previous`.) The three optional files are
   **optional together**: all there, or none (one kernel installed). A required file that's
   missing, or only part of the optional three, fails the backup.
   **Never backed up and never written:** `loader/entries/Recovery-*` (and the recovery's own
   `EFI/Recovery-*` folder), `loader/loader.conf`, `loader/random-seed`,
   `loader/entries.srel`, `EFI/BOOT/`, `EFI/systemd/`. A test checks that a put-back leaves
   them as they were (same files, not rewritten ones).
   A copy that doesn't match, or a backup that fails: stop before touching the boot files,
   and go to the **boot files failed** path below with nothing to put back. Each file's size
   and SHA-256 go into `esp-backup/manifest.json`, written last: a backup without a manifest
   isn't one, and a put-back verifies the backup against it before it writes anything to the
   ESP. This happens before kernelstub, the one command that writes the ESP.
5. **Boot files**, on the restored `/`. No chroot is needed: the running system *is* the target,
   and the tool is the restored system's own. **Settled on apsis-test (owner, 2026-10-01):
   exactly what Pop's hooks run, and nothing else:**
   ```
   kernelstub --verbose --preserve-live-mode
   ```
   Both `/etc/kernel/postinst.d/zz-kernelstub` and `/etc/initramfs/post-update.d/zz-kernelstub`
   call it that way. The flag is hidden from `--help` but defined in
   `kernelstub/application.py`; `kernelstub --dry-run --verbose --preserve-live-mode` exits 0.
   What it does (dry run, kernelstub 3.1.4): ESP folder `EFI/Pop_OS-<root UUID>`; the newest
   `/boot/vmlinuz-*` becomes `vmlinuz.efi` + `initrd.img`, the next one
   `vmlinuz-previous.efi` + `initrd.img-previous`; loader entry `loader/entries/Pop_OS-current`;
   manage-only (NVRAM entry -1, no efibootmgr writes); copies `/proc/cmdline` into the ESP
   folder. Config: management mode true, install loader true, config version 3.
   **No `update-initramfs`.** The restored `/boot/initrd.img-*` are the snapshot's, built on
   that system from the modules and configuration that are now back; rebuilding them would
   run kernelstub through the hooks for every kernel it rebuilds (several kernelstub runs
   where one is wanted; **corrected 2026-10-04**: this said "twice per kernel" and "five
   ESP writes", read from the two hooks and never counted. "Twice per kernel" was wrong: in
   check 8's drill, inside a chroot, the hooks ran kernelstub once per kernel, and the
   by-hand call after them changed nothing; how many of those three runs rewrote files on
   the ESP was not counted. Outside a chroot the count is **unverified**),
   run `kernel-install add` with plugins Apsis doesn't know, rewrite the protected kernel's
   initrd that rule 10 keeps, make `/boot` differ from the snapshot, and take minutes of the
   boot screen. `esp::esp_needs` is exact now: the initrd copied to the ESP is the one in the
   snapshot's `/boot`. (DECISIONS.md, 2026-10-01, "checks 0.1 to 0.3" and "helper slice".)
   **The live `crypttab` versus the snapshot's initrd** (1b of the helper slice): the initrd
   is the snapshot's, while `/etc/crypttab` and `/etc/fstab` stay live (rule 4). **Verified
   on apsis-test (owner, 2026-10-01, `initrd.img-7.1.5`)**: the initramfs holds
   `main/cryptroot/crypttab`, 0 bytes, and `main/etc/fstab`, empty. initramfs-tools embeds an
   **empty** fstab, never the live one's content, and the `cryptroot` hook embeds the crypttab
   entries the system needs at boot (root, `/usr`, resume, `initramfs`-flagged ones); the live
   crypttab has only cryptswap with a `/dev/urandom` key, which the hook doesn't carry, hence
   0 bytes. So **preparing compares the snapshot's `etc/crypttab` with the live one** and
   refuses when they differ (6b.7; the rebuild is 0.5.x). On apsis-test both are the same
   one line; the comparison matters for an encrypted-root install, whose initramfs carries
   root's crypttab line (unverified here, no such machine; claims table). `fstab` isn't
   compared: the initramfs never holds its content, and fstab changes (a data disk added) are
   common and are exactly what rule 4 keeps.
   **The flag must exist in the snapshot's kernelstub**: the snapshot's
   `etc/initramfs/post-update.d/zz-kernelstub` must contain `--preserve-live-mode` (then its
   kernelstub takes the flag, and it's the call that system was tested with). Otherwise
   `KernelIncomplete` (6b.7). The baseline snapshot's two hooks both contain it (owner,
   `grep -c` 1 each, 2026-10-01).
   kernelstub picks the newest `/boot/vmlinuz-*` by version; the check below reads the
   `/boot/vmlinuz` link. On Pop!_OS the link is to the newest (`linux-update-symlinks`), so
   they agree; a snapshot where they don't ends `boot-kept`, which is the right outcome.
   **At apply they never agree in a rollback** (check 2's setup, 2026-10-02): after pass 1,
   `/boot` holds the snapshot's kernels and the protected running kernel (rule 10), which is
   the newest, while the links are the snapshot's. Left alone, kernelstub would put the
   running kernel on the ESP and every rollback would end `boot-kept`. **So the apply's call
   names the kernel**: `kernelstub --verbose --preserve-live-mode --kernel-path /boot/<what
   /boot/vmlinuz points to> --initrd-path /boot/<what /boot/initrd.img points to>`. Both
   options win over the newest-by-version choice (`application.py:167-193`) and aren't saved
   in kernelstub's configuration (read on apsis-test). Without the two links the plain call
   runs and the check decides. The "previous" pair stays kernelstub's own choice (the second
   newest in `/boot`, the protected kernel's neighbour until step 7 removes it); a mismatch
   there is reported, never a failure, and the next kernel update rewrites it.
6. **Check, byte for byte**: the ESP's `vmlinuz.efi` and `initrd.img` equal the files
   `/boot/vmlinuz` and `/boot/initrd.img` point to; `/usr/lib/modules/<that version>/` exists;
   `Pop_OS-current.conf` exists. kernelstub exited 0.
   - **Which tree** (owner, 2026-10-01): the check (`esp::check(esp, root, root_uuid)`) runs
     twice, against two trees. **Before arming, against the live system** (`/` and its
     `/boot/efi`): a current pair that fails refuses the restore (6b.7), since rule 10 only
     protects the kernel the ESP boots if that's the one `/boot` links to. **Here, in the
     apply, against the restored tree** (`/` after pass 1 and the boot refresh of step 5).
   - **The previous pair**: if `vmlinuz-previous.efi` and `initrd.img-previous` are on the
     ESP, they must equal the files `/boot/vmlinuz.old` and `/boot/initrd.img.old` point to,
     and that version's modules folder must exist. A mismatch there is **reported, never a
     failure**: not a refusal before arming, and no put-back here. The current pair still
     boots. It goes to the journal and the result.
     **Only part of the optional three** (step 4) is different before arming: it **refuses**
     (6b.7; owner, 2026-10-01), because the apply's backup would fail on it after the copy.
     Here, after the boot refresh, it's only reported.
   - **Passes:** go on.
   - **Fails** (or a command failed): **boot files failed**. Put the backed-up files back
     on the ESP (each written to a temporary name in the same folder, fsynced, renamed), then
     byte-compare them with the backup. The protected kernel stays (step 7 is skipped), so the
     ESP boots the kernel it booted before, whose modules and `/boot` files rule 10 kept.
     That's checked too (`esp::boots_kernel`): a put-back that worked onto a kernel whose
     files are gone is `boot-broken`, not `boot-kept`. Then
     step 8, result `boot-kept`: the system is the snapshot's, but it still starts the newer
     kernel. Not retried (a second kernelstub run would do the same). If putting back also
     fails to compare: result `boot-broken`, and the boot screen and result point to the
     recovery steps (6b.11).
7. **Protected kernel, cleanup** (only after a passed check): if the snapshot doesn't have the
   running kernel's version, remove the rule 10 files (the modules folder with
   `prune::remove_at`, one filesystem, no symlinks followed). The system now matches the
   snapshot. A power cut here is harmless: the ESP already boots the restored kernel.
8. **End**, in this order: write `result.json` (the outcome, the snapshot, the safety
   snapshot, the time); save step `end` in `state.json`; remove `/system-update` (first),
   the unit and its wants link and the helper copy; remove `esp-backup/`; `sync`, restart.
   The result is on disk before the link goes, so a power cut can't leave a finished restore
   with no result; a boot that finds the link and step `end` only does the cleanup.
   Every way a restore ends goes through this step and restarts: with the link gone that's
   the normal boot.
   **A link that can't be removed gets no restart** (owner, 2026-10-01): with
   `/system-update` still there, a restart comes straight back to the apply. So if removing
   the link fails, the apply removes nothing else, doesn't restart, and the helper exits 0;
   `result.json` and step `end` are already saved. systemd then removes the link itself and
   restarts once `system-update.target` is reached (`systemd.offline-updates(7)`, point 7).
   The only restarts the apply makes with the link in place are the retries after a broken
   copy, and an attempt is on disk before each copy: two at most.

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
FailureAction=reboot

[Service]
Type=oneshot
ExecStart=/var/lib/apsis/restore/apsis-helper --apply-restore
StandardOutput=journal
```

`StandardOutput=journal`, not `journal+console` (check 0.3 showed the console text over the
splash; what the person sees goes through plymouth only). `FailureAction=reboot`, not
`OnFailure=`. **Never `KillMode=none`** (pop-upgrade-init's `KillMode=none` is what left
`upgrade.sh` and `apt-get` running into the final kill in check 0.3). No `Before=` or
`Conflicts=` against the other offline-update units (below).

The helper binary is copied to `/var/lib/apsis/restore/` on arm, for the same reason. No
`daemon-reload` is needed: the unit is read at the next boot.

**pop-upgrade-init** (found in check 0.3; helper slice, 2026-10-01). Pop's
`/usr/lib/systemd/system/pop-upgrade-init.service` is wanted by `system-update.target`, has
only `ConditionPathExists=/system-update` (it doesn't look at the link's target) and
`KillMode=none`. Its `upgrade.sh` unconditionally removes `/pop-upgrade` and
`/pop_preparing_release_upgrade`, switches plymouth to system-upgrade mode, touches
`/upgrade-attempted`, masks `acpid` and `pop-upgrade`, and runs `apt-get install -f` and
`apt-get full-upgrade --no-download`; on success it removes `/system-update`, removes HWE
kernels, autoremoves, runs `update-initramfs -c -k all`, deletes and re-creates the EFI boot
entry (`efibootmgr -B` / `-c`) and reboots; on failure, `systemctl rescue`. It ran next to the
dummy unit in check 0.3, on Apsis's link. **It must never start in a boot whose
`/system-update` is Apsis's.** `packagekit-offline-update` checks the link's target itself
("no trigger, exiting", or "another framework set up the trigger" in check 0.4) and
`fwupd-offline-update` starts, finds nothing of its own and finishes at once (checks 0.3 and
0.4; it is not skipped on a condition); neither needs anything from Apsis.

The fix, three parts:

1. **A drop-in written on arm**, `/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf`
   (text in `apsis_core`, tested, like the unit):
   ```ini
   # Written by apsis-helper while a restore is armed, removed when it ends.
   # Pop!_OS's release upgrade must not run in the boot that applies an Apsis restore.
   # The path below exists exactly while /system-update is Apsis's link.
   [Unit]
   ConditionPathExists=!/system-update/apsis-helper
   ```
   The marker is **the helper copy, reached through the link**: `/system-update` is a symlink
   to `/var/lib/apsis/restore`, and `ConditionPathExists=` follows it (`access(2)`), so the
   path exists exactly while the link points at Apsis's folder. Nothing extra is created or
   removed to keep it true: the link is the single commit point (6b.5), and this rides on it.
   - **Not shipped in `/usr`** (the owner's first thought) but written on arm, in `/etc`,
     and **protected** (6b.2). A packaged drop-in would be deleted by pass 1 for every
     snapshot made before 0.5.0, and in a **retry boot** after such a copy pop-upgrade-init
     would run on Apsis's link. The runtime drop-in survives the copy, so it's there in every
     armed boot. The protect-list test still passes: it's not a packaged file.
   - Written and synced before the link (6b.5's order), removed by `Runner::remove_arm_files`
     with the unit, on disarm, and by `postrm purge`. A leaked one is inert: without the
     link, the path doesn't resolve and the condition passes.
   - systemd reads a drop-in for a unit in `/usr/lib` from `/etc/systemd/system/<unit>.d/`;
     a drop-in for a unit that isn't installed is ignored. No `daemon-reload`: read at the
     next boot.
   - **Every end**, with the condition evaluated when systemd starts the unit, at the start
     of the offline boot's transaction (both units are wanted by `system-update.target`, both
     `After=sysinit.target`; nothing has been copied yet):
     - `Finished`, `GaveUp`: the link is removed first, then the restart. In that boot
       pop-upgrade-init was already skipped at the start; at the next boot there's no link
       at all, so its own `ConditionPathExists=/system-update` fails. Free, and nothing runs.
     - `Retry`, a power cut mid-copy, a helper that dies after the link check: the link
       stays, the drop-in is protected, so the next boot blocks it again.
     - `LinkStuck`: the link stays, the helper exits 0; pop-upgrade-init was skipped at the
       start of this boot; systemd's `system-update-cleanup.service` removes the link and
       reboots. If the link survives even that, the next boot is Apsis's again (cleanup
       only) and blocks it again.
     - `NotArmed` (the link is another tool's): `/system-update/apsis-helper` resolves into
       that tool's folder and isn't there, so pop-upgrade-init runs when Pop meant it to.
       Apsis doesn't touch the link (6b.6 step 1).
     - A later boot after a removed link: the drop-in is gone with the arm, or if leaked,
       inert. **It can never stay blocked.**
   - A snapshot made while a drop-in was leaked holds it; a restore of that snapshot leaves
     the live (protected) one in place. The helper's next start removes a leaked one with
     the other leftovers (6b.5).
2. **Arming refuses while a Pop!_OS release upgrade is in progress or half done** (6b.7,
   `Refusal::PopUpgradePending`): `/pop-upgrade`, `/pop_preparing_release_upgrade` or
   `/upgrade-attempted` exists. **Three names**, `lstat`, anything at the name counts; in
   core, pure (`refusal::check_pending(found)`), with tests. `/system-update` and
   `/etc/system-update` as anything are the existing `PendingUpdate`. Checked in the dialog,
   when preparing and right before the link is made; **not at apply** (there `/system-update`
   is Apsis's own, and the drop-in holds pop-upgrade off).
   **Not refused** (owner, 2026-10-01, from apsis-test): fwupd's `/var/lib/fwupd/pending.db`
   is its history database and exists on most machines (on apsis-test since Sep 30 with
   nothing pending); `fwupd-offline-update` ran in check 0.3's boot and finished at once,
   and `fwupdoffline` checks the `/system-update` link itself and asks the database for
   pending devices. PackageKit's prepared update (`/var/lib/PackageKit/prepared-update`) is
   staged, not triggered; `pk-offline-update` exits "no trigger" in that state (check 0.3),
   and COSMIC Store stages updates routinely, so refusing on it would block restores for no
   reason. A real fwupd or PackageKit update makes its own `/system-update` link, which
   `check_arming` already refuses.
3. **No `Before=` and no `Conflicts=`** on `apsis-restore.service` against the three units.
   `Conflicts=` inside one transaction (both wanted by the same target) makes systemd drop
   one of the two jobs, and which one isn't ours to choose. `Before=` would only move
   pop-upgrade-init's condition check from the start of the boot to after the copy, which
   gains nothing while the drop-in is protected. None of the three has any ordering against
   Apsis's unit (verified, 5 below), so a cycle isn't the worry; it's still ordering against
   units Apsis doesn't own. The drop-in is the fix; the refusal keeps a real Pop upgrade and
   a restore from being armed at the same time.
4. **Unit facts for the record** (owner, apsis-test, 2026-10-01): `pop-upgrade-init`,
   `packagekit-offline-update` and `fwupd-offline-update` are all wanted from
   `/usr/lib/systemd/system/system-update.target.wants/`; none has ordering against ours.
   `pop-upgrade-init`: `Before=... pop-upgrade.service`, `FailureAction=reboot`,
   `KillMode=none`, output to `/var/log/upgrade.log`. `packagekit-offline-update` has no path
   condition of its own (`pk-offline-update` decides). No drop-in folders exist today.
   `udevadm wait` exists (systemd 255.4).

**systemd's `system-update-cleanup.service`** (owner, apsis-test, 2026-10-01: `After=
system-update.target`; runs only while `/system-update` or `/etc/system-update` exists
(`ConditionPathExists=|` and `ConditionPathIsSymbolicLink=|` on both); `ExecStart=rm -fv
/system-update /etc/system-update`; `SuccessAction=reboot`; no `FailureAction`). It runs
after every unit the target wants, so after Apsis's, once Apsis's unit has exited. Every end
through it:

| end | the link when Apsis's unit exits | who reboots | relies on cleanup? | if cleanup's `rm` fails |
|---|---|---|---|---|
| `Finished`, `GaveUp` | gone (Apsis removed it first) | Apsis (`systemctl reboot --no-block`, then exit 0); cleanup's condition is false, so it's skipped | no | n/a |
| `Retry` | kept on purpose | Apsis, with the link in place | no, and **it must not run**: `rm` would cancel the retry and the next boot would be normal with `/` half copied. Apsis enqueues the reboot before its unit exits, and the reboot transaction (`replace-irreversibly`) drops cleanup's pending start job through cleanup's `Conflicts=shutdown.target`. **Unverified on apsis-test** (claims table); if cleanup lacks that `Conflicts=`, `restart` becomes a blocking `systemctl reboot`, so Apsis's unit never exits and the target is never reached | n/a |
| `LinkStuck` | kept because `rm` failed for Apsis | **cleanup** (`SuccessAction=reboot`), if its `rm` succeeds where Apsis's failed | **yes, the only one** | nothing reboots: the machine sits in `system-update.target`, no desktop, the boot screen showing Apsis's last line. The next power-on comes back to the apply (step `end`, cleanup only, the same `rm`), counted against the boot cap, and gives up the same way. **The user sees** the boot screen with no restart: so `say` for `LinkStuck` must tell them what to do (wording under "for the UI slice": turn it off and on; if it comes back, the README's recovery steps, which include `rm /mnt/system-update`) |
| `NotArmed` | the other tool's | that tool, or cleanup when it's done | not Apsis's business | that tool's problem, unchanged by Apsis |

**A reboot call that fails** (D-Bus down, logind gone) would leave `Finished` and `GaveUp` in
the same seat: the link is gone, cleanup is skipped, nothing reboots. So the real runner's
`restart` has a fallback: if `systemctl reboot --no-block` doesn't return 0, the helper
**exits 1**, and the unit's `FailureAction=reboot` reboots. With the link gone that's the
normal boot; in `Retry` it's the retry the apply wanted, with its attempt already on disk, so
the "never exit non-zero with the link in place" rule has this one bounded exception. For
`LinkStuck` and `NotArmed` nothing changes: exit 0, no restart.

### 6b.7 Refusals (in the dialog, again when preparing, again at apply)

Each opens the "Can't restore this snapshot" dialog instead: the reason in one plain line, one
line on what to do, and Close (wording in 6b.8's string table). The lines below are the gist:

| check | line |
|---|---|
| no `/sys/firmware/efi` | "Restore needs a computer that starts in UEFI mode." |
| no `/etc/kernelstub/configuration` or `kernelstub` (GRUB, other distros) | "Restore only works on Pop!_OS with systemd-boot for now." |
| ESP not mounted at `/boot/efi` as vfat, or no `EFI/Pop_OS-<root-uuid>/` on it | "The boot partition isn't where Pop!_OS keeps it." |
| root filesystem isn't ext4 | "Restore doesn't support a <fstype> system disk yet." |
| root's device (lsblk, walked by `PKNAME`) is neither a plain partition (`TYPE` `part`) nor, since 0.6, the layout of an encrypted Pop!_OS install: an `lvm` volume on one `crypt` mapping (`LVM2_member`) of one `part` (`crypto_LUKS`) that the live crypttab opens under the mapping's name by that partition's UUID (`refusal::encrypted_root`; both runs of the LUKS spike, apsis-test, 2026-10-05 and 2026-10-06). LUKS without LVM, LVM without LUKS, a volume on several devices, a mapping of a whole disk or of a RAID, a mapping name or UUID that isn't a plain word: refused | "Restore doesn't support the way this system disk is set up yet." |
| `/boot`, `/usr` or `/var` a separate mount | "Restore doesn't support a system split over several partitions yet." |
| **root UUID**: the snapshot's `info.json` `sys-uuid` differs from the UUID of the filesystem mounted at `/` (`findmnt`), or the snapshot's `etc/kernelstub/configuration` names another `root=UUID=` | "This snapshot is from another installation." |
| snapshot isn't rsync, has no `localhost/` or `exclude.list` | "This snapshot can't be restored: <reason>." |
| snapshot's `/boot/vmlinuz` has no `/usr/lib/modules/<version>/` in the snapshot, or the snapshot has no `kernelstub`, or its `etc/initramfs/post-update.d/zz-kernelstub` is missing or doesn't contain `--preserve-live-mode` (6b.6 step 5; helper slice, 2026-10-01). `update-initramfs` is no longer required: the apply doesn't run it | "This snapshot's kernel files are incomplete, so it can't be restored safely." |
| **another update is pending**: `/system-update` or `/etc/system-update` already exists, as anything and pointing anywhere (a link, a dangling link, a file, a folder; systemd's generator reads both names). In `refusal::check`, and again as the last check before arming makes the link (`refusal::check_arming`, pure; owner, 2026-10-01). The same `PendingUpdate` refusal | "A system update is waiting for a restart. Restart first, then restore." |
| **a restore is armed**: `/system-update` is Apsis's own link, it points at the state folder (`Refusal::RestoreArmed`, `restore-armed`; the armed-gap fix, 2026-10-04). Told apart from the row above, whose words would be false for it, and checked in its place in the order: in `refusal::check` for the dialog, by `Restore` before the password and again before the job lock, and by the preparation before it touches anything (6b.5). Armed and another tool's `/etc/system-update` at once is this row. **Never at apply**, where the link is the restore's own | "A restart to restore is already waiting. Restart the computer, or wait for it to time out." |
| **a Pop!_OS release upgrade is in progress or half done** (helper slice, 2026-10-01; 6b.6 "pop-upgrade-init"): `/pop-upgrade`, `/pop_preparing_release_upgrade` or `/upgrade-attempted` exists (`Refusal::PopUpgradePending`). Three names, `lstat`, anything at the name counts. `refusal::check_pending`, pure, in the dialog, when preparing and with `check_arming`; **not at apply**. fwupd's and PackageKit's files are **not** refused (owner, 2026-10-01; 6b.6) | wording in the UI slice (gist: "A Pop!_OS upgrade is in progress." / "Finish or cancel it first, then restore.") |
| **the disk setup changed**: the snapshot's `etc/crypttab` differs from the live one, compared with comments and blank lines dropped and each line's fields joined by one space, in order (`refusal::crypttab_differs`, pure; helper slice, 2026-10-01; 6b.6 step 5). The snapshot's initrd carries the crypttab entries the snapshot's system needed at boot, and the apply doesn't rebuild it (0.5.x does). A snapshot with no `etc/crypttab` compares as empty. On apsis-test the embedded crypttab is empty (cryptswap with a random key isn't carried), so this matters once an encrypted root is accepted | wording in the UI slice (gist: "The encrypted disks are set up differently from when this snapshot was made." / "Restore needs a snapshot made with the current disk setup.") |
| not enough space (6b.4) | the space line |
| a dry run gave no size that could be read (`space::dry_run_size`, `Refusal::SizeUnknown`; added 2026-10-01, core). The size is never taken as zero | wording in the UI slice |
| the ESP is short for the boot refresh and a put-back (`esp::esp_needs`; added 2026-10-01, core). Not in `request.json`: re-checked at "Restart now" from a live `statvfs` (6b.4) | wording in the UI slice |
| on the live system, the ESP's `vmlinuz.efi` and `initrd.img` aren't the files `/boot/vmlinuz` and `/boot/initrd.img` point to, or that kernel has no modules, or there's no current entry (`esp::check_before_arming`, 6b.6 step 6; added 2026-10-01, core). A whole previous pair that isn't the `.old` links' never refuses | wording in the UI slice |
| on the live system, the ESP has some but not all of `vmlinuz-previous.efi`, `initrd.img-previous` and `Pop_OS-oldkern.conf` (`esp::check_before_arming`, `Refusal::BootFiles(PreviousIncomplete)`; owner, 2026-10-01). All three or none pass. Checked at arming and again at "Restart now"; the apply's backup keeps failing on it as the backstop (6b.6 step 4) | wording in the UI slice |
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
restore-apsis-0.4    This snapshot has Apsis 0.4.0. Install this version of Apsis again
                     afterwards to restore again.
restore-apsis-0.3    This snapshot has an older Apsis (0.3.1) that can't read the current
                     settings. Install this version of Apsis again afterwards.
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
| dialog | `This snapshot has Apsis {version}. Install this version of Apsis again afterwards to restore again.` |
| dialog | `This snapshot has an older Apsis ({version}) that can't read the current settings. Install this version of Apsis again afterwards.` |
| dialog muted | `Experimental · if it won't start, see "If a restore goes wrong" (README)` |
| buttons | `Cancel`, `Restore` |
| refusal title | `Can't restore this snapshot` |
| refusal | `It was made on another installation of the system, so its files don't fit this one.` / `Pick a snapshot made on this installation.` |
| refusal | `This computer doesn't start in UEFI mode, which restore needs.` / `Restore can't be used on this computer.` |
| refusal | `Restore works only on Pop!_OS with systemd-boot for now.` / `Restore can't be used on this computer yet.` |
| refusal | `The boot partition isn't where Pop!_OS keeps it.` / `Restore can't be used on this computer.` |
| refusal | `Restore doesn't support a {fstype} system disk yet.` / `Restore can't be used on this computer yet.` |
| refusal | `Restore doesn't support the way this system disk is set up yet.` / `It works on a plain partition and on Pop!_OS's standard encrypted install. Snapshots still work: their files are on the backup disk, under timeshift/snapshots.` (0.6; before: `Restore doesn't support an encrypted or LVM system disk yet.` / `Restore can't be used on this computer yet.`) |
| refusal | `Restore doesn't support a system split over several partitions yet.` / `Restore can't be used on this computer yet.` |
| refusal | `This snapshot's kernel files are incomplete.` / `Pick another snapshot.` |
| refusal | `This snapshot can't be read: {reason}.` / `Pick another snapshot.` |
| refusal | `A system update is waiting for a restart.` / `Restart first, then restore.` |
| refusal | `A restart to restore is already waiting.` / `Restart the computer, or wait for it to time out.` |
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
| ready, gone | `The preparation is gone. Start the restore again.` |
| refusal at Restart now (owner, 2026-10-02) | `The preparation was dropped. Restore again to measure afresh.` (a third line in the "Can't restore" dialog when the refusal came from "Restart now") |
| boot screen | the six lines under "Boot screen" |
| result | `System restored to {date}` |
| result | `System restored · still boots the previous kernel · see README` |
| result | `Restore incomplete · system partly restored · {what to do}` |
| what to do | `reconnect the backup disk` / `free space on the system disk` / `see README` |
| result | `The restore didn't start · nothing was changed · {what to do}` |
| button | `Restore again` |
| result tooltips | the helper's reason, then: `Restored from snapshot {name} ("{comment}"). Home folders were kept. Safety snapshot: {date}.`; `Restored to {date}. The new boot files didn't check out, so the ones it had were put back: it runs on the kernel from before the restore ({version}). The next kernel update sets this right. See "If a restore goes wrong" in Apsis's README.`; `Stopped after 3 tries: {reason}. Reconnect it and click Restore again, or restore the safety snapshot from {date}.` |

### 6b.9 Helper

**The interface becomes `io.github.atraxsrc.Apsis.Helper3`** (helper slice design,
2026-10-01; **confirmed by the owner the same day**, with the raw flags string). `List` has to carry each
snapshot's format for the row tooltip (0.4.1), and `(sss)` -> `(ssss)` is a breaking wire
change; names.rs's rule is "a breaking change gets a new interface". Bus name, object path and
unit stay; the bus policy file, the error prefix (`Helper3.Error`) and the constants change.
A 0.4.x panel process still running after the upgrade gets `UnknownInterface` until it's
re-added (log out and in), as 0.3.x did at 0.4.0; `just deb-install` prints that. The
alternatives and why not: a second list method next to `List` (`ListDetails`) keeps a stale
0.4.x panel working until re-login, but leaves a dead method to carry for ever; a per-snapshot
`a{sv}` dictionary avoids future bumps that nothing planned needs. Shipping applet and helper
in one .deb with `prerm` stopping the helper means the only mismatch ever is that stale panel
process; after a restore the snapshot's own Apsis is installed whole, panel and helper alike.

| method | polkit | |
|---|---|---|
| `List() -> ((sssa(ssss)asas)a{st})` | `list` | as `Helper2`'s, and each snapshot is `(name, tags, comment, rsync_flags)`: the raw `apsis-rsync-flags` value from its `info.json`, `""` when the key is missing (Timeshift's, Apsis <= 0.4.0). The applet asks core (`Info`'s rule, as a function of the string) whether that's the old format; carrying the string, not a bool, means a later flag change (`-H`, 0.5.x) needs no wire change |
| `Create`, `Delete`, `Stop`, `Job`, `ReadConfig`, `WriteConfig` | as now | unchanged |
| `DeleteMany(as names)` | `delete` | the bulk delete as one job (0.4.2 below). If 0.4.2 ships first it's already on `Helper2` and only moves |
| `CheckRestore(s snapshot) -> (b ok, s refusal, b has_home, b has_root, b old_format, s apsis_note)` | `list`, not interactive | Mounts read-only like `List`; the 6b.7 checks; fills the dialog. `refusal` is a stable word per `Refusal` variant (plus the numbers for the space ones), decoded by core for the applet |
| `Restore(s snapshot, b restore_home, b safety_snapshot)` | **`restore`, `auth_admin`, asked every time** | Returns once started. Job kind `restore`, with `JobChanged` progress and `Finished("restore", ok, msg)`; ok means "ready" |
| `RestartToRestore(s snapshot)` | none for the uid that started the plan, else `restore` | Re-checks space, the ESP, the boot files and freshness, arms (unit, drop-in, helper copy, `state.json`, `/system-update`, `sync`), starts the disarm timer, then logind `Reboot(false)` over the system bus (zbus, no new crate) |
| `CancelRestore()` | none for the starter's uid, else `restore` | Removes the plan (not armed yet) |
| `RestoreResult() -> (s state, s snapshot, s message, x when, s home, s safety_snapshot)` | `list` | `ready`, `done`, `problems`, `boot-kept`, `boot-broken`, `not-started`, `failed` or `""`; `home` is `keep` or `restore`, and `""` stands for none. The window asks when it opens (6b.8) |
| `Stop(s snapshot)` (existing) | as now | Also stops a restore's preparation |

**The lock, the ready plan, and the signals** (settles item 7 of the helper slice; the rule
is in 0.4.2 below too):

- **Reads share, writes are exclusive, and reads aren't jobs** (0.4.2 below, built first):
  `List` and `CheckRestore` hold a refcounted read-only mount together and never refuse each
  other; a write (`Create`, `Delete`, `DeleteMany`, `Restore`, `WriteConfig`) waits for the
  readers to finish (bounded) instead of being refused; a read that arrives while a write runs
  or waits is refused `Busy` as today. Reads announce no `JobChanged` and `Job()` doesn't
  report them.
- **The lock is released before the end is announced and before `Finished` is sent** (0.4.2).
  Today `start` announces the end (`JobChanged`) while the lock is still held and releases it
  on the next line; `Finished` already comes after. `Running::end(state)` consumes the guard,
  takes the job out, frees the lock, then announces, so no listener can act on an end and
  find the helper busy. A `State` test holds it: after the end is received, `is_running()` is
  false and `begin` succeeds.
- **A ready plan isn't the lock.** While a plan waits at the ready prompt the helper keeps a
  `ready` state next to the lock. It refuses with `Busy` everything that writes: `Create`,
  `Delete`, `DeleteMany`, `WriteConfig`, another `Restore`, `Stop`. It doesn't refuse reads:
  `List`, `Job`, `ReadConfig`, `CheckRestore`, `RestoreResult` (nothing is mounted while a
  plan is ready, and the snapshot can't be deleted under it since `Delete` is refused). So
  the popup and other windows keep listing instead of showing Busy for up to 30 minutes.
- **The ready plan is a job**: `Job()` says kind `restore`, state `running`, the snapshot's
  name, percent 100, and `JobChanged` announced it when it became ready. Other windows see a
  running job and keep Create and Delete disabled (0.4.0's rule). It ends `stopped` on
  `CancelRestore`, on the disarm timer, on a stale plan's removal and at "too old"; `done`
  right before logind's `Reboot`. No new state word.
- **The helper doesn't idle-exit while a plan is ready**, and it watches the starter's
  unique bus name (`NameOwnerChanged`): when the window that prepared the plan leaves the
  bus without an answer (crash, logout), the plan is removed at once, logged, and the job
  ends `stopped`. The backstop for a helper that was killed meanwhile: at start, a
  `request.json` with no `/system-update` link is a leftover and is removed (6b.5); a window
  that still shows the prompt then gets "The preparation is gone. Start the restore again."
  from `RestartToRestore` (the same handling as "too old").
- `Finished("restore", true, ..)` means ready, and is the one `Finished` that's sent while the
  helper still refuses writes, on purpose: the plan holds them off until the starter
  restarts or cancels.

- New polkit action `io.github.atraxsrc.Apsis.restore`: `auth_admin` for active, inactive and
  any (no `_keep`). That makes six actions; the resource test counts them.
- Input checks as for delete: the name matches `YYYY-MM-DD_HH-MM-SS`, is in a fresh list and
  isn't a leftover, and `snapshots/<name>/localhost/` is reached with `O_NOFOLLOW` from the
  mount. The helper runs every 6b.7 check itself; the applet isn't trusted.
- The starter's uid (`GetConnectionUnixUser`) is kept in `request.json`, so it survives the
  helper's idle exit.
- `/var/lib/apsis/restore/` (root, 0700): `request.json` (snapshot, its `created` from `info.json`, backup UUID, home choice,
  format, safety snapshot name, root UUID, running kernel, space needed per destination
  partition, the separate `/home`'s filesystem UUID when home is restored and `/home` is its
  own mount (else none), starter uid, prepared-at), `restore.filter`, `state.json`, `rsync-log`, `esp-backup/`, `result.json`, and
  the helper copy while armed. serde_json, already a dependency.
- `apsis-helper --apply-restore` is the offline entry point. It uses no D-Bus.
- The disarm timer (6b.5): a transient timer unit started on arm, 10 minutes, in the current
  boot only, calling the helper to disarm. Its service has `Conflicts=shutdown.target`. Arm
  creates `/system-update` last and disarm removes it first; the helper's start and the next
  arm clean leftovers that have no link. Started like the other tools (fixed argv) or over
  the system bus; no new crate. Disarm removes the drop-in with the unit.
- Journal: `restore "2026-09-25_11-28-00" keep-home safety for :1.42: ready`, `armed;
  restarting`; in the offline boot, each step, the attempt number, rsync's summary, both boot
  commands' output, the ESP backup and check results.
- No new crates: rsync, udevadm, kernelstub, plymouth and logind are called like today's
  tools (fixed argv, fixed `PATH`).

**Decided 2026-10-04, not built yet** (the triage; DECISIONS.md, "triage of checks 1 to 9").
Until it is built, the table and the list above are what the code does.
- **The job lock on disk.** The helper holds a `flock` on a file under `/run` for the
  duration of every job (`State::begin` to the job's end), and around the arm. The kernel
  drops the lock with the process, so a killed helper leaves nothing stale; the file's
  existence means nothing. A ready plan holds no lock. **Built (fix 1, row 5)**:
  `/run/apsis/job.lock`, root's, 0600, opened `O_NOFOLLOW | O_CLOEXEC` (a symlink there
  refuses the job as `Busy` with a journal line; no child inherits it), taken with
  `LOCK_EX | LOCK_NB` after the `running` flag and before anything is announced, so a write
  refused by a package script was never a job. After the lock is taken the open file must
  still be the one at the path, else it is opened once more. Released at the job's end
  before the end is announced, and when a plan becomes ready; unlocked explicitly, not only
  closed. Reads, `--disarm` and `--apply-restore` take none. A ready plan is taken to its
  end with the lock (6b.5, row 5).
- **The `prerm`** (`remove`, `upgrade`, `deconfigure`, and the new package's
  `failed-upgrade`), in this order: it takes the lock without waiting and **refuses with
  one line** if a job holds it, so a refused operation changes nothing; it **disarms** an
  armed restore, removing `/system-update` only after checking that its target is Apsis's
  state folder, and failing with the exact manual command if the link can't be removed;
  then it stops the helper, still holding the lock. **`postrm remove`** also removes
  Apsis's link and the unit files; the state folder and the config stay purge-only.
  **Built (fix 1, 1b)**, with the owner's rulings of 2026-10-04:
  - the refusal is `apsis: an Apsis job is running; try again when it has finished`, on
    stderr, exit 75 (`flock -n -E 75`; any other failure to take the lock is exit 1 with
    its own line);
  - after `apsis-helper --disarm` **the link decides, not the exit status** (the open
    point, option (e)): still Apsis's link, exit 1 with `rm /system-update` and `systemctl
    stop apsis-disarm.timer` to run by hand, and neither the timer nor the helper is
    stopped; gone, the script goes on, saying `apsis: the restore that was waiting for a
    restart is cancelled` after a clean disarm, or a warning that the removal may not be on
    disk after a non-zero exit. The helper is run only when the link is Apsis's, so a
    0.4.x helper (no `--disarm`) is never called that way;
  - `--disarm`'s own lines say `disarm:` and name no caller;
  - `postrm` on `remove` and `purge` stops `apsis-disarm.timer` before the unit files go,
    removes Apsis's link (same check), the unit, its wants link and the drop-in, and the two
    folders when empty, then reloads systemd and the bus (the reload used to come before
    purge's removals);
  - the scripts name each path once, as a variable at their top. The helper's tests compare
    those with the helper's constants and run copies of the scripts against a temp folder,
    with a fake `systemctl`, `busctl` and helper; the harness refuses root, a path outside
    its folder, and a command that isn't its fake.
  **Knowingly left** (owner, 2026-10-04): a job begun after the `prerm` has exited, during
  an upgrade's unpack, runs the old binary to its end.
- **`RestoreResult() -> (s state, s snapshot, s message, x when, s home, s
  safety_snapshot)`**: the tooltip's two lines come from `result.json`, not from fixed
  strings. `Helper3` is unshipped, so the signature changes without a new interface.
  **Built (fix 6).**
- **The journal says `refused:` for a refused restore** (`Error::RestoreRefused` in
  `describe_error`), while preparing and at "Restart now".
- **Delete renames first**: after its checks, a delete moves the folder from `snapshots/`
  into `apsis-staging/`, removes the tag links, then removes the folder there. A cut delete
  is a leftover, which the list, Delete and the next create handle. A folder already
  half-deleted in `snapshots/` (a readable `info.json`, no `exclude.list`) is listed as
  such a row. Nothing is removed from `snapshots/` unasked, and a folder there with no
  `info.json` stays refused (a known limit). **Built (fix 2)**, with the owner's rulings: the
  move is `RENAME_NOREPLACE` (any error refuses, nothing moves, no plain-rename fallback),
  then `fsync` of both folders before the first removal; one text for every leftover row,
  "Unfinished snapshot or delete · Delete removes it"; no `incomplete: no exclude.list`
  warning for a folder that is now a row. **Knowingly left** (owner): Timeshift making a
  snapshot in place could show as such a row while it runs (not known whether it ever has
  `info.json` without `exclude.list`); Delete needs a click, a confirm and a password.
  **Knowingly left** (owner, 2026-10-04): a folder half-deleted in place by Apsis 0.4.x or
  Timeshift that still has `info.json` and `exclude.list` is listed as a whole snapshot. It
  can be the `--link-dest` base (costs space only) and it is offered for Restore: a
  full-system restore of that partial tree would remove what it lacks; the safety snapshot
  is the way back. Not detectable reliably; no code.
- **The arm's cleanup also removes the wants folder when it is empty**
  (`remove_arm_files`, as for the drop-in's folder), and `postrm` does the same.
- **The state folder gains** `restore.note` (a working file, like `restore.filter`), and
  `last-restore.filter` and `last-restore.note`, which only an arm writes (6b.5).
- **Knowingly left** (owner): the upgrade from 0.4.2 runs 0.4.2's `prerm`, which has no
  guard; `just uninstall` bypasses the maintainer scripts (one README sentence).

### 6b.10 Failure states

| state | when | what happens | counted as an attempt? |
|---|---|---|---|
| **never started** | Step 1 or 2 fails, or the snapshot check right before the copy: the backup disk isn't there after 60 s, the mount fails, a path check or refusal fails, the snapshot is gone or isn't the plan's (`info.json`), a separate `/home` being restored isn't mounted or has another UUID than the plan's | Nothing is written in this boot. Remove `/system-update`, the unit and the helper copy; write `result.json` `not-started` with the reason; boot normally. The boot screen says "The restore didn't start: the backup disk wasn't found. Starting normally." After login: the result line (6b.8) | **No** |
| **copy broke** | Pass 1 exits with anything but 0, 23 or 24 (disk pulled out, I/O error, disk full), or with 23 and rsync's "skipping file deletion" line, or the power goes during pass 1 | `/system-update` stays. The boot screen says "The restore was interrupted. Restarting to try again (attempt 2 of 3)." and restarts; the next boot runs from step 1 (rsync picks up where the files differ) | **Yes** |
| **boot files failed** | Step 4 or 6 | The ESP files are put back, the protected kernel is kept, `boot-kept` (or `boot-broken`), end (6b.6) | Not retried |

**A result that can't be saved** (owner, 2026-10-01; decided in the state machine slice): if
saving the real `result.json` is refused (a field fails validation), the apply writes a
minimal one instead.
- `result.json` version 1 (unshipped): **`when` may be `null` with any outcome** (the clock
  is its only source; owner, 2026-10-01). **`snapshot` may be `null` only with `failed`.**
- The minimal report **keeps the real outcome**. Its message is "result could not be saved,
  see journal", and `when` is `null` unless the clock gave a time after 1970.
- Only if that's refused as well (the snapshot's name is what's wrong) is the report `failed`
  with no snapshot, and the message then names the real outcome: "... (the restore ended:
  `done`)".
- `result.json`'s time is never checked against the plan's or any other time (a hardware
  clock in local time makes them disagree).
- `RestoreResult` (6b.9) gives `""` and `0` for a `null` snapshot or time, and `""` for no
  safety snapshot.

**Exit 23** (found with real rsync, decided by the owner, 2026-10-01; the fourth case found
in check 6, 2026-10-03). rsync 3.2.7 exits 23, and nothing else, in four cases that aren't
the same:
- some files couldn't be read or written: the rest is copied and deleted as usual. **Plain
  23 with the snapshot still there stays `problems`**, and the apply goes on.
- a whole folder of the snapshot can't be read while the disk stays. From there on rsync
  deletes nothing, and says so on its standard output when it next reaches a folder's
  deletion pass: "IO error encountered -- skipping file deletion". **Exit 23 together with
  that line is a copy that broke**: no boot refresh, the attempt counts, the link stays, and
  it's tried again like any other broken copy. After the third, `failed`, and the message
  says the system may be a mix of the snapshot and what was there before.
- **the backup disk goes away under the copy** (check 6, 2026-10-03, the USB pulled 17 s
  into a 54 s copy): every remaining `readdir` fails at once, the walk collapses within a
  second, and **no deletion pass comes after the error, so the line is never printed**:
  rsync exits a plain 23 with 59 files already deleted. So **after a plain 23 the apply
  looks at the snapshot once more** (`find_snapshot` and `check_snapshot`, the same check as
  before the copy; on ext4 the vanished disk's mount is in its shutdown state and every
  open fails): gone, it's **a copy that broke**, with the reason in the journal; still
  there, it's `problems` as above. Before this the apply refreshed the boot files over a
  tree rsync had walked for a third of its length and reported `problems`, which the window
  shows as `System restored to …`.
  **Decided 2026-10-04, not built yet**: when it's `problems`, the apply writes the last 20
  lines of rsync's standard error to the journal, one line each (`Runner::say`, which in
  the real runner is the journal only), and the window gets a line of its own for
  `problems`. Shown by a unit test and the real-rsync test only: as root on apsis-test a
  plain 23 needs a real I/O error. **Built (item 9).**
- the snapshot's folder is missing altogether: nothing is copied or deleted, and there's no
  such line. rsync can't tell this from the first case, so **the apply checks the snapshot
  itself right before every copy** (`apply::check_snapshot`, pure): its `localhost/` is a
  folder, and its `info.json` reads, has the creation time the plan recorded
  (`snapshot_created`; `info.json` has no name in it), this root's UUID, and the type
  `rsync`. Failing any of these is **never started** (or `failed` after an earlier copy
  wrote): nothing is touched in that boot.

**Never `--ignore-errors`**, for the restore or its dry run: with it rsync would go on
deleting after a read error, when it can't know what the snapshot holds. That refusal to
delete is what the rule above reads. `argv::NEVER` lists it with the other options the
restore never uses, and a test holds it.

**The boot cap** (owner, 2026-10-01): a restore may begin 5 offline boots (`MAX_BOOTS`:
three copies, and two to spare for power cuts). Each boot is counted in `state.json` before
it does anything else, so a helper that's killed or panics in every boot, at any point after
the count, is stopped. The boot that finds the count at 5 gives up: it removes the link
**first**, writes `result.json` (`failed`, "started in 5 boots and ended in none"), saves
step `end`, removes the unit files, and **restarts** (`End::GaveUp`; changed by the owner,
2026-10-01): with the link gone a restart can only reach a normal boot, so it can't loop. If
the link can't be removed it's `End::LinkStuck`, and that still doesn't restart. The ESP backup
is left in place for a recovery by hand, and the result says so if the boot files are
neither the restored kernel's nor the ones from before. What the count can't cover: a helper
that dies in the link check, in reading `state.json`, or in removing the link itself.
**When the apply restarts is decided in core** (`apply`), not in the helper: after every end
where the link is gone (`Finished`, `GaveUp`), and over the link only as a retry (two at
most). Never for `LinkStuck` or `NotArmed`.

**A plan or state that can't be read** (6b.6 step 1): the link is Apsis's, but
`request.json` or `state.json` is missing or refused. Nothing is applied. The arm is removed
and `result.json` says why, with the outcome `failed`: without the plan there's no snapshot
to name (`null`), and without the state it isn't known whether an earlier boot wrote
anything. A link that isn't Apsis's is left alone, with no result at all (6b.6 step 1).

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

**A power cut in the other steps** (state machine slice, 2026-10-01): the next boot runs
steps 1 and 2, then goes on from the step in `state.json`.
- Step `copy`: pass 1 again, as a new attempt.
- Step `boot-files` (the copy ended, the boot refresh hasn't started): the copy is skipped.
  The boot files are still as they were, so the backup may be taken:
  - No `esp-backup/` folder: the first backup, as in step 4.
  - **A folder with no manifest** (the power went during the backup; the manifest is written
    last): the partial folder is removed and **the backup is taken once more** (owner,
    2026-10-01). Only a real folder at that name is removed, never through a link, and never
    one that has a manifest (`esp::remove_partial`).
  - A folder that verifies against its manifest (and is of this root UUID): kept.
- Step `boot-refresh` (the refresh started): **no backup is ever taken again**. The boot
  files may be changed, and a backup now would be of the changed ones.
  - The backup verifies: kept. A temporary file that a cut put-back left on the ESP is
    removed (`esp::clear_temporaries`), then the boot refresh, the check and, if it fails,
    the put-back run again, whole. The ESP ends fully back or fully refreshed, never mixed.
- At either step, a backup that's there and **doesn't verify** (a damaged file, a manifest
  that isn't one; at `boot-refresh` also no manifest or no folder): no backup is taken, **no
  boot refresh is run and nothing is put back**. The outcome is what the ESP boots as it is:
  it passes the check against the restored tree: `done` (or `problems`), with step 7; else
  it's still the kernel from before, whole (`esp::boots_kernel`): `boot-kept`; else
  `boot-broken`.
- Step `end`: only the cleanup.

A test cuts the power before and after every single thing the apply asks of its runner, for
a good restore, a failed boot refresh, a restore that never starts and one with problems:
the boots after it end in the same outcome. Two more cut **inside** a step: a backup with
two files copied and no manifest, and a put-back with two files written and a temporary file
left.

### 6b.11 If a restore breaks booting

The recovery partition and the ESP's `Pop_OS-oldkern` entry are never touched by a restore;
the by-hand steps below rebuild that entry's initrd and rewrite its files on the ESP. The
README has a section, "If a restore goes wrong". A copy with this machine's UUIDs and the
snapshots' names filled in is written to the backup disk (`timeshift/apsis-restore-RECOVER.txt`)
while preparing, atomically, and the arm keeps it as `last-restore.note` in the state folder.
It holds only UUIDs and snapshot names: no user names, no emails.

On an encrypted system disk (0.6; the layout `refusal::encrypted_root` lets through) the
note has three more things, and the plain note stays byte for byte what it was. Before the
root's mount line: `sudo cryptsetup luksOpen /dev/disk/by-uuid/<luks-partition-uuid> <name>`
and `sudo vgchange -ay`, with a paragraph before them and one after. `<name>` is the
mapping's name in the live crypttab (`cryptdata` on a default install): `update-initramfs`
in the chroot looks the root's mapping up in `/etc/crypttab` by name, so a disk opened under
another name (a file manager's `luks-<uuid>`) would get an initrd that can't unlock it. The
paragraph after says what to do when the device is already open under another name. The
note then also holds that name, and says so; a name or UUID that isn't a plain word is
refused before any note is written (`root-device`). A second "Known limit" line says the
unlock lines have run in a recovery once and the "in use" case has not (the drill on
apsis-test, 2026-10-06).

1. At power-on, hold Space for the systemd-boot menu and pick **Pop!_OS Recovery**, or boot a
   Pop!_OS live USB of the same version (the recovery opens an installer window; its install
   choices are a reinstall).
2. In a terminal, one line at a time; a line that prints an error stops the rest:
   ```
   sudo mount /dev/disk/by-uuid/<root-uuid> /mnt
   sudo mount /dev/disk/by-uuid/<esp-uuid> /mnt/boot/efi
   sudo mkdir -p /media/backup
   sudo mount -o ro /dev/disk/by-uuid/<backup-uuid> \
     /media/backup
   # one of: (a) the same snapshot again, (b) the safety snapshot
   sudo rsync -a -A -X --numeric-ids --delete --force --sparse \
     --exclude-from=/mnt/var/lib/apsis/restore/last-restore.filter \
     /media/backup/timeshift/snapshots/<snapshot or safety-snapshot>/localhost/ /mnt/
   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done
   sudo chroot /mnt update-initramfs -u -k all
   sudo chroot /mnt kernelstub --verbose
   sudo rm -f /mnt/system-update
   ```
   `RECOVER.txt` has both rsync commands, labelled, with the names filled in (with no safety
   snapshot, one line says so in place of (b)), and a line after them: without the filter,
   stop. `-A -X` only for a new-format snapshot; a safety snapshot always is. The filter's
   rules are anchored at the transfer root, so they work unchanged against `/mnt/`. Its rule
   5 lines name the installed system's mount points, which are right for `/mnt` too.
   `--rbind /sys` brings `efivars` for kernelstub. `--numeric-ids` matters here: the live
   system's user database isn't the installed one. The README adds the check that the
   result boots: two `cmp` lines, the ESP's `vmlinuz.efi` and `initrd.img` against what
   `/boot/vmlinuz` and `/boot/initrd.img` link to (each about 100 columns, so not in the
   note).
3. Restart.

Timeshift from a live USB can also read these snapshots, but its boot refresh isn't
kernelstub's, so the README gives the steps above.

**Decided 2026-10-04, built in steps 8 and 9** (check 8's findings; DECISIONS.md, "triage of
checks 1 to 9").
- **What the drill showed, and what it didn't.** Drilled (check 8): the same kernel on both
  sides, Pop's recovery partition, the safety snapshot. Not drilled: a by-hand go-back
  after a restore across a kernel change, where the saved filter keeps out the kernel that
  ran at the preparation, and finishing such a restore by hand. For 0.5.0 the README says
  so, names the kernel case as a known limit without promising an outcome, and says that
  when the system still starts from either boot entry the way back is the window. **Built
  (step 9).** A filter without the kernel lines for by-hand use, with its own by-hand
  rollback drill, is 0.5.x.
- **The first sentence above is wrong for these steps themselves**: the restore never
  touches the recovery partition or the `Pop_OS-oldkern` entry, but the by-hand lines
  rebuild both initrds and rewrite the previous pair on the ESP. The README and the note
  say that; `update-initramfs -u -k all` stays, as drilled. **Built (steps 8 and 9).**
- **The note carries two complete, labelled commands**, "restore the same snapshot again"
  and "go back to the safety snapshot", each with its own flags (a safety snapshot is
  always the new format). With no safety snapshot it says so in one line. It has one
  caveat line about the kernel limit. Nobody has to edit a line. **Built (step 8).**
- **Both commands and the README name `last-restore.filter`** (6b.5), not
  `restore.filter`. Without that file rsync exits 11 before it copies or deletes anything,
  and the README says so. **Built (steps 8 and 9).**
- **The note is written atomically** on the backup disk, and the arm keeps its text as
  `last-restore.note` in the state folder, readable after the first mount line. The note
  on the backup disk is written at every preparation, so it carries one line saying which
  to trust: the pair in the state folder, which is the last arm's. **Built (step 8).**
- **The README also gains** what check 8 had to supply: the UUIDs from `lsblk -f`; what to
  do if the backup disk is mounted already; that the recovery opens an installer window
  whose install choices are a reinstall; that a failing line means stop; the two compare
  lines that show the ESP's current pair is what `/boot` links to; that no ESP backup is
  made by hand; that `update-initramfs` takes about two minutes; the panel's restart with
  nothing unmounted; and that `result.json` still names the earlier restore afterwards.
  **Built (step 9).**
- **Knowingly left** (owner, 2026-10-04): after restore 1 runs, a restore 2 that is armed
  and then disarmed replaces the kept pair and `RECOVER.txt`. The note then names restore
  2's snapshot and its safety snapshot. Restore 1's safety snapshot stays on the disk as a
  list row with its comment, but the note no longer names it.

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
  being restored is checked on its own filesystem; the ESP's needs with the owner's real
  sizes and free space.
- **ESP**: the file set is the seven of 6b.6 step 4 and nothing else; the optional three
  together or not at all; the check against a live and a restored tree, with a previous
  pair that's reported and never fails it; a put-back leaves the recovery entry,
  `loader.conf`, the random seed, `entries.srel`, `EFI/BOOT` and `EFI/systemd` alone.
- **Apply state machine** (fake runner and fake ESP, temp root): never started does not count
  and removes the link (also for a separate `/home` that's missing or has another UUID, with
  rsync never run, and for a snapshot that's gone or isn't the plan's, checked before every
  copy); copy broke counts and keeps it, also exit 23 with the deletions skipped; the third break gives up with `failed`;
  23 vs 11; ESP backup, a failed check puts the files back and keeps the protected kernel
  (`boot-kept`); a put-back that doesn't compare gives `boot-broken`; cleanup only after a
  passed check and only when the snapshot lacks the kernel; a power cut at each step
  (going on from the saved step) ends in the same result, and so does a cut inside the
  backup or the put-back, with the ESP never left mixed; a backup cut before the boot
  refresh started is taken once more, and never after; a plan of any age is applied (no
  clock is read); an unreadable plan or state disarms and reports why; another tool's link
  is left alone; a link that can't be removed gets no restart; a refused `result.json` is
  replaced by the minimal report (the same outcome, no time); every boot is counted first,
  a helper that dies at any point after the count ends within the cap, and the cap ends
  with one restart once the link is gone and none over a link that's stuck; no end leaves a `.apsis-tmp` file on the ESP (checked after every
  apply in every test).
- **Real rsync** on temp trees (`tests/restore.rs`, as the tester, no root; under
  `target/tmp/restore/`): core's own argv (`argv::rsync`, `argv::rsync_dry_run`) and filter
  (`filter::rules`), run with a real rsync from a fake snapshot onto a fake live root, in the
  C locale. They run in the normal `cargo test`: CI installs rsync (`ci.yml`), so there's no
  gate like `test-ext4`'s; without rsync each test skips with a message.
  - Everything on the protect list, and every excluded path (the ESP, `/recovery`, fstab and
    crypttab, the runtime folders, the journal, other mounts), is the same file after
    (inode, time, bytes), though the snapshot has other files at those names. The fixed
    rules hold with nothing mounted but `/`.
  - What the snapshot lacks is deleted; changed files come back byte for byte, with their
    time; a file that's the same isn't rewritten. Links come back as links, and a live link
    where the snapshot has a file or a folder is replaced, never written through. A FIFO
    comes back as a FIFO.
  - Home kept (untouched, including `/home`'s own mode and time) and restored, each also
    with `/home` as a mount of its own; paths the snapshot's `exclude.list` left out stay.
  - The booted kernel's modules folder and four `/boot` files survive pass 1 as the same
    files; without rule 10 they're deleted.
  - A copy killed partway (the whole process group, mid-file) and run again ends with the
    same tree as an uncut copy; rsync's leftover temporary file is gone.
  - Real exits mapped by core (`Copied::end`): 0, plain 23 (a file that can't be read:
    `problems`), 23 with the deletions skipped (a folder that can't be read: a copy that
    broke), 24 (a file that vanished), 11 (no filter file), 20 (SIGTERM), none (SIGKILL).
  - A snapshot that's gone gives plain 23 from rsync, and is what `check_snapshot` refuses
    before the copy, on the same lab; another snapshot at the same name is refused too.
  - The dry run's size read by `space::dry_run_size` from real output, equal to what the
    files add up to and to the real run's; nothing written. Human-readable output, and a dry
    run that failed, refuse.
  - A new-format restore brings back a `user.*` xattr and an ACL; an old-format one doesn't
    strip a live xattr from an unchanged file (with `-X` it would: both are run). These skip
    with a message on a filesystem that stores neither.
  - **Not shown without root**, as ignored tests with the reason (check 11 below): owners by
    number, a device node, a file capability. **They stay ignored**: check 11 showed all
    three on apsis-test's real root (2026-10-03), with an ACL and a `user.*` attribute on a
    root-owned file, so nobody is asked for a root run of the tests.
- **Unit text** and its install path; **the drop-in's text and path** (the condition is
  `!/system-update/apsis-helper`, the path is under `/etc/systemd/system/`, and it is on the
  protect list and not in the package); **RECOVER.txt** has only UUIDs and the snapshot name.
- **Refusals, added in the helper slice**: `check_pending` for each of the three Pop names
  (one at a time, and a name that is a folder or a dangling link counts; fwupd's and
  PackageKit's files present don't refuse); `crypttab_differs` on
  byte-equal files, a comment or blank-line change (same), a field change (differs), a
  missing snapshot crypttab against an empty live one (same); the hook flag rule of
  `KernelIncomplete` (present, missing, without the flag).
- **Helper**: introspection (`Helper3`, the new methods, `(ssss)` in the list), policy file
  (6 actions, `restore` is `auth_admin` everywhere), the lock is free when an end is announced
  (`Running::end`), Busy for writes and not for reads while a plan is ready, the ready plan as
  a job, the starter exemption for restart and cancel, the expired plan, the plan removed when
  the starter leaves the bus (`State` level: the bus name is handed in), the disarm timer
  (started on arm; disarming removes the arm, the drop-in and `request.json`), the order
  (the link made last and removed first; the drop-in before the link), leftovers without the
  link cleaned at start, `Conflicts=shutdown.target` in the disarm service's text, the
  rsync runner hands core every line of standard output and the exit, `udevadm wait`'s argv.
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
- **The baseline of 2026-10-01 holds Apsis 0.4.1**, so every restore from it reinstalls 0.4.1:
  that is check 9's case. **Take a fresh baseline once 0.5.0 is installed on apsis-test**, for
  checks 1 to 8 (owner, 2026-10-01).
- **Baseline first.** Take the baseline snapshot only after apsis-test has, in its system files:
  the Wi-Fi connection, the `sudoers.d` entry the SSH checks use, and the test-only polkit rule
  that lets the SSH session call the helper. Every snapshot the tests restore must be taken
  after that. An earlier one would roll them back, and the restore would cut the SSH link it's
  being watched through. The sudoers and polkit files live only on apsis-test: never in the
  repo, never in the .deb, and removed when testing ends. Their exact content is the owner's
  (it names the test account). The Wi-Fi connection is a file under `/etc/netplan`, not under
  `/etc/NetworkManager/system-connections` (found by check 9): a runbook that compares the
  harness looks there.
- A small read-only `tools/restore-check.sh`, run with sudo after each restore: ESP vs `/boot`
  hashes, modules for `uname -r`, `dpkg --audit`, `/system-update` gone, the unit's journal,
  `result.json`, and (0.6) the unlock pieces in the ESP's initrd and the entry's `root=`. A VM
  harness (OVMF, a Pop!_OS install) isn't worth it for 0.5.0: it needs root and KVM, and the
  risk is Pop's boot chain on real hardware, which apsis-test is.
- **Check 11's files stay on apsis-test** (found 2026-10-03): `/opt/apsis-check.txt` (an ACL
  and a `user.*` attribute), `/opt/apsis-test-ids` (owner `54321:54322`) and
  `/opt/apsis-test-null` (a device node) are live and inside "6b baseline" and "baseline
  0.5.0", not inside "baseline1". No runbook creates files of those names or cleans `/opt`;
  restoring "baseline1" deletes them from the live system (check 9 picks its snapshot
  knowing that). Before a runbook has the owner create files, it has them list what is
  there.
- **A runbook script never guesses its mode** (check 11, 2026-10-03: a skipped step left the
  snapshot's name empty, the script fell back to looking at the live system, and its pass
  word was printed for a snapshot that didn't exist). Three rules for every runbook from
  now on:
  1. A script that can look at the live system or at a snapshot takes its mode as a
     required first argument, the word `live` or a snapshot name. No argument is an error.
  2. Every command that uses a name set in an earlier step fails in the owner's shell when
     the name is empty (`${snap:?...}`), and the step that sets it ends with a command that
     shows the name and that the folder is on the backup disk.
  3. A pass word is printed only by a command that first finds, in the log it compares,
     the header naming what was looked at (`== inside snapshot <name>`). And a step that
     changes apsis-test is chained to the pass word of the step before it, so it can't run
     after a step that was skipped.

Checks:
- **0.1** Facts: `sudo ls -la <backup>/timeshift/snapshots/<name>/localhost/boot/efi
  .../localhost/recovery`; `ls /etc/initramfs/post-update.d/`; `cat
  /etc/kernelstub/configuration`; `ls /usr/lib/systemd/system-generators/ | grep update`;
  `plymouth --help | grep system-update`.
- **0.2** The recovery partition boots (hold Space, Pop!_OS Recovery).
- **0.3** Offline-mode spike, **before the helper slice**: a dummy unit that only logs, armed by
  hand with a `/system-update` link. It runs, the desktop doesn't start, the unit removes the
  link and restarts, and the next boot is normal. This is the condition on 6b.1.
  **Passed, but contaminated** (2026-10-01): `pop-upgrade-init` ran next to the dummy unit
  (6b.6), so the plymouth and console observations are redone in 0.4.
- **0.4** (added 2026-10-01; corrected after review the same day) The spike once more, **with
  the drop-in in place by hand**, before the helper slice is built.
  **Passed (apsis-test, 2026-10-01, 21:59 AEST; DECISIONS "check 0.4").** The offline boot
  lasted 2 s; every spike line is there (the USB mounted read-only, 2 snapshots);
  `pop-upgrade-init` was skipped on the drop-in's condition, logged at info; nothing of
  `upgrade.sh` ran; `system-update-cleanup` didn't run; the next boot was normal with 0 failed
  units; the cleanup verified clean. The screen showed the plymouth text and progress; the boot
  was fast, as in 0.3, and rebooted normally; **the splash was clean, no unit text over it**
  (owner looked). The `StandardOutput=journal` observation 0.3 couldn't make is made. 0.3's unit, script and
  `/var/lib/apsis-spike` were removed after 0.3, so 0.4 re-creates them. The names below are
  0.4's (`apsis-spike.service`, `/var/lib/apsis-spike/spike.sh`); if 0.3's were different,
  either works. The drop-in's condition is the real one, `!/system-update/apsis-helper`, so the
  spike's folder holds an empty file named `apsis-helper`. Every step is the owner's (root);
  all of it is undone at the end.
  **Not a vacuous pass**: the spike removes `/system-update` first, and `pop-upgrade-init` has
  its own `ConditionPathExists=/system-update`, so if `pop-upgrade-init` were evaluated after
  the spike it would be skipped on its own condition and the drop-in would never be consulted.
  The spike unit is therefore ordered `After=pop-upgrade-init.service`: when the drop-in is
  evaluated the link is there, only the drop-in's condition can skip the unit, and the pass
  needs the journal line that names it. If the drop-in doesn't take, `pop-upgrade-init` runs
  `upgrade.sh` and the spike waits for it (the 0.3 incident); the cleanup is 0.3's: unmask
  `acpid` and `pop-upgrade`, remove `/upgrade-attempted`.
  ```sh
  # 0.4a: the spike's folder, marker and script (writes under /var/lib/apsis-spike)
  sudo mkdir -p /var/lib/apsis-spike
  sudo touch /var/lib/apsis-spike/apsis-helper
  sudo tee /var/lib/apsis-spike/spike.sh >/dev/null <<'EOF'
  #!/bin/sh
  # Apsis check 0.4: the offline-boot spike with the pop-upgrade-init drop-in in place.
  log() { echo "apsis-spike: $*"; }
  target=$(readlink /system-update || true)
  log "link -> ${target:-none}"
  if [ "$target" != "/var/lib/apsis-spike" ]; then log "not ours, leaving it"; exit 0; fi
  # Remove the link first, so no failure below can loop the boot.
  rm -v /system-update
  plymouth display-message --text="Apsis check 0.4: offline boot" 2>/dev/null || log "no plymouth"
  plymouth system-update --progress=42 2>/dev/null || true
  log "packagekit: $(systemctl show packagekit-offline-update.service -p ActiveState -p Result | tr '\n' ' ')"
  log "fwupd: $(systemctl show fwupd-offline-update.service -p ActiveState -p Result | tr '\n' ' ')"
  log "display manager: $(systemctl is-active cosmic-greeter.service gdm.service 2>/dev/null | tr '\n' ' ')"
  log "/: $(findmnt -no FSTYPE,OPTIONS /) ; /boot/efi: $(findmnt -no FSTYPE,OPTIONS /boot/efi)"
  uuid=<the test USB's UUID, hard-coded; the owner's runbook has it>
  if udevadm wait --timeout=60 "/dev/disk/by-uuid/$uuid"; then
    log "backup disk present"
    mkdir -p /run/apsis-spike
    if mount -o ro,nosuid,nodev,noexec "/dev/disk/by-uuid/$uuid" /run/apsis-spike; then
      log "mounted read-only: $(ls /run/apsis-spike/timeshift/snapshots | wc -l) snapshots"
      umount /run/apsis-spike
    else log "mount failed"; fi
  else log "backup disk not found in 60 s"; fi
  ls -la /upgrade-attempted /pop-upgrade 2>&1 | sed 's/^/apsis-spike: /'
  log "done, restarting"
  systemctl reboot --no-block
  EOF
  sudo chmod 0755 /var/lib/apsis-spike/spike.sh
  # 0.4b: the spike's unit, as 0.3's but with StandardOutput=journal (the unit text of 6b.6)
  # and After=pop-upgrade-init.service (see above). Note first whether
  # /etc/systemd/system/system-update.target.wants already exists (0.4f).
  ls -ld /etc/systemd/system/system-update.target.wants 2>&1
  sudo tee /etc/systemd/system/apsis-spike.service >/dev/null <<'EOF'
  [Unit]
  Description=Apsis check 0.4: offline-boot spike
  DefaultDependencies=no
  Requires=sysinit.target
  After=sysinit.target system-update-pre.target local-fs.target pop-upgrade-init.service
  Before=system-update.target shutdown.target
  ConditionPathIsSymbolicLink=/system-update
  FailureAction=reboot

  [Service]
  Type=oneshot
  ExecStart=/var/lib/apsis-spike/spike.sh
  StandardOutput=journal

  [Install]
  WantedBy=system-update.target
  EOF
  sudo mkdir -p /etc/systemd/system/system-update.target.wants
  sudo ln -sf /etc/systemd/system/apsis-spike.service /etc/systemd/system/system-update.target.wants/apsis-spike.service
  # 0.4c: the drop-in, exactly as the helper will write it
  sudo mkdir -p /etc/systemd/system/pop-upgrade-init.service.d
  sudo tee /etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf >/dev/null <<'EOF'
  [Unit]
  ConditionPathExists=!/system-update/apsis-helper
  EOF
  sudo systemctl daemon-reload
  systemctl show pop-upgrade-init.service -p DropInPaths                 # the drop-in (ConditionPathExists isn't a show property)
  systemctl cat pop-upgrade-init.service                                   # both conditions loaded: this is the check
  systemctl show pop-upgrade-init.service -p After -p Before -p WantedBy   # no cycle with the spike; system-update.target pulls it in
  systemctl is-enabled acpid pop-upgrade; sudo tail -3 /var/log/upgrade.log 2>&1   # the baseline 0.4e compares with
  # 0.4d: arm and reboot (record the boot id first; the wait is on a changed boot id, not on ssh alone)
  sudo ln -s /var/lib/apsis-spike /system-update
  ls -la /system-update /system-update/apsis-helper      # the marker resolves through the link
  cat /proc/sys/kernel/random/boot_id
  sudo systemctl reboot
  # 0.4e: after the two boots, from the offline boot's journal (-b -1: holds only with exactly
  # one reboot after arming; else journalctl --list-boots). Save all of it to a file first:
  # the cleanup deletes spike.sh and the marker.
  journalctl --list-boots
  journalctl -b -1 --no-pager -u apsis-spike.service
  journalctl -b -1 --no-pager -u pop-upgrade-init.service   # MUST show the skip naming ConditionPathExists=!/system-update/apsis-helper; no lines is not a pass
  journalctl -b -1 --no-pager -g 'pop-upgrade-init'          # the same line, found without -u
  journalctl -b -1 --no-pager | grep -Ei 'upgrade\.sh|apt-get|system-upgrade|system-update-cleanup'
  journalctl -b -1 --no-pager -u packagekit-offline-update.service -u fwupd-offline-update.service   # both start and finish at once: packagekit "another framework set up the trigger"
  systemctl is-enabled acpid pop-upgrade; ls -la /upgrade-attempted /system-update 2>&1   # as the 0.4c baseline; neither file
  tail -5 /var/log/upgrade.log 2>&1                               # nothing new from this boot
  ls -la /run/apsis-spike 2>&1; findmnt -S UUID=<the test USB's UUID> || echo usb-not-mounted   # /run is tmpfs: No such file; not mounted
  # the drop-in's condition, evaluated now, with no link:
  systemd-analyze condition 'ConditionPathExists=!/system-update/apsis-helper'; echo rc=$?    # rc=0
  # 0.4f: clean up, as after 0.3
  sudo rm /etc/systemd/system/system-update.target.wants/apsis-spike.service /etc/systemd/system/apsis-spike.service
  sudo rm /etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf
  sudo rmdir /etc/systemd/system/pop-upgrade-init.service.d
  sudo rm -r /var/lib/apsis-spike
  sudo rm -f /system-update
  sudo systemctl daemon-reload
  ls -ld /etc/systemd/system/system-update.target.wants 2>&1   # harmless if left empty; rmdir it if 0.4b found it absent
  systemctl show pop-upgrade-init.service -p ConditionPathExists -p DropInPaths   # the unit's own condition only
  systemctl is-enabled acpid pop-upgrade                       # as the 0.4c baseline
  ```
  **0.4c settles the drop-in claim** (6b.13's table) before any reboot: `systemctl cat` must
  show the unit's own condition and the drop-in's both loaded, and `DropInPaths` the drop-in.
  (`ConditionPathExists` is not a `systemctl show` property on systemd 255.4: `show` prints
  nothing for it, which is not "replaced"; found in 0.4.)
  **Passes when**: the spike ran in the offline boot (its journal lines, the backup disk
  mounted read-only); `pop-upgrade-init.service`'s journal for that boot has the skip line
  naming `ConditionPathExists=!/system-update/apsis-helper` (its own
  `ConditionPathExists=/system-update` was true at the time: the spike runs after it); nothing
  of `upgrade.sh` ran (`/var/log/upgrade.log` unchanged, `acpid` and `pop-upgrade` as in the
  0.4c baseline, no `/upgrade-attempted`); the desktop stayed down; the next boot is normal;
  the plymouth text and progress were shown and **no unit text appeared over the splash**
  (that's the `StandardOutput=journal` observation 0.3 couldn't make cleanly). Also noted for
  the record: whether `system-update-cleanup.service` ran (it shouldn't: the spike removed the
  link), and the skip line's journal priority (0.4: 6, info; found with `-u`).
- 1. Keep home, safety on: after the snapshot, add `/etc/apsis-test-marker`, a file in `~`, and
  `sudo apt install cowsay`. Restore: the marker and cowsay are gone, the `~` file is kept and
  `/home`'s mode is unchanged, `dpkg --audit` is clean, `swapon --show` shows cryptswap, `getcap
  /usr/bin/ping` is right (new-format snapshot), the journal shows each step, and the safety
  snapshot is listed. **Flatpak** (decision 6, `-H` off): `flatpak --user list`, launch one
  app, `du -sh ~/.local/share/flatpak` before and after (apsis-test's Flatpak is the user
  install; the system one is empty there).
  **Passed (apsis-test, 2026-10-02, run 3; DECISIONS "check 1 passed").** Three runs: run 1
  found four bugs before the copy (the quiet runner's empty stdout, the safety dry run's
  exclude list, the apply counting its own link as a pending update, the drop-in folder and
  the plan left behind), run 2 restored a safety snapshot picked by mistake and found the
  boot screen showing the journal's lines and the unit killed by TERM at the restart, run 3
  passed every line: the copy 53 s, rsync exit 0, the splash with the one line for the whole
  boot, `Deactivated successfully`, the marker and cowsay gone, the kept file there, every
  "before" number unchanged. Left from the list: launching one Flatpak app after the restore
  (the owner, any time before check 3).
- 2. **Kernel rollback** (the central risk): snapshot, kernel update, restart on the new kernel,
  restore. `uname -r` is the snapshot's kernel, the ESP hashes match `/boot`, and the new
  kernel's modules are gone.
  **Passed (apsis-test, 2026-10-02; DECISIONS "check 2 passed").** The setup itself found the
  bug: kernelstub takes the newest kernel in `/boot`, which after the copy is the protected
  running one, so the apply now names the snapshot's kernel (step 5 above). With that build:
  a mainline 7.2.6 as the new kernel (no Pop kernel was on offer, and 7.1.5 can't be removed
  without `pop-desktop`), the baseline restored, `uname -r` 7.1.5, the ESP byte for byte
  `/boot`'s, `removed kernel 7.2.6...`, 7.2.6's modules and packages gone, `done` in 62 s,
  `Deactivated successfully`. The result carries the designed report about the previous
  pair (kernelstub skipped it as the same kernel; the ESP's old previous stays until the
  next kernel update); the window shows a plain `System restored to`.
- 3. Restore home too: the `~` file is gone, and it's in the safety snapshot. **Flatpak** as in
  check 1: `flatpak --user list`, launch one app, `du -sh ~/.local/share/flatpak` before and
  after.
  **Passed (apsis-test, 2026-10-02; DECISIONS "check 3 passed").** The baseline with "Restore
  them too": two home files and a `.bashrc` line made after it gone and in the safety
  snapshot; the Flatpak list and size the same before and after (1.9G), the app starts; the
  copy 67 s, `done`, `home: restore`. apsis-test's snapshot settings include home, so the
  "home added to the safety snapshot" branch wasn't exercised by it (the earlier safety
  snapshots had home too).
- 4. Stop during the safety snapshot; Cancel at the ready prompt; after a cancel, a normal
  restart doesn't restore. Fill the system disk between ready and Restart now: refused with
  the one line.
  **Passed (apsis-test, 2026-10-03; DECISIONS "check 4 passed").** Nothing restored and
  nothing left armed at any of the four; the journal and status wording differ from the
  design in three places, noted there and deferred.
- 5. Power cut at about 30% on the boot screen: attempt 2 finishes it.
  **Passed (apsis-test, 2026-10-03; DECISIONS "check 5 passed").** The power button held
  about 20 s into the copy; the next power-on came back to the restore by itself; the retry
  boot's `copying, attempt 2 of 3` (the only proof of the count: `state.json` goes with the
  cleanup, and the cut boot's journal kept only its first second), the copy 55 s, `done`,
  both markers gone, restore-check clean, `pop-upgrade-init` skipped in the retry boot
  (item 12's first part). 3 min 28 s from Restart now to the desktop.
- 6. Backup disk unplugged before Restart now's reboot: never started, normal boot, the
  message after login. Unplugged at about 30%: copy broke. Replugged: attempt 2 finishes.
  Left out: "didn't finish", and Restore again finishes it.
  **Passed (apsis-test, 2026-10-03; DECISIONS "check 6 Part A passed…" and "check 6
  passed").** The pull is after `armed; restarting` (the arm reads the snapshot's `/boot`
  sizes from the disk): the offline boot waited 60 s, `not-started`, nothing changed, the
  window's line after login. The pull 13 s into the copy: `rsync exited 23`, `the copy
  broke (…)`, `Retry { attempt: 2 }`, the replugged disk found at the retry boot's start,
  `attempt 2 of 3`, `done`, `pop-upgrade-init` skipped in both boots (item 12). **Found and
  fixed on the way** (commit `62b1560`): a disk that vanishes mid-copy can give rsync a
  plain 23 without its "skipping file deletion" line, and the apply refreshed the boot files
  over a mixed tree as `problems`; after a plain 23 the snapshot is now checked again.
- 7. Boot files failed, simulated by the owner (e.g. a `kernelstub` that exits 1 on the test
  machine, put back afterwards): the ESP files are back, the newer kernel boots, `boot-kept` shown.
  **Passed (apsis-test, 2026-10-03; DECISIONS "check 7 passed (and check 12)").** The
  stand-in was `/usr/local/sbin/kernelstub`, bind-mounted onto itself before the
  preparation so the filter's mount rule kept it through the copy (a stand-in placed live
  is overwritten or deleted before the call). `rsync exited 0`, `the boot refresh failed
  (kernelstub exited with code 1: …): putting the boot files back`, `boot-kept`,
  `Finished(BootKept)`; the ESP's seven files hash as before, `cmdline` included, which the
  stand-in had changed; kernel 7.1.5 still boots; both markers gone; the window's line and
  tooltip as designed. The copy 55.6 s, the put-back 5.2 s, the unit 65 s. The kernel was
  the same on both sides, so `boot-kept` across a kernel change isn't shown by it.
- 8. Recovery drill: from the recovery partition, the README steps with the safety snapshot.
  **Passed (apsis-test, 2026-10-03, the last check; DECISIONS "check 8 passed").** In two
  sittings. First the baseline `2026-10-02_10-50-46` through the window (keep home, the
  safety snapshot on; `done`, the copy 55 s), with `RECOVER.txt` on the backup disk read
  back and equal to `recover::text` byte for byte. Then 6b.11's lines typed in Pop's
  recovery with the safety snapshot's name: the three mounts, the rsync line with the
  saved `restore.filter` (a further pass as a dry run found nothing left), the four
  `--rbind` mounts, `update-initramfs -u -k all` and `kernelstub --verbose` in the chroot,
  `rm -f /mnt/system-update` (a no-op, nothing was armed). The machine started by itself,
  ssh answered, the system was the safety snapshot's (both markers, cowsay and the current
  build back; home, the config, `/var/lib/apsis`, `/opt` and the harness untouched), the
  ESP's current pair was what `/boot` links to, and Apsis listed and checked a restore.
  The USB ended with its three snapshots and the free bytes of before. What the run found
  about the steps, none of it fixed here: `RECOVER.txt` doesn't name the safety snapshot
  and sits on the disk whose mount line it holds; the lines rebuild both initrds and
  rewrite the ESP's previous pair, so "Pop_OS-oldkern is never touched" isn't true of the
  recovery steps themselves; kernelstub in the chroot copies the recovery's command line
  into the ESP's `cmdline` file (nothing boots from it); `update-initramfs` reached
  kernelstub once per kernel, not twice (6b.6's "five ESP writes" were three kernelstub
  runs; the third changed nothing); by hand there is no ESP backup and no check that the
  result boots; the window keeps naming the restore that was undone. Read from the code
  and not exercised (same kernel, both
  snapshots new format): after a kernel rollback the saved filter keeps out the kernel
  the safety snapshot would bring back, and the note's `-A -X` follows the restored
  snapshot's format. Not shown: a system that doesn't start, a restore that broke
  mid-copy, a reinstall. Not looked at: the boot menu's entries word for word, the
  window's lines after the drill. The recovery there: Pop!_OS 24.04 on kernel 7.0.11 (the
  installed system's previous kernel), rsync 3.2.7 with ACLs and xattrs, sudo without a
  password, no network and no ssh server, the backup USB not automounted.
- 9. Snapshot with Apsis 0.4.x: the dialog line; afterwards 0.4.x runs with the protected
  config; reinstalling 0.5 shows the result.
  **Passed (apsis-test, 2026-10-03; DECISIONS "check 9 passed").** Without the version bump
  (owner): the dialog's line comes from the snapshot's dpkg record, and the same build said
  `This snapshot has Apsis 0.4.1. Install Apsis 0.5 again afterwards to restore again.` for
  "6b baseline" and the same with 0.4.2 for "baseline 0.5.0" (`apsis no-restore:0.4.1` and
  `no-restore:0.4.2` in the journal). The snapshot differed from the live system in one
  package, apsis; same kernel, same harness. The restore: `rsync exited 0` after 58 s,
  `done`, `Finished(Done)`; afterwards dpkg `0.4.1-1`, both binaries hashing as the
  snapshot's, `Helper2`, no `restore` action. 0.4.1 started, read the version 2 config,
  listed four snapshots and left `/var/lib/apsis` byte for byte alone. `apt install` of the
  current .deb over it (an upgrade, no script error), a log out and in: `System restored to
  2026-10-01 17:19`. The way back was a restore of "baseline 0.5.0" and a reinstall of the
  current build; the USB ended with its three snapshots and the free bytes of before. Two
  limits: the config in the snapshot was byte for byte the live one, so its protection is
  shown by the filter's rule and the temp-tree test, not by this run; and what needs the
  bump (dpkg saying 0.5.0 after the install, a snapshot with no Apsis line) moved to the
  release gate. Four new findings for the triage are in the entry, the first a backup disk
  the desktop had mounted (the lists failed with the raw mount error).
- 10. `pkaction --verbose --action-id io.github.atraxsrc.Apsis.restore`: `auth_admin`.
- 12. (added 2026-10-01) **pop-upgrade-init stays off**: in check 5's or 6's retry boot, the
  journal of that boot shows `pop-upgrade-init.service` skipped on its condition; after any
  finished restore, `systemctl is-enabled acpid pop-upgrade` aren't `masked`, no
  `/upgrade-attempted`, and `ls /etc/systemd/system/pop-upgrade-init.service.d/` is empty or
  gone. Then `sudo touch /pop-upgrade` and Restore: refused with the Pop line; `sudo rm
  /pop-upgrade` afterwards.
  **Passed (apsis-test, 2026-10-03, with check 7; DECISIONS "check 7 passed (and check
  12)").** The skip on `ConditionPathExists=!/system-update/apsis-helper` in the retry boots
  of checks 5 and 6 and in check 7's offline boot; after check 7's restore `acpid` and
  `pop-upgrade` `disabled`, no `/upgrade-attempted`, the drop-in folder gone; with
  `/pop-upgrade` present the dialog refused with the Pop line (`check-restore … refused:
  pop-upgrade-pending`). The same refusal at Restart now wasn't run: it is the regression
  test for fixes 3 and 4.
- 13. (added 2026-10-01) **Bulk delete** with both monitors on (two panel applet processes)
  and the window open: delete three snapshots at once from the window; all three go, no
  "Busy" line, and `journalctl -u apsis-helper` shows one `delete-many` job, then **one
  `list` per Apsis process (three), none refused** (0.4.2). On 0.4.1 the journal showed the
  cause: six `list` calls from six bus names in the same second after every job end, about
  half refused busy by each other, and the second delete refused because a list held the
  lock; the three processes were the two per-output applets and the window.
  **Passed on 0.4.2 (2026-10-01); to be rerun on 0.5.0** at the release gate (`DeleteMany`
  moved to `Helper3`, and the fix for a half-deleted snapshot touches the delete; DECISIONS
  2026-10-03, triage). A partial data point from check 7's preparation (2026-10-03, the
  0.5.0 build, both applets and the window open): five snapshots in one `delete-many` job,
  `done`, one list per client afterwards; whether a "Busy" line showed wasn't observed.
- 11. **What the temp-tree tests can't show without root** (added 2026-10-01). Before the
  snapshot the restore goes back to, as root: `chown 54321:54322` a file under `/opt` (ids no
  user database names); `mknod /opt/apsis-test-null c 1 3`; note `getcap /usr/bin/ping`.
  After the snapshot: `chown root:root` that file, remove the device node, and
  `setcap -r /usr/bin/ping`. After the restore: `stat -c '%u:%g'` gives `54321:54322`
  (`--numeric-ids`), `stat -c '%F %t,%T'` gives `character special file 1,3`, and `getcap
  /usr/bin/ping` is what was noted. Also there: `getfacl` on a file with an ACL, on the real
  ext4 root. The same three as ignored tests in `tests/restore.rs`, for a root run nobody is
  asked to make.
  **Passed (apsis-test, 2026-10-03; DECISIONS "check 11 passed").** The items were the
  preparation of 2026-10-01, still live under `/opt` and inside two of the base snapshots,
  plus a `user.*` attribute as a fifth: the owner `54321:54322`, the node `c 1,3`, ping's
  `cap_net_raw=ep`, the ACL `user:65534:r--`, `user.apsis`. A snapshot taken for the check
  (the current build, so no reinstall afterwards) held all five; all five were changed
  with no size or modification time moving; a read-only `rsync --dry-run -i` as root named
  each (`.f....og...`, `cD+++++++++`, `.f........x`, `.f.......ax`); the restore (`rsync
  exited 0` after 48 s, `done`) brought all five back, the capability byte for byte. Two
  limits: `--numeric-ids` itself can't be told apart on one machine with one user database,
  and `rsync-log` names the created node but none of the attribute-only changes. The three
  ignored tests stay ignored: this check is their run on hardware.

### 6b.13 Order of work (after the final look)

0. **0.4.1** (above), released on its own, with its apsis-test checks. **0.4.2** (below)
   likewise, released 2026-10-01 with check 13 passed, and merged into `restore-6b-core`.
1. **Core** (no root): filter and protect list, refusals, home and format detection, Apsis
   version reading, space parsing, plan and state files, argv, ESP backup and check, the apply
   state machine with a fake runner, real-rsync temp-tree tests. The state machine does no
   wall-clock age check (6b.6 step 1), and writes the minimal report when the real
   `result.json` is refused (6b.10; its `null` fields and the no-cross-check rule are decided
   there, before 0.5.0).
2. The owner runs checks 0.1 to 0.3 and sets up the baseline. Revise here if 0.1 or 0.3
   surprise.
   **Status: 0.1, 0.2 and 0.3 pass (apsis-test, 2026-10-01; Apsis 0.4.1, kernel 7.1.5).**
   6b.1's condition holds: the apply stays at the next boot. 0.1's findings (the ESP also
   holds kernel-install's empty `<machine-id>/<version>/` and `EFI/Linux/`; snapshots hold
   an ESP copy; ESP file times are off by the time zone and are never used) are in the
   core's tests, with no production code changed. For step 3: the unit's
   `StandardOutput=journal`, and `udevadm wait` with a timeout for the backup device. Open
   for the owner: what the boot refresh runs (6b.6 step 5). All in DECISIONS.md,
   2026-10-01, "checks 0.1 to 0.3". The baseline isn't recorded here yet.
3. **Helper**: methods, polkit action, the unit, `--apply-restore`, plymouth, logind reboot,
   journal. Also decide how the list carries a snapshot's format for the row tooltip (moved
   here from 0.4.1; step 4 builds the tooltip).
   **Design written 2026-10-01 (helper + core + unit + packaging only; nothing visual).** The
   settled facts from apsis-test and the answers to the open items are in 6b.6 (the boot
   refresh, the unit, pop-upgrade-init), 6b.7 (the new refusals), 6b.9 (`Helper3`, the list's
   format field, the lock rule, the ready plan) and 0.4.2 below (the bulk delete). What the
   slice builds, in order, each part with its tests first:
   1. **Core additions** (no root). **Status (2026-10-01, unattended session, commit on
      restore-6b-core):** done: `restore::unit` (new module: the unit's and the drop-in's
      texts and paths, byte-exact tests), the drop-in in `filter::PROTECTED` (6 entries; the
      package test and the real-rsync protect test pass), `refusal::pop_upgrade_found`
      (`lstat` of the three names under a root) + `refusal::check_pending` +
      `Refusal::PopUpgradePending`, `refusal::crypttab_differs` + `Refusal::CrypttabDiffers`,
      `Snapshot::rsync_flags` with `helper::WireSnapshot3`/`WireList3` and their
      `to_wire3`/`from_wire3` (types and serialisation only; `Helper2`'s `(sss)` untouched
      until the helper moves), `native::info::is_old_format(&str)`, and
      `apply::exit_code(End, restart_failed)`. Not done (the helper slice proper, or waiting
      on check 0.4): `KernelIncomplete`'s hook-flag rule, `refresh_boot`'s contract, the
      `Helper3` names, the `CheckRestore`/`RestoreResult` wire types, `Refusal`'s words on
      the wire, `JobKind::Restore`, the ready plan. The design of the drop-in (text, path,
      protection) is built as written and stands or falls with check 0.4c.
      **Update 2026-10-02**: of those, done with items 2 to 4 below: `JobKind::Restore` and
      the ready plan (item 2), the `Helper3` names (item 3), the hook-flag rule, `Refusal`'s
      words and the `CheckRestore` wire type (item 4). Still open: `refresh_boot`'s contract
      (item 8) and the `RestoreResult` wire type (item 9).
      The list as designed: `restore::unit` gets the drop-in's text and path;
      `filter::PROTECTED` gets the drop-in (6 entries; the package test still passes);
      `refusal::check_pending` and `Refusal::PopUpgradePending` (the three Pop names);
      `refusal::crypttab_differs` with the normalisation of 6b.7; `KernelIncomplete` reads the
      hook flag and drops `has_update_initramfs`; `Snapshot` and the list wire carry
      `rsync_flags` (`(ssss)`), and `Info`'s old-format rule is also a function of the string;
      `restore::Runner::refresh_boot`'s contract is one command, `kernelstub --verbose
      --preserve-live-mode`; the names of `Helper3` (interface, error prefix, methods, the
      `restore` action, `OP_RESTORE`, `OP_DELETE_MANY`); the wire types of `CheckRestore` and
      `RestoreResult`; the words of `Refusal` on the wire and back.
   2. **`State`**: `Running::end` (release, then announce) **is in from 0.4.2**, as is the
      end-before-`Finished` ordering (the end's announcement carries a `oneshot` that
      `start` awaits, 2 s at most); the `ready` plan next to the lock (refuses writes, not
      reads; a job of kind `restore`; keeps the helper alive; removed when the starter's bus
      name goes); `JobKind::Restore`. **This is where the build starts.**
      **Status (2026-10-02): done**, `state.rs` and core's `job.rs` (DECISIONS 2026-10-02):
      `JobKind::Restore` (`changes_the_list` false, the UI slice decides what another window
      shows), `Running::ready`, `State::{is_ready, ready, take_ready, starter_left}`,
      `Ready::end`, `ReadyInfo::is_too_old` (`READY_MAX_AGE`, 30 min), Busy for writes and
      `Stop` while ready, reads through, `Job()` reports the plan, no idle exit, a restore
      preparation stoppable like a create. Unused by the service until items 5 to 7, so the
      new items carry `#[cfg_attr(not(test), expect(dead_code, ..))]` for now. Next: item 3
      (`DeleteMany` moves with the rest of `Helper3`'s names) and item 4 (`CheckRestore`).
   3. **`DeleteMany`**: shipped in 0.4.2 on `Helper2`; here it only moves to `Helper3` with
      the other methods. **Status (2026-10-02): done.** The interface, the error prefix, the
      bus policy and `List`'s `(ssss)` are `Helper3` on both sides; the restore method names,
      `OP_RESTORE` and `ACTION_RESTORE` are constants in `names.rs` (the methods come with
      items 4 to 7, the polkit entry with item 10). DECISIONS 2026-10-02.
   4. **`CheckRestore`**: read-only mount, the reads (mountinfo, lsblk, `findmnt`, the ESP's
      names, `/sys/firmware/efi`, the kernelstub configuration, the pending names, the live
      crypttab; the snapshot's `info.json`, `exclude.list`, `boot/`, `usr/lib/modules/`, its
      kernelstub configuration, its hook, its crypttab, its `var/lib/dpkg/status`), then core's
      `refusal::check`, `check_pending`, `crypttab_differs`, `esp::check_before_arming`,
      `filter::has_home`, the Apsis-in-the-snapshot line.
      **Status (2026-10-02): done** (`check.rs` in the helper, `restore::dialog` in core,
      `HelperClient::check_restore`; DECISIONS 2026-10-02 "CheckRestore"). With it, item 1's
      leftovers: the hook-flag rule (`refusal::Snapshot::hook`, `HOOK_FLAG`), `Refusal` and
      `InSnapshot` on the wire, the `(bsbbbs)` wire type. Open for the owner: `has_root` was
      never defined; it's "the snapshot's `exclude.list` lets `/root` in".
   5. **`Restore`**: the same checks, both dry runs (`argv::rsync_dry_run`, `LC_ALL=C`,
      `space::dry_run_size`), the space checks, the safety snapshot (the 0.4.x create, with
      `/home` when home is restored), `request.json` (with `snapshot_created`, the starter
      uid, the needs), `restore.filter`, RECOVER.txt on the backup disk, then ready.
      Stoppable through `Stop` until ready.
      **Status (2026-10-02): done** (`prepare.rs` in the helper, `restore::recover` and
      `filter::home_only` in core, `Error::RestoreRefused` on the wire, `Ending::Ready` in
      `service::start`, `HelperClient::restore`; the polkit `restore` action is in the policy
      file already). The real run is apsis-test check 1. DECISIONS 2026-10-02 "Restore".
   6. **`RestartToRestore`**: freshness (30 min), the live re-checks (space on each
      destination, the ESP's needs, `esp::check_before_arming`, `check_arming` on an `lstat`
      of both link names, `check_pending`), then the arm in 6b.5's order (unit, wants link,
      drop-in, helper copy, `State::default()`, `sync`, the link), the disarm timer, logind
      `Reboot(false)`. Journal `armed; restarting`.
      **Status (2026-10-02): done** (`arm.rs` in the helper: `arm`, `disarm`,
      `clean_leftovers`, `link_state`, the `systemd-run` timer argv, `--disarm`;
      `check_and_arm` and `reboot` in `service.rs`; `esp::sizes_on_esp`/`sizes_in_boot` and
      `plan::TOO_OLD`/`GONE` in core; `HelperClient::restart_to_restore`). A refused
      "Restart now" removes the plan (open for the owner). DECISIONS 2026-10-02
      "RestartToRestore".
   7. **`CancelRestore`**, the stale plan, the disarm timer's service, the leftover cleanup at
      start.
      **Status (2026-10-02): done** (`cancel_restore`, `remove_plan` and `watch_starters` in
      `service.rs`; `arm::clean_at_start` run from `main.rs`; the timer's service is item 6's
      `--disarm`; `HelperClient::cancel_restore`). DECISIONS 2026-10-02 "CancelRestore".
   8. **`--apply-restore`**: the real `Runner` for `apsis_core::restore::apply::apply`: the
      link check (`readlink`, compared with the state folder), `udevadm wait --timeout=60`,
      the read-only mount and the delete's path checks, `find_snapshot`, the copy (rsync in
      the helper's fixed environment, standard output read **line by line** as it comes, each
      progress line to plymouth and the whole of it kept for `Copied::new`; standard error's
      tail kept; `syncfs` on `/` and a restored separate `/home` before returning),
      `esp::back_up`, `refresh_boot`, `esp::put_back`, the kept kernel's removal with
      `prune::remove_at`, `remove_link`, `remove_arm_files` (unit, wants link, drop-in,
      helper copy), the clock, `say` (journal + `plymouth display-message`, progress with
      `plymouth system-update --progress=N`), `restart` (`systemctl reboot --no-block`, as in
      check 0.3; **if that call fails, exit 1 so `FailureAction=reboot` reboots**, 6b.6). A
      panic or an error outside `apply` removes the link before exiting. `CheckRestore` and
      `List` share the read mount of 0.4.2.
      **Status (2026-10-02): done** (`apply.rs` in the helper: `apply_restore`, `RealRunner`;
      `native::mount_by_uuid`). Plymouth's progress bar is sent after the copy, not during
      (DECISIONS 2026-10-02 "--apply-restore"); the rest is as listed. Check 1 is its run.
   9. **`RestoreResult`**, read from `result.json` through core; `""` and `0` for `null`.
      **Status (2026-10-02): done** (`restore::state::RestoreResult` and its wire in core,
      `restore_result` in `service.rs`, `HelperClient::restore_result`; `ready` from the
      helper's ready plan). DECISIONS 2026-10-02 "RestoreResult".
   10. **Packaging**: the polkit file's sixth action; the bus policy names `Helper3`; `postrm
       purge` removes the drop-in and its folder when empty, `/var/lib/apsis/` and a leftover
       unit; nothing new is installed. `just deb-install`'s re-login note stays.
       **Status (2026-10-02): done** (the action with item 5, the policy with item 3; `postrm
       purge` also stops the disarm timer and removes `/system-update` only when it's
       Apsis's link). DECISIONS 2026-10-02 "packaging".
   11. `cargo test --workspace`, clippy `-D warnings`, `cargo fmt`, summary. No commit.
       **Status (2026-10-02): done**; each item was committed on `restore-6b-core` with the
       owner's "yes". **The helper slice is complete.** The open points for the owner and the
       next steps (the UI slice, step 4; check 1's runbook) are in DECISIONS 2026-10-02
       "packaging".

   **For the UI slice (6b.13 step 4)**, wording this slice needs but doesn't build; the
   string table in 6b.8 gets them there:
   - the `PopUpgradePending` refusal pair (gist in 6b.7): "A Pop!_OS upgrade is in
     progress." / "Finish or cancel it first, then restore.";
   - the crypttab pair: "The encrypted disks are set up differently from when this snapshot
     was made." / "Restore needs a snapshot made with the current disk setup.";
   - the boot-screen line for `LinkStuck` (6b.6, cleanup table), where nothing may restart:
     "The system is restored, but the computer couldn't be set to start normally. Turn it off
     and on again. If this screen comes back, see 'If a restore goes wrong' (README)."; and
     the README's recovery steps already end with `rm /mnt/system-update`;
   - the row tooltip reads the format from the list's fourth field through core's rule;
     the text stays "Older format: made without ACLs and extended attributes";
   - the ready-plan job in another window's status area, kind `restore` ("Restore ready in
     another window" or the like; the popup shows it as any running job);
   - "The preparation is gone. Start the restore again." for a plan the helper no longer has.

   **Claims this design rests on, and their status** (asked for by the owner, 2026-10-01).
   "Verified" means seen on apsis-test by the owner, or in this repo's code; the first
   command block the owner ran on 2026-10-01 is in DECISIONS.md with its results. Each
   unverified claim has a read-only command; none writes anything.

   | claim | status | evidence, or the command |
   |---|---|---|
   | The three offline-update units are wanted by `system-update.target` from `/usr/lib`, with no ordering against Apsis's unit | verified | owner, `systemctl show`, 2026-10-01 |
   | `pop-upgrade-init` checks only that `/system-update` exists, has `KillMode=none`, `FailureAction=reboot`, logs to `/var/log/upgrade.log`; `upgrade.sh` does what 6b.6 lists | verified | owner, `systemctl cat`, the script, check 0.3's journal |
   | `pk-offline-update` checks the link's target itself and exits "no trigger" | verified | check 0.3's journal |
   | `fwupdoffline` checks the link and asks `pending.db` for pending devices; `pending.db` exists with nothing pending | verified | owner, `strings`, `ls`, check 0.3's journal |
   | `system-update-cleanup.service`: `After=system-update.target`, runs only while a link exists, `rm -fv` both names, `SuccessAction=reboot`, no `FailureAction` | verified | owner, `systemctl cat`, 2026-10-01 |
   | **`system-update-cleanup.service` has `Conflicts=shutdown.target`**, so a reboot Apsis enqueues before its unit exits drops cleanup's pending start job and a `Retry`'s link survives | verified | owner, `systemctl cat`, systemd 255.4, 2026-10-01: `Conflicts=shutdown.target` in `[Unit]`. `restart` stays `systemctl reboot --no-block` |
   | `systemctl reboot --no-block` works from inside a unit in `system-update.target` | verified | check 0.3 |
   | `ConditionPathExists=` follows a symlink in the path (`access(2)`), so `!/system-update/apsis-helper` is true exactly while the link points at Apsis's folder | verified | owner, 2026-10-01: `systemd-analyze condition 'ConditionPathExists=/lib/systemd/systemd'` succeeded (`/lib` -> `usr/lib`), and `'ConditionPathExists=!/system-update/apsis-helper'` succeeded with no link present. Check 0.4 shows it on the real unit |
   | A drop-in in `/etc/systemd/system/<unit>.d/` applies to a unit in `/usr/lib`, and a `ConditionPathExists=` line in it is added to the unit's conditions, not replacing them | **verified** (check 0.4, apsis-test, 2026-10-01, systemd 255.4) | `systemctl cat pop-upgrade-init.service` showed the `/usr/lib` unit's `ConditionPathExists=/system-update` and the drop-in's `!/system-update/apsis-helper` both loaded (`DropInPaths=` named the drop-in); in the offline boot, with the link present (the spike was ordered after it, so its own condition was true), the journal has `pop-upgrade-init.service ... was skipped because of an unmet condition check (ConditionPathExists=!/system-update/apsis-helper)` at priority 6. `ConditionPathExists` is not a `systemctl show` property (nothing printed for it); `cat` is the check |
   | The initramfs holds an empty `main/etc/fstab` and a 0-byte `main/cryptroot/crypttab`; cryptswap with a random key isn't carried | verified | owner, `unmkinitramfs` of `initrd.img-7.1.5`, 2026-10-01 |
   | An encrypted-root install's initramfs carries root's crypttab line (so the crypttab comparison matters there) | verified in the hook's source | owner, 2026-10-01: `/usr/share/initramfs-tools/hooks/cryptroot` looks up crypttab entries for the devices of `/` (`get_mnt_devno /`, line 180), the resume device (`get_resume_devno`, line 188) and `/usr` (line 192) |
   | initramfs-tools never embeds the live fstab's content | verified for 7.1.5 | the empty `main/etc/fstab` above |
   | kernelstub picks the newest `/boot/vmlinuz-*` for `vmlinuz.efi` and the next for `-previous`; `--preserve-live-mode` is hidden from `--help`, defined in `application.py`, and exits 0 | verified | owner, dry run, 2026-10-01 |
   | The baseline snapshot's two kernelstub hooks contain `--preserve-live-mode` | verified | owner, `grep -c` 1 each |
   | `/boot/vmlinuz` is a link to the newest installed kernel on Pop!_OS, so the check and kernelstub agree | verified | owner, 2026-10-01: `/boot/vmlinuz` -> `vmlinuz-7.1.5-76070105-generic`, `/boot/vmlinuz.old` -> 7.0.11; the same for `initrd.img` and `initrd.img.old` |
   | `udevadm wait --timeout` exists | verified | systemd 255.4 |
   | `C.UTF-8` exists on the HP (the dry runs and the real restore both run rsync under `LC_ALL=C.UTF-8`) | verified | owner, 2026-10-02: `LC_ALL=C.UTF-8 locale charmap` printed `UTF-8` on apsis-test. In the offline boot: the same system, the same locale files; check 1's offline copy (2026-10-02, run 3) ran under it and exited 0 |
   | Flatpak's ostree repositories are hard-link heavy; `/usr` has few | verified | owner: user repo 1.9G `du` vs 4.1G `du -l`; `/usr` 28 multiply-linked files; system repo empty on apsis-test (decision 6) |
   | Every `List` from the applet opens a new system-bus connection, so each call has its own unique name | verified | `HelperClient::connect` runs `Connection::system()` per call; `list_snapshots`, `background_list`, `poll_job` each connect |
   | A process told Busy lists again on any job's end, and a `List` is itself a job that announces its end: three processes make 3 + 2 + 1 = 6 lists with 3 refusals | verified in the code; the count matches the journal | `app.rs` `on_job` (`changed \|\| self.helper_busy`), `service.rs` `list` (`begin(JobKind::List)`) |
   | **The three processes are two panel applets and the window.** cosmic-panel runs one applet process per output (`apsis %F`, both children of the same cosmic-panel; Apsis is listed once in the panel's `plugins_wings`; the machine has two monitors, and the second process started when a monitor came up). So N applet processes is normal and N follows the number of displays; no reinstall was involved (`dpkg.log` empty for that time) | verified | owner, apsis-test, 2026-10-01. 0.4.2 assumes one applet per display plus the window |

   Carried over from the core slice (all stay):
   - the dry runs are `argv::rsync_dry_run` (`--dry-run --no-human-readable`, the restore's
     own flags and filter, no log), and every rsync runs with `argv::LOCALE` (`LC_ALL=C`),
     so the `--stats` sizes are plain byte counts;
   - the size is read with `space::dry_run_size`: no readable size is `Refusal::SizeUnknown`,
     never zero;
   - a copy is handed to core as `Copied::new(exit, stdout, tail)` with **all of rsync's
     standard output**, not its last lines: the "skipping file deletion" line is printed
     there, anywhere in the run, and `Copied::end` needs it (6b.10, "Exit 23");
   - `Runner::find_snapshot` reads `snapshots/<name>/localhost` (a folder, not followed) and
     `info.json`'s text from the mounted backup disk; core's `check_snapshot` judges them
     before every copy. Preparing records the snapshot's `created` in `request.json`;
   - free space is `statvfs` `f_bavail`, not `f_bfree`;
   - the plan and state files are read and written only through `apsis_core::restore::{plan,
     state}` (version 1, refused whole when invalid; DECISIONS.md, 2026-10-01). The helper
     makes `/var/lib/apsis/restore/` (root, 0700) and decides what an unreadable file means
     at each point;
   - arming starts the 10-minute disarm timer (6b.5, 6b.9);
   - the apply is `apsis_core::restore::apply::apply(paths, runner)`: the helper writes the
     real `Runner` (the link check, the backup disk, rsync, `esp::back_up` and
     `esp::put_back`, the boot refresh, the kept kernel's removal, removing the link and then
     the unit files, the clock, the journal and boot screen, the restart). Arming writes
     `State::default()`;
   - **the real runner's copy step calls `syncfs` on the restored filesystem** (`/`, and a
     separate `/home` when it's restored) after rsync exits and before it returns. Core saves
     step `boot-files` only after that, so `state.json` never records a copy as ended that
     isn't on disk (6b.6 step 3);
   - **the helper never exits non-zero with Apsis's link in place.** `End::LinkStuck` and
     `End::NotArmed` exit 0 without a restart. A panic or an error outside the apply removes
     the link before exiting: the unit restarts on failure, and with the link there that's a
     loop the attempts don't count (DECISIONS.md, 2026-10-01). The boot count (6b.10) is
     what bounds it if that removal is never reached;
   - **the unit has `FailureAction=reboot`**, not `OnFailure=reboot.target`
     (`systemd.offline-updates(7)`, recommendation 3; owner, 2026-10-01);
   - arming runs `refusal::check_arming` on an `lstat` of `/system-update` and
     `/etc/system-update` right before it makes the link;
   - **the helper never decides whether to restart**: core calls `Runner::restart` itself,
     after `Finished`, `GaveUp` and `Retry`. After `LinkStuck` and `NotArmed` it doesn't, and
     the helper only exits 0;
   - removing a stale plan (6b.5) stays the helper's: core has none;
   - (added 2026-10-01) the runner reads rsync's standard output **line by line** while it
     runs (progress to the boot screen) and keeps all of it for `Copied::new`; `syncfs` after
     the copy; never exit non-zero with the link in place; `check_arming` on an `lstat` right
     before the link; core calls `restart`; the disarm timer; the stale plan is the helper's.
4. **UI**, rebuilt from the preview (6b.8) in the real code: the toolbar's Restore (no key, no
   tooltip), the dialog, refusals, preparing status, ready prompt, results in the status line,
   the row tooltip from the list's format field, the strings listed under "for the UI slice"
   in step 3. The preview branch stays unmerged.
   **Status (2026-10-02): done.** Step A: the model, the flow, the four dialogs, the toolbar,
   the row tooltip, the status and result lines, the strings (`app::tests::restore`, 13
   tests). Steps B and C: the layout test over sixteen states at both window sizes (`.rgba`
   screenshots on request) and UI.md. DECISIONS 2026-10-02 "UI slice, step A" and "steps B
   and C". The README, man page and CHANGELOG are step 5's.
5. Docs: README (experimental; "If a restore goes wrong"; how it differs from Timeshift, never
   "restores like Timeshift"; **known limitation: a file changed in place with the same size
   and modification time as in the snapshot isn't restored**, since rsync compares size and
   time, and `--checksum` would read every file on both sides, too slow for a full system;
   **hard links aren't kept** (decision 6, `-H` off; DECISIONS.md, 2026-10-01): Flatpak data
   takes more room in snapshots (apsis-test's user install: 1.9G live, 4.1G in the snapshot)
   and can take more on the system disk after a restore; a user Flatpak app may need a system
   runtime the snapshot doesn't have after a keep-home restore ("runtime not installed",
   `flatpak install` or `flatpak repair` fixes it); `/usr` itself has a handful of linked files
   (28 on apsis-test); **the restore keeps the live `fstab` and `crypttab`** and refuses a snapshot
   whose crypttab differs; **the boot files are refreshed with kernelstub only**, as Pop's own
   kernel hooks do, and the snapshot's initrds are used as they are), man page (the key),
   UI.md, ARCHITECTURE.md (Helper3, the restore's files), CHANGELOG, DECISIONS.md. Then the
   owner's checks 0.4 and 1 to 13. Version 0.5.0.
   **Status (2026-10-02): the docs are done** (DECISIONS 2026-10-02 "step 5"); left for the
   owner at release: the version bump, the Debian changelog entry, the man page's `.TH`
   line, the metainfo's release. Check 0.4 passed 2026-10-01; **check 1 passed 2026-10-02**
   (third run, after six fixes found by the first two; DECISIONS "check 1 passed"); **check 2
   passed 2026-10-02** (after the kernelstub fix its setup found; DECISIONS "check 2
   passed"); **check 3 passed 2026-10-02**; checks 4 to 13 are next, check 4 (Stop, Cancel,
   a filled disk) first.
   **Update 2026-10-03**: checks 4, 5, 6, 7 and 12 passed (DECISIONS, the entries of that
   day). Left, in this order (owner's triage, 2026-10-03): checks 11, 9 and 8; then the
   fixes found by the checks, all before release; then the release gate on the real 0.5.0
   .deb: one happy-path restore (check 1's flow) and check 13 again.
   **Update 2026-10-03, later**: check 11 passed (DECISIONS "check 11 passed"). Left, in
   this order: checks 9 and 8; then the fixes; then the release gate.
   **Update 2026-10-03, evening**: check 9 passed (DECISIONS "check 9 passed"), without the
   version bump (owner): the bump comes once, with the fixes, for the real 0.5.0 .deb. Left,
   in this order: check 8; then the fixes; then the release gate, which gains one line: its
   happy-path snapshot is taken with the real 0.5.0 .deb installed, and its Restore dialog
   shows no Apsis line (`apsis current` in the journal).
   **Update 2026-10-04**: check 8 passed (DECISIONS "check 8 passed", and its corrections),
   so every check has passed. The triage is done and decided (DECISIONS "triage of checks 1
   to 9 and the check 8 findings, and the owner's decisions"); nothing of it is built yet.
   **The order of work**, one small commit each, with build, test, clippy and fmt:
   1. the docs of the triage (this update);
   2. fix 3 (`refused:` in the journal);
   3. fix 4 (the status after a refused restart);
   4. fix 6 (`RestoreResult` carries `home` and `safety_snapshot`);
   5. the result texts in one pass: fix 5 (`problems`), `boot-broken`'s own line, the
      `boot-kept` tooltip, the README's two places, the layout states;
   6. rsync's last 20 lines on a plain exit 23, with the two stale doc comments in core's
      `restore/apply.rs` (`Runner::say`, `Runner::refresh_boot`);
   7. the filter's lifetime (6b.5): `last-restore.filter`, `last-restore.note`, `rsync-log`
      cleared at the arm, the working files removed, the sync in `disarm()`, the empty wants
      folder;
   8. `RECOVER.txt` (6b.11): two labelled commands, the safety snapshot, the caveat, the
      atomic write;
   9. the README's "If a restore goes wrong" and its limits (6b.11);
   10. fix 2 (the delete renames first);
   11. the package scripts and the helper's job lock (fix 1 and 1b, 6b.9);
   12. the small README rows (the desktop's mount, a filter without a leading slash, `just
       uninstall`, the upgrade from 0.4.2), and in the applet Settings' Cancel, always
       clickable, leaving the page (C3);
   13. the version bump, last, before the gate's build. The tag comes after the gate.

   **The release gate**, on the real 0.5.0 .deb, in one list:
   - the file is the .deb CI builds from the tag, taken from the draft release: its hashes
     (the .deb, the helper, the applet), and the install over 0.4.2-1: dpkg says 0.5.0-1;
   - one happy-path restore (check 1's flow) of a snapshot taken with that .deb installed,
     whose dialog shows no Apsis line (`apsis current`); `RECOVER.txt` read back at the
     ready prompt; `tools/restore-check.sh` clean;
   - check 13 in full;
   - **the C1 probe**: the backup disk mounted in Files, then a list, a create and a
     check-restore; the disk unmounted afterwards with `udisksctl unmount`, never ejected;
   - **after the happy-path restore, written down word for word**: the window's status
     line (`System restored to ...`), its tooltip, the four rows, and both monitors;
   - **`RestoreResult` by `busctl`** after the happy-path restore: the answer is
     `(sssxss) "done" "<baseline>" "" <a number> "keep" "<safety snapshot>"`;
   - **the systemd-boot menu's entries word for word**: `bootctl list`, and the menu on the
     screen at one boot;
   - **the kept pair, by hash**: at the ready prompt, the hashes of `restore.filter` and of
     `RECOVER.txt` on the backup disk are recorded; after the restore `last-restore.filter`
     and `last-restore.note` must have those hashes (the working files are gone by then, by
     design);
   - **the refused "Restart now"** (`/pop-upgrade` present at the prompt; the regression
     test for fixes 3 and 4): afterwards `restore.filter` is gone and `last-restore.filter`
     and `last-restore.note` are unchanged;
   - **`flock -n -E 75 /run/apsis/job.lock true; echo $?` while a create runs**: 75
     (`-E` needs util-linux 2.29 or later: this shows the tool, the flag and the lock at
     once);
   - **what dpkg really does after fix 1's refusal**: `apt install --reinstall` of the 0.5.0
     .deb during a create, apt's output word for word (the script's line, dpkg's lines, the
     new package's `prerm failed-upgrade` attempt); `dpkg -s apsis` still `install ok
     installed` at the same version; the create ends `done`; no stop in the journal;
   - **`apt remove apsis` during a delete**: refused, the package still installed, the
     delete ends `done`;
   - **a reinstall at the "Ready to restore" prompt** goes on; then "Restart now" gives
     "The preparation is gone.";
   - the armed branch of the `prerm`, with a hand-made link (Claude's proposal): the
     script's line, the link gone, `systemctl list-timers` without `apsis-disarm`; a link
     to another place is left alone;
   - **the helper killed (`kill -9`) during a create, then a reinstall**: goes on at once
     (no stale lock); no rsync left afterwards (`pgrep rsync`); the staging folder is a row;
   - fix 2: the helper stopped by hand in the middle of a delete of a throwaway snapshot
     (Claude's proposal): a row appears and Delete removes it;
   - **Settings' Cancel during a running save**: it drops the edits and leaves the page
     while the save still writes; what the window and `config.toml` show afterwards;
   - the close: the three base snapshots and the free bytes of before, `/opt` and the
     harness untouched, nothing armed.

   **Not shown by the gate, knowingly** (owner): the window's texts for `problems` and
   `boot-broken` (no hand-placed `result.json`), the plain-23 path, the FAT "not properly
   unmounted" lines (`fsck.fat -n` was not run), a by-hand go-back across a kernel change.

   **Update 2026-10-05: the release gate passed** on CI's .deb from the tag `v0.5.0` (on the
   release commit `6e5a5a5`), all 25 steps (DECISIONS "the release gate passed"). Settings'
   Cancel right after Save is shown (the edits dropped, the page left, the save written);
   Cancel during a running save is not (the save ended before the click could be seen).
   Two lines above were off: busctl prints the answer as `sssxss`, without brackets, and
   the restore left 7 rows, not four. The draft release waits for the owner's publish;
   the gate's findings are for after 0.5.0.

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

**Open after the helper slice design (2026-10-01), for the owner**, each with the
recommendation the design is written to. **Answered by the owner the same day** where marked:

1. ~~**`Helper3`** for the list's format field (6b.9), rather than a second list method.~~
   **Yes** (owner): one interface bump beats two list methods, since applet and helper ship
   together.
2. ~~**The format travels as the raw flags string** (`(ssss)`, `""` = old), not a bool.~~
   **Yes** (owner): it carries a future `-H` format without another wire change.
3. ~~**crypttab compared, fstab not**, pending the `lsinitramfs` result.~~ **Settled**: the
   initramfs holds an empty fstab and a 0-byte crypttab on apsis-test (6b.6 step 5); the rule
   stays as written.
4. ~~**PackageKit's staged update refuses too**.~~ **No** (owner): staged isn't triggered,
   `pk-offline-update` exits "no trigger", COSMIC Store stages routinely. **fwupd's
   `pending.db` is out too** (owner): a history database that exists on most machines. Only
   the three Pop names refuse (6b.6, 6b.7).
5. **The drop-in is written on arm, in `/etc`, and protected**, not shipped in `/usr`
   (6b.6): a packaged one is deleted by the copy of any pre-0.5.0 snapshot and is then
   missing in a retry boot. Yes. Check 0.4 proves the condition on the real unit.
6. **No `Before=`, no `Conflicts=`** against the other offline-update units (6b.6). Yes.
7. ~~**The bulk delete fix is its own slice, 0.4.2**, before the helper slice is built.~~
   **Yes** (owner): it changes the lock order the restore depends on, so it's proven in a
   small release first. Its design grew with the journal's evidence (0.4.2 below).
8. ~~**Decision 6 (`-H`)**: the block still has to be appended.~~ **Done**: appended
   verbatim to DECISIONS.md, 2026-10-01.
9. **A ready plan doesn't block reads** (6b.9): `List`, `Job`, `ReadConfig`, `CheckRestore`,
   `RestoreResult` run while a plan waits; writes get Busy. The old wording was "held for
   everything but its own restart and cancel". Yes.
10. **The helper stays alive while a plan is ready and watches the starter's bus name**,
    removing the plan when the window goes (6b.9), rather than at the next call. Yes.
11. ~~(new) **`system-update-cleanup.service`'s `Conflicts=shutdown.target`** has to be
    confirmed.~~ **Verified** (owner, 2026-10-01, `systemctl cat`): it's there, so a `Retry`'s
    reliance on it holds and `restart` stays `systemctl reboot --no-block`. Of the claims
    table only one is still open, the drop-in's condition being added to the `/usr/lib`
    unit's, which check 0.4c proves.
12. (new) **`restart` falls back to exit 1** when the reboot call fails, so
    `FailureAction=reboot` reboots (6b.6). Recommended; it's the one case where the helper
    exits non-zero with the link in place, bounded by the attempt already on disk.
13. (new) **Reads aren't jobs any more** (0.4.2): `List` and `CheckRestore` announce no
    `JobChanged` and `Job()` doesn't report them. A 0.4.1 process relied on a list's end to
    retry after Busy; with shared reads nothing gets Busy from a read, and the `BusyRetry`
    timer stays as the fallback for a Busy from a write. Recommended.

## 0.4.2 - Reads that share, one bulk-delete job (released 2026-10-01)

**Status: released as v0.4.2 (2026-10-01), from main at 41bcac9; check 13 passed on
apsis-test, both monitors on.** What the owner saw, against the design below: lists from the
window and the two panel applets overlap in the journal (`list for :1.A: ok`, `list for
:1.B: ok` in the same second) with the same bus name per process every time, and none
refused; a create's end is followed by exactly one list per process (three); a selection of
three is one `delete-many` job (one password, "Deleting 1 of 3" to "Deleted 3 snapshots",
then three lists); the stop-at-failure case (two selected, the older deleted behind Apsis's
back) shows "Delete stopped at <name>" with the Deleted / Not deleted tooltip **after the
race fix** (the window's own end arriving after its `Finished` was taken for another
window's; commit 2d3f8c7, in the release: the end is on the bus before `Finished`, and the
applet copes with either order); the helper's idle exit still works (inactive after the
minute); zero refusals in the whole session. The `%F` backlog item below rode along
(`Exec=apsis`). Also in the release: the helper's reasons shown without the `apsis-helper:`
prefix (41bcac9). After the tag: the fake-runner mount tests no longer touch
`/run/apsis/backup`, which CI's runner can't create (10ac37c on main; test-only). DECISIONS
2026-10-01 has the build, the fix and check 13's account. The text below is the design as
built.

**A small release before 0.5.0, first** (owner, 2026-10-01): it changes the lock order the
restore depends on, so it's proven in a small release before the restore is built on it.
Applet and helper ship in one .deb anyway.

**The bug** (apsis-test, 2026-10-01, Apsis 0.4.1): deleting several snapshots, the first
delete finished and the next was refused as busy; closing and reopening the window fixed it.

**What the journal showed** (owner, 2026-10-01): at every job end (after a create and after
each delete) **six `List` calls arrive in the same second from six bus names**
(`:1.1040` to `:1.1045`, `:1.1101` to `:1.1106`, ...), about half refused "busy with another
snapshot operation" by each other; the second delete (`:1.1103`) was refused because `List`
`:1.1101` held the lock. So `List` takes the same exclusive lock as the writes, and reads
exclude reads.

**The cause**, from the code (helper `service.rs` `list`, `start`, `state.rs`; applet
`app.rs` `on_job`, `on_finished`, `start_list`, `background_list`; core `client.rs`
`connect`):
- **Six names are six calls, not six processes**: every `List` goes through
  `HelperClient::connect()`, which opens a **new system-bus connection** per call
  (`Connection::system()`, then `ListActivatableNames` and `NameHasOwner`), so each call has
  its own unique name. `poll_job` and `background_list` do the same.
- **A `List` is a job**: the helper takes the exclusive lock (`begin(JobKind::List)`), so two
  lists refuse each other, and announces `running` and `done` on `JobChanged` to everyone.
- **The applet lists on every job's end**, and twice over: `on_job` lists when a create or
  delete ended, **or whenever this process was told Busy** (`changed || self.helper_busy`),
  and that includes a `List`'s end. So one delete's end in a system with three Apsis
  processes goes: 3 lists, 2 refused; those 2 list again on the survivor's end, 1 refused; it
  lists again on that end: **3 + 2 + 1 = 6 lists, 3 refusals**, which is what the journal
  shows. The `BusyRetry` timer (every 5 s) adds more later.
- **The three processes were two panel applets and the window** (owner, apsis-test,
  2026-10-01): cosmic-panel runs **one applet process per output**, and apsis-test has two
  monitors. That is normal, and the number follows the displays. So the applet-side dedupe
  alone can never be enough: however little each process lists, every display adds a
  reader that lists at the same moment as the others. **0.4.2 assumes one applet per
  display plus the window, and the shared reads are what fix it.**
- **The job's end is announced while the lock is still held** (`state.end`, then
  `drop(running)` on the next line, then `Finished`), so even the first refresh can land on
  a lock that's about to be free.
- **A bulk delete is N `Delete` calls**, the next sent on the window's own `Finished`; each
  end sets off the storm above, and the next `Delete` lands in it and is refused `Busy`
  (refused, not queued, by design). The applet then shows "Busy: another snapshot job is
  running", drops the rest of the bulk delete and says nothing about which were deleted.

**The fix**, four parts, helper and applet, no visual change:

1. **Reads share, writes are exclusive.** The helper keeps a **refcounted read-only mount**
   next to the lock: the first reader mounts `ro`, readers run concurrently (they only read
   `info.json` files and `statvfs`), the last one unmounts. A write (`Create`, `Delete`,
   `DeleteMany`, `WriteConfig`; 0.5.0 adds `Restore`) **waits for the readers to finish**,
   up to 15 s, instead of being refused; readers are a second or two. A read that arrives
   while a write runs, or while a write is waiting, is refused `Busy` as today (writer
   priority: a refresh storm can't starve a write, and a list during a create is pointless;
   the end announcement brings the refresh). `List` and `CheckRestore` (0.5.0) are the
   readers; `ReadConfig` and `Job` touch no mount and take no lock, as now.
2. **Reads aren't jobs.** `List` doesn't `begin` a job, announces no `JobChanged`, and
   `Job()` never reports `list`. The end-of-list cascade can't happen. `WriteConfig`
   (`configure`) stays a job: it's a write and short.
3. **The lock is released before the end is announced and before `Finished` is sent.**
   `Running::end(state)` consumes the guard, takes the job out, frees the lock, then
   announces. `start` uses it. This is **the rule the restore builds on** (6b.9): the one
   deliberate exception is `Finished("restore", true)`, where the ready plan keeps refusing
   writes.
4. **`DeleteMany(as names)`** on `Helper2` (an addition, no bump): the names are checked
   first (each a snapshot name, none repeated, at least two), `refuse_if_running`, polkit
   `delete` once, then **one job** holding the lock throughout: each name in order, `Job`'s
   `snapshot` set to the one being deleted and `percent` to done/total, stopping at the first
   failure. `Finished("delete-many", ok, message)`; on failure the message is encoded for
   core to decode into (deleted, failed on, left), so the window's "which were deleted" text
   comes from the helper, not from counting. Journal: `delete-many ["a", "b", +1 more] for
   :1.42: started`, each `deleted <name>`, `done` or the failure. `Delete(s)` stays for one.

**Applet** (its part, and why this is a slice of its own):
- **One connection per process**: `HelperClient` is made once and kept in the model
  (re-made when the bus drops), so a process has one unique name for all its calls and the
  two activation round trips go. The journal then shows one name per process.
- **One refresh per process per job end**: `on_job` lists only when a create or delete
  ended (the `helper_busy` re-list on any job's end goes; with shared reads a read is never
  refused for a read, and the 5 s `BusyRetry` stays as the fallback for a Busy from a
  write). The starter's `on_finished` list stays and dedupes against `loading` as now.
- **`DeleteMany`** with more than one snapshot selected, instead of the loop; progress
  "Deleting 2 of 4…" from `JobChanged`'s `snapshot` and `percent`; the result text from the
  decoded message. Same strings and status area as today.

**Tests**: `State` (the lock rule; **several readers at once**, as many as displays plus a
window, say five, all admitted and none refused; **a write arriving right after a job end**,
while those readers hold the mount, waits for them and then takes the lock, and a sixth
reader arriving while it waits is refused; a reader during a write is refused; no
`JobChanged` for a read); the refcounted mount with a fake runner (one `mount`, one
`umount` for five overlapping readers; the writer's `rw` mount only after the `umount`);
a job end followed at once by N reads and one write, in that order, ends with every read
answered and the write done (the bulk delete's shape); `DeleteMany`'s name checks, progress
fields and message encode/decode; the applet's messages (one connection, one list per end,
one `DeleteMany` call, the progress line, the stopped-at text). Check 13 on apsis-test with
both monitors on: one `delete-many` job, then one `list` per process (three), none refused.

**Journal after 0.4.2**, one job end: `delete-many [...] for :1.42: done`, then `list for
:1.42: ok`, `list for :1.7: ok`, `list for :1.9: ok` (one per process, concurrent, none
refused) and nothing more until the next action.

**Backlog, not 0.4.2 unless trivial** (owner, 2026-10-01; **done in 0.4.2**, `Exec=apsis`):
the applet entry `io.github.atraxsrc.Apsis.desktop` had `Exec=apsis %F`, and cosmic-panel passes `%F` to the
applet literally. Today it's harmless: `main.rs` `mode` and `StartView::from_args` match only
`--window`, `--settings` and `--about` and ignore everything else, so a literal `%F` does
nothing. **Recommended: drop `%F`** (the app takes no files; `MimeType=` is empty; the
field code is the template's leftover), a one-line change to `resources/app.desktop` that
can ride with 0.4.2's packaging. The argument scans stay as they are, which already ignores
anything unknown; no explicit `%F` handling in code.

## 0.5.x - After the restore (not scheduled)

- **A live plymouth progress bar during the copy** (owner, 2026-10-02): 0.5.0 sends the boot
  screen's line once at the start of the apply and the bar's percents after rsync returns
  (`run_streaming`'s callback can't borrow the runner's tools). A second runner for plymouth,
  or a channel from the callback, would show the bar as the copy runs.

1. **`-H`** in create and restore (decision 6, owner, 2026-10-01): hard links kept across a
   snapshot and a restore. The cost on create is rsync's table of multiply-linked files on
   each run; the gain is Flatpak's and ostree's repositories coming back as hard links. The
   `info.json` flags string carries it (`-aHAX --numeric-ids`), and the list's format field
   already travels as that string, so no wire change.
2. **Rebuild the initramfs when the snapshot's crypttab differs from the live one** instead
   of refusing (6b.6 step 5): `update-initramfs -u -k <restored version>` against the live
   crypttab, then the kernelstub call, with the ESP space and the hook's double kernelstub run
   accounted for. Needed before an encrypted root can be accepted.
3. Encrypted or LVM root, split `/boot`, `/usr`, `/var` (the 6b.7 refusals), each its own
   design.
   **Update 2026-10-06: the encrypted root of a standard Pop!_OS install (LUKS, then LVM)
   is released as 0.6.0.** The spike and the drill passed on apsis-test, then the release
   gate on CI's .deb from the tag `v0.6.0` (on the release commit `6ec5d5e`), all 20 pass
   words (DECISIONS "the 0.6.0 release gate passed"). Item 2's rebuild was not needed for
   it and is not built: a snapshot whose crypttab differs from the live one is still
   refused. LUKS without LVM, LVM without LUKS and the split layouts stay refused.
   **Not shown by the gate, knowingly:** a forced `boot-kept` and the recovery (the drill
   showed each once, on the dev build), the note's "in use" case, the by-hand lines after
   a restore that changed the kernel, a plain, unencrypted install.

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

Roadmap, not scheduled: undock (optional, late). No scheduler: it was dropped on purpose
(Phase 5.2, 2026-09-28) and `phase-5.2-schedule` stays parked (owner, 2026-10-03).

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
