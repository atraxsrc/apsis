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
  - native retention (Timeshift applies it on its next run until then)
  - idle I/O priority for the native rsync (Timeshift 24.01.1 doesn't use it either)
  - Btrfs: Phase 5.1 below
  - recognise Timeshift's `Ret=NNN` lines as diagnostics (they're ignored now)

## Phase 5.1 - Native btrfs

- btrfs: read-only subvolume snapshots of `@` / `@home` directly.

## Phase 5.2 - Native schedule

- Scheduling via systemd timers; retention counts per tag.

## Phase 6a - File-level restore

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

## Phase 6b - Full-system restore (not started)

- Never restore the running root live. Stage the restore, require typing the snapshot name,
  then reboot. Pop!_OS uses systemd-boot (no GRUB), so no grub-btrfs style boot entries.

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

## Phase 7 — Release

- README screenshots, metainfo, `just vendor` tarball, tag `v0.1.0` (user pushes).
- Draft the cosmic-project-collection entry for `applets.ron`; the user opens the PR.
- **Status: prep done 2026-09-26** (libcosmic pinned by `Cargo.lock`, CI, SECURITY.md, README, metainfo,
  CHANGELOG, collection drafts). Left for the user: see the checklist in `docs/RELEASE.md`
  (vendor tarball, tag, release, collection PR).
