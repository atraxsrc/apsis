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

## Open

- ~~App ID~~ - resolved 2026-09-25, see above.
- ~~License~~ - resolved 2026-09-25, see above.
- Icon artwork.
- Phase 4 helper vs. shipping a narrow pkexec wrapper script — decide after Phase 3.
