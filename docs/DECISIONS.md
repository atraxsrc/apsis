# Decisions

Append-only. Newest at the bottom. Format: date — decision — why.

- 2026-09-25 — Name **Apsis** (repo/binary `apsis`). "A point on an orbit you can return to."
- 2026-09-25 — Panel applet + popup (always floats, native place for it) rather than a tiled window.
- 2026-09-25 — Wrap the `timeshift` CLI first; native btrfs/rsync backend later.
- 2026-09-25 — Own repo, separate from cosmic-workbench.
- 2026-09-25 - App ID **`io.github.atraxsrc.Apsis`** - chosen by the user; used for the .desktop file,
  metainfo, icons, config and (phase 4) polkit action prefix.
- 2026-09-25 - License **GPL-3.0-only** - chosen by the user; matches most COSMIC apps.
- 2026-09-25 - Phase 0 scaffold from `pop-os/cosmic-applet-template` (cargo-generate 0.25, `--vcs none`),
  generated into `.scratch/apsis`, then split into a workspace:
  - Root `Cargo.toml` is a virtual workspace (`members = ["crates/*"]`, `resolver = "2"` per PLAN).
    Version, edition (2024), license and repository live in `[workspace.package]`; all deps incl. the
    libcosmic git dep and its features live in `[workspace.dependencies]`. The `[patch]` hint for a
    local libcosmic moved to the root, since patches only work there.
  - `i18n/`, `resources/`, `justfile`, `rustfmt.toml` sit at the root. Anything that resolves paths
    from the crate dir was pointed at `../../`: `build.rs` (xdgen input + `rerun-if-changed`),
    `#[folder]` for rust-embed in `i18n.rs`, and `assets_dir` in `crates/apsis/i18n.toml` (i18n-embed
    reads `i18n.toml` from the crate's manifest dir, so that file stays in the crate).
  - `build.rs` writes the expanded `.desktop`/metainfo to `<workspace>/target/xdgen/`, which is where
    the justfile's `install` recipe looks.
  - justfile: `run` uses `-p apsis`; `check` adds `--workspace`; `uninstall` also removes the
    metainfo (template forgot it); `tag` now bumps only the root `Cargo.toml` - the template's
    `find ... sed` would have overwritten `version.workspace = true` in each crate.
  - Removed the template's unused `struct MySubscription` in `app.rs` (dead code fails clippy -D warnings).
    `pending().await` became `pending::<()>().await;` so `just check` (clippy::pedantic) is clean too;
    the turbofish is needed once the semicolon removes the type hint.
  - Not carried over: template `.zed/` editor settings, template README (ours already exists),
    template `.gitignore` (root one already covers `target/`, `vendor/`).
- 2026-09-25 - Template issues left for later (Phase 7): metainfo `<icon type="remote">` points at
  `resources/icons/hicolor/...` which doesn't exist; metainfo installs to `share/appdata` (modern path
  is `share/metainfo`); the `vendor` recipe deletes `vendor/` without writing `vendor.tar`. Panel icon
  is still the template's `display-symbolic` placeholder (Phase 2).
- 2026-09-25 - `docs/TIMESHIFT-CLI.md` verified against the user's Timeshift **v24.01.1** `--help` and
  real `--list` output (rsync mode). Help text itself was not saved as a fixture (its first line has
  the upstream author's contact details).
- 2026-09-25 - **Create without `--tags`.** On v24.01.1 `timeshift --create --tags O` fails with
  `Unknown value specified for option --tags (O)` (found by the user), even though `--help` lists
  `O` and says it's the default. Leaving `--tags` out gives an `O` snapshot. This replaces the
  `--tags O` in PLAN.md Phase 3 and the earlier TIMESHIFT-CLI.md draft.
- 2026-09-25 - Backend passes `--snapshot-device <dev>` when the device is known (user's call), so
  list/create/delete all target the same device as the list the user is looking at.
- 2026-09-25 - Fixtures in `crates/apsis-core/tests/fixtures/` are redacted with same-length
  placeholders so column alignment is preserved: block devices -> `/dev/sdX1`, UUIDs -> all zeros,
  PIDs in `/run/timeshift/<pid>/` and `(process:<pid>)` -> `99999`, free space -> `123.4 GB`.
  Fixtures keep Timeshift's trailing padding on purpose; don't let an editor strip it.
- 2026-09-25 - `--snapshot-device` gets the **UUID** from the last `--list`; the device path is the
  fallback only when there's no UUID (user's call). UUIDs survive `/dev/sdX` renaming between boots.
- 2026-09-25 - `--scripted` doesn't change the `--list` format (user diffed both; only the mount PID
  differed), so the existing fixtures cover the `--scripted` path. No extra fixture.
- 2026-09-25 - Phase 1 `apsis-core` design:
  - `created` is a `jiff::civil::DateTime` (local, no zone), parsed strictly from the name. `jiff`
    and `thiserror` 2 were already in the dependency tree via libcosmic, so no new crates.
  - `Backend` methods are blocking and take `&self`; the applet will call them from a background
    task. `TimeshiftCli` keeps the remembered `--snapshot-device` in a `Mutex`: set by a successful
    list, cleared by an unconfigured list, left alone when a list fails.
  - No real `Runner` yet - nothing executes timeshift in Phase 1. Phase 2 adds one that runs
    `pkexec timeshift ...` without a shell.
  - Input checks before any argv is built: delete names must match `YYYY-MM-DD_HH-MM-SS` (so
    nothing can be read as an option); comments are trimmed, may not contain control characters
    (a newline would break `--list` rows), may not start with `-` (defence in depth, in case
    Timeshift ever parsed it as an option), and are capped at 200 characters (Apsis's own limit).
    A blank comment omits `--comments`.
  - Parser matches lines by content, skips GLib warnings anywhere, and returns `BadRow` for an
    unparseable table line instead of silently dropping it, so format changes show up as errors.
    Output with neither a table nor "No snapshots found" is `UnrecognisedOutput`.
  - Known parser limitation: the token after the name counts as tags if it's only `OBHDWM` letters,
    so a tag-less row whose comment starts with e.g. "BOW" would be misread. Timeshift always prints
    at least one tag, so this shouldn't happen.

## Open

- ~~App ID~~ - resolved 2026-09-25, see above.
- ~~License~~ - resolved 2026-09-25, see above.
- Icon artwork.
- Phase 4 helper vs. shipping a narrow pkexec wrapper script — decide after Phase 3.
- ~~`--snapshot-device`: device path or UUID?~~ - resolved 2026-09-25, see above.
- ~~Does `--scripted` change the `--list` format?~~ - resolved 2026-09-25, see above.
- No real fixture yet for "configured device, zero snapshots" or for multi-tag rows (`BD` etc.);
  both are covered by synthetic tests built from the real layout.
- Phase 2: check whether Timeshift prints the `--list` table on stdout or stderr, and what exit codes
  it uses on failure (Apsis treats any non-zero exit as failure).
