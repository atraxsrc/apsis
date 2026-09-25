# Release

Steps for a release, and the drafts for the COSMIC community collection. Claude prepares files;
the user tags, pushes, publishes the release and opens the collection PR.

## v0.1.0 checklist

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
    description: "Timeshift-style system snapshots for the COSMIC™ desktop",
    repository: "https://github.com/atraxsrc/apsis",
    app_id: "io.github.atraxsrc.Apsis",
    icon: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.1.0/resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg",
    screenshot: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.1.0/docs/screenshot.png",
),
```

`applications.ron` - the same program as a window, from the launcher entry
(`io.github.atraxsrc.Apsis.Window.desktop`, `Exec=apsis --window`):

```ron
(
    name: "Apsis",
    description: "Timeshift-style system snapshots for the COSMIC™ desktop, in a window (apsis --window)",
    repository: "https://github.com/atraxsrc/apsis",
    app_id: "io.github.atraxsrc.Apsis",
    icon: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.1.0/resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg",
    screenshot: "https://raw.githubusercontent.com/atraxsrc/apsis/v0.1.0/docs/screenshot.png",
),
```

If the collection lists one program only once, keep the `applets.ron` entry and mention window
mode in its description instead.

PR text (draft):

> Add Apsis, a panel applet for Timeshift-style system snapshots: list, create and delete
> Timeshift snapshots from a terminal-style popup that follows the COSMIC theme. It also opens
> as a window from the app launcher (`apsis --window`). A native rsync backend and file-level
> restore are included as experimental. GPL-3.0-only.
