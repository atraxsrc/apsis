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

- `[c]reate`: inline prompt for a comment → `timeshift --create --comments <c> --tags O --scripted`.
- `[d]elete`: inline confirm (type `y`) → `timeshift --delete --snapshot <name> --scripted`.
- Progress line while running; refresh list after.
- The user tests these manually. Claude never triggers them.
- **Done when:** create/delete work end-to-end on the user's machine.

## Phase 4 — Privileged helper (removes repeated password prompts)

- `crates/apsis-helper`: small system D-Bus service (zbus) run as root on demand.
- Methods: `List`, `Create(comment)`, `Delete(name)`; each checks a polkit action
  (`list` → allowed for active local users; `create`/`delete` → `auth_admin_keep`).
- Ship `resources/` files: D-Bus system service + policy, polkit `.policy`, systemd unit.
- Applet talks to the helper; `pkexec` path kept as fallback.

## Phase 5 — Native backend (optional, after Timeshift path is solid)

- btrfs: read-only subvolume snapshots of `@` / `@home` directly.
- rsync + hardlinks (`--link-dest`) for ext4 (Pop!_OS default).
- Scheduling via systemd timers; retention counts per tag.

## Phase 6 — Restore

- File-level: browse a snapshot, copy files back (easy, safe).
- Full system: never restore the running root live. Stage the restore, require typing the snapshot
  name, then reboot. Pop!_OS uses systemd-boot (no GRUB), so no grub-btrfs style boot entries.

## Phase 7 — Release

- README screenshots, metainfo, `just vendor` tarball, tag `v0.1.0` (user pushes).
- Draft the cosmic-project-collection entry for `applets.ron`; the user opens the PR.
