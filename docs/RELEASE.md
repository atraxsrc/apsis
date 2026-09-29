# Release

Steps for a release, and the drafts for the COSMIC community collection. Claude prepares files;
the user tags, pushes, publishes the release and opens the collection PR.

## Every release

Files to bump (Claude prepares these; the user reviews and commits):

- `version` in the root `Cargo.toml` (`[workspace.package]`), then `cargo update --workspace`
  so `Cargo.lock` follows (only the three apsis crates change)
- `resources/app.metainfo.xml`: a new `<release>` at the top of `<releases>`
- `resources/deb/changelog`: a new entry at the top, `apsis (<version>-1) noble`, signed
  with the repo's commit identity
- `CHANGELOG.md`: the new section and its compare link
- `docs/apsis.1`: the version and date in the `.TH` line (check with `man -l docs/apsis.1`)

Then `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `just deb` and
`lintian --pedantic target/debian/apsis_<version>-1_amd64.deb` (0 errors; the kept warnings are
in DECISIONS.md, 2026-09-26 ".deb package").

**The .deb is automatic.** Pushing a `v*` tag runs `.github/workflows/release.yml`: it builds
the .deb on ubuntu-24.04 (`just deb --locked`) and uploads `apsis_<version>-1_amd64.deb` to
that tag's GitHub release. If the release doesn't exist yet, the workflow makes a **draft**
with placeholder notes; edit its notes and publish it. If it does exist, the file is added
(or replaced) there. Check the workflow's run in the Actions tab before announcing the release.

## v0.3.0 checklist

Prepared (2026-09-29): version 0.3.0 (`Cargo.toml`, `Cargo.lock`), metainfo `<release>` and
description, `resources/deb/changelog`, the man page's `.TH` line, `CHANGELOG.md`, README.
Covers the UI polish (status model and tooltip, panel label, disk colours, progress, the
four-room window, the read-only popup, theme audit, the staging warning wording).

Screenshots retaken (2026-09-29): `docs/screenshot-{window,popup,create,schedule}.png`. The
metainfo and the collection entries below point at the `v0.3.0` copies, which only resolve
once the tag is pushed.

User steps, in order: as for v0.2.0 below, with `v0.3.0` (commit, push and check CI, tag and
push, wait for the Release workflow, paste the `0.3.0` section of `CHANGELOG.md` as the
notes, publish). `CLAUDE.md` still has the old "Timeshift-style" listing line; update it.

## v0.2.0 checklist (done)

Prepared (2026-09-29): version 0.2.0 (`Cargo.toml`, `Cargo.lock`), metainfo `<release>` and
summary, `resources/deb/changelog`, the man page, `CHANGELOG.md`, README, SECURITY.md (0.2.x
supported). Covers the standalone rework (no Timeshift) and the unreleased `90272f5` and
`36c094f` work (keep last N, reminder, low-priority rsync, disk bar, progress).

User steps, in order:

1. Commit, push `main`, and check that CI passes.
2. Tag and push:

   ```sh
   git tag -a v0.2.0 -m 'Apsis 0.2.0'
   git push origin v0.2.0
   ```

3. Wait for the **Release** workflow (Actions tab). It creates a draft release `Apsis 0.2.0`
   with the .deb attached.
4. Edit the draft: paste the `0.2.0` section of `CHANGELOG.md` as its notes (it has the
   "upgrading: open settings once and press w" note), check that `apsis_0.2.0-1_amd64.deb` is
   attached, and publish.
5. The listing line changed: "Simple system snapshots and file restore for the COSMIC™
   desktop" (no Timeshift). `CLAUDE.md` still has the old one; update it, and use the drafts
   below for the collection PR.

## v0.1.2 checklist

Prepared (2026-09-27): version 0.1.2 (`Cargo.toml`, `Cargo.lock`), metainfo `<release>`,
`resources/deb/changelog`, the man page's `.TH` line, `CHANGELOG.md`. Covers commits `7076a1c`
(bulk delete, delete labels, menu, `Ret=`) and `e92d5b9` (settings guide, man page, settings
details).

User steps, in order:

1. Commit, push `main`, and check that CI passes.
2. Tag and push:

   ```sh
   git tag -a v0.1.2 -m 'Apsis 0.1.2'
   git push origin v0.1.2
   ```

3. Wait for the **Release** workflow (Actions tab). It creates a draft release `Apsis 0.1.2`
   with the .deb attached.
4. Edit the draft: paste the `0.1.2` section of `CHANGELOG.md` as its notes, check that
   `apsis_0.1.2-1_amd64.deb` is attached, and publish.
5. Optional: install the attached .deb and check `man apsis` and the right-click menu.

## v0.1.1 checklist (done)

Done: `v0.1.1` tagged and the release published with the .deb attached (confirmed by the user
2026-09-27).

Prepared (2026-09-26): version 0.1.1, metainfo `<release>`, `resources/deb/changelog`,
`CHANGELOG.md`; the .deb tested by the user on Pop!_OS (install, helper activation, polkit
actions, create/delete, reinstall, remove).

User steps, in order:

1. Commit, push `main`, and check that CI passes.
2. Tag and push:

   ```sh
   git tag -a v0.1.1 -m 'Apsis 0.1.1'
   git push origin v0.1.1
   ```

3. Wait for the **Release** workflow (Actions tab). It creates a draft release `Apsis 0.1.1`
   with the .deb attached.
4. Edit the draft: paste the `0.1.1` section of `CHANGELOG.md` as its notes, check that
   `apsis_0.1.1-1_amd64.deb` is attached, and publish.
5. Optional: download the attached .deb and install it on a clean 24.04 machine or VM.

## v0.1.0 checklist (done)

Prepared (Phase 7 prep, 2026-09-26):

- [x] libcosmic pinned by `Cargo.lock` (commit `03d7dcb8`), with `--locked` in CI; no `rev` in
      `Cargo.toml` (see "libcosmic pin" below)
- [x] CI: `.github/workflows/ci.yml` (fmt, clippy `-D warnings`, tests; `--locked`)
- [x] `SECURITY.md`, `CHANGELOG.md`, README (features, install, experimental parts)
- [x] `resources/app.metainfo.xml`: description, screenshot, release 0.1.0, OARS
      (`appstreamcli validate --no-net` on `target/xdgen/app.metainfo.xml`: only the `COSMIC`
      category warning, which COSMIC applets use on purpose)

User steps, in order:

1. On GitHub, enable **Settings → Code security → Private vulnerability reporting** (SECURITY.md
   points to it).
2. Push `main` and check that CI passes.
3. `just vendor` for the vendored tarball, and check it builds (`just build-vendored`).
4. If the release date isn't 2026-09-26, change `<release date=...>` in the metainfo and the
   date in `CHANGELOG.md`.
5. Tag and push (don't use `just tag`: the version is already 0.1.0, so its commit would be
   empty and fail, and it tags `0.1.0` without the `v`):

   ```sh
   git tag -a v0.1.0 -m 'Apsis 0.1.0'
   git push origin v0.1.0
   ```

6. Create the GitHub release for `v0.1.0` with the CHANGELOG section as its notes. The
   metainfo's screenshot URL (`.../apsis/v0.1.0/docs/screenshot.png`) and release URL only
   resolve after the tag is pushed.
7. Open the PR to `cosmic-utils/cosmic-project-collection` with the entries below.

### libcosmic pin

libcosmic is a git dependency with no `rev`; `Cargo.lock` holds the commit, and CI builds with
`--locked`, so an unintended bump fails CI instead of slipping in. A `rev` was tried and
dropped: `cosmic-panel-config` asks for libcosmic without one, so a `rev` put five crates in
`Cargo.lock` twice, which `cargo vendor` refuses. To move to a newer libcosmic on purpose:
`cargo update -p libcosmic`, then build, test and commit the lock file.

## cosmic-project-collection entries (draft)

**Unverified schema.** These were written without reading the current `applets.ron` /
`applications.ron` (no network in Claude's session). Before opening the PR, copy the field
names and order from a neighbouring entry in each file and adjust these to match.

`applets.ron` - the panel applet:

```ron
(
    name: "Apsis",
    description: "Simple system snapshots and file restore for the COSMIC™ desktop",
    repository: "https://github.com/atraxsrc/apsis",
    app_id: "io.github.atraxsrc.Apsis",
    icon: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.3.0/resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg",
    screenshot: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.3.0/docs/screenshot-window.png",
),
```

`applications.ron` - the same program as a window, from the launcher entry
(`io.github.atraxsrc.Apsis.Window.desktop`, `Exec=apsis --window`):

```ron
(
    name: "Apsis",
    description: "Simple system snapshots and file restore for the COSMIC™ desktop, in a window (apsis --window)",
    repository: "https://github.com/atraxsrc/apsis",
    app_id: "io.github.atraxsrc.Apsis",
    icon: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.3.0/resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg",
    screenshot: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.3.0/docs/screenshot-window.png",
),
```

If the collection lists one program only once, keep the `applets.ron` entry and mention window
mode in its description instead.

PR text (draft):

> Add Apsis, a panel applet for simple system snapshots: create, list and delete rsync
> snapshots and restore single files from them, in a terminal-style window that follows the
> COSMIC theme, with a read-only overview in the panel popup. It uses Timeshift's on-disk layout, so existing Timeshift snapshots keep
> working, but doesn't need Timeshift. It opens as a window from the app launcher
> (`apsis --window`). GPL-3.0-only.
