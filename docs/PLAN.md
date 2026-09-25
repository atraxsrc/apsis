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

## Phase 6 — Restore

- File-level: browse a snapshot, copy files back (easy, safe).
- Full system: never restore the running root live. Stage the restore, require typing the snapshot
  name, then reboot. Pop!_OS uses systemd-boot (no GRUB), so no grub-btrfs style boot entries.

## Phase 7 — Release

- README screenshots, metainfo, `just vendor` tarball, tag `v0.1.0` (user pushes).
- Draft the cosmic-project-collection entry for `applets.ron`; the user opens the PR.
