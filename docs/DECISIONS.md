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

- 2026-09-25 - Phase 2 applet (read-only):
  - `PkexecRunner` lives in `apsis-core`. It resolves `timeshift` on the user's `PATH` first (missing
    -> `NotInstalled`), then runs `pkexec --disable-internal-agent /abs/path/timeshift ...` with no
    shell, stdin null, `LC_ALL=C.UTF-8` and no `LANGUAGE` (pkexec passes both through; the parser
    matches Timeshift's English, UTF-8 keeps non-ASCII comments). `--disable-internal-agent`: with
    no graphical polkit agent, fail instead of prompting on the terminal `just run` came from. A
    missing `pkexec` is a distinct I/O error, not "timeshift not installed".
  - The first list runs when the popup first opens, not at login, because every list is a password
    prompt until the Phase 4 helper. Re-opening shows the cached list; `r` refreshes. Only one list
    runs at a time. Once the helper allows `list` without a prompt, list on every open instead.
  - Snapshots are shown newest first (Timeshift lists oldest first). The selection follows the
    snapshot name across refreshes.
  - Tooltip: `Apsis - last snapshot 3h ago` / `Apsis - no snapshots`, plain `Apsis` before the first
    list. Plain hyphen instead of UI.md's dash (project writing style).
  - Panel icon: stock `document-open-recent-symbolic` (a clock with an arrow) until there's original
    artwork, so there's no SVG with a baked-in colour. "Icon artwork" stays open.
  - Hints are `[r]efresh` (`[r]etry` after an error), `[?]help`, `[esc]`. `[c]`/`[d]` and the input
    line arrive with Phase 3; for now the `>` line shows `_`, or `timeshift --list` and a spinner.
  - Keys come from `event::listen_with`, only while the popup is open and only for the popup's
    window; Ctrl/Alt/Super combinations are ignored. Esc closes details/help first, then the popup.
    Rows: click selects, double-click (or Enter) shows details.
  - Colours, all from the theme: accent text (`~/apsis`, `>`, the `▸` marker, help/detail keys);
    accent at 15% alpha for the selected row; the background's `on` colour at 70% for the summary;
    destructive text for errors. Text is `monotext` (libcosmic's monospace preset).
  - Layout: popup 520 px wide, 24 px rows, 8 rows then scroll. Row comments are cut to 28
    characters; details show the whole comment. Arrow keys scroll by `snap_to(i / (n - 1))`, which
    always keeps an equal-height row in view.
  - Errors show the exit code and the last 6 non-blank stderr lines; exit 126/127 (pkexec) adds
    "not authorised, or the password dialog was dismissed".
  - Fluent's Unicode isolation marks are turned off; they render as stray glyphs in monospace text.
  - The applet enables `jiff`'s `tz-system` + `tzdb-zoneinfo` to get local "now" for "3h ago".

- 2026-09-25 - Icons (artwork by the user, made outside the repo):
  - `resources/icons/hicolor/scalable/apps/io.github.atraxsrc.Apsis.svg` is the full-colour app
    icon; `resources/icons/hicolor/symbolic/apps/io.github.atraxsrc.Apsis-symbolic.svg` is the
    single-colour panel icon. The template's empty `resources/icon.svg` is gone.
  - `just install` puts them in `$prefix/share/icons/hicolor/{scalable,symbolic}/apps/`;
    `uninstall` removes both. The metainfo now installs to `share/metainfo` (was `share/appdata`).
  - Desktop entry `Icon=` and metainfo `<icon type="stock">` both use the app ID. The old
    `type="remote"` icon pointed at a file that never existed.
  - Panel button: `io.github.atraxsrc.Apsis-symbolic` from the icon theme, drawn as symbolic so
    libcosmic tints it with the theme text colour. Looked up once at startup, with libcosmic's
    name fallback **off**: by default it retries the name cut at each `-`, which would pick the
    full-colour `io.github.atraxsrc.Apsis` when only the symbolic one is missing. If the lookup
    fails (`just run` before `just install`), the applet uses the same SVG embedded with
    `include_bytes!`, so there is one copy of the artwork.
  - Popup header: the same symbolic icon, 16 px, tinted with `accent_text_color()`, the colour
    of the `~/apsis` text beside it (the theme's accent, adjusted for contrast).
  - These are pre-existing template leftovers, not caused by the icon change; `appstreamcli
    validate` and `desktop-file-validate` report the same things before and after. For Phase 7:
    the metainfo has no `<description>` (an error for appstream), no homepage `url`, and no
    `developer` element, and `COSMIC` isn't a registered category (`X-COSMIC` or a main category).

- 2026-09-25 - Keyboard in the panel popup (user report: keys do nothing in the panel; mouse fine).
  - The `>` line is now a real libcosmic `text_input` (`inline_input` style, monospace, always
    empty) with an `Id`. It is `always_active()` (libcosmic re-focuses it on every layout) and also
    gets an explicit `text_input::focus` when the popup opens, after each `pkexec` list returns
    (the polkit dialog takes focus), and after Esc closes an overlay (Esc unfocuses the input).
  - Routing: while the line has focus it captures printable keys and Enter, which arrive as
    `on_input` (each character is a command: j/k/r/?) and `on_submit` (details). The event
    subscription handles characters and Enter only when no widget captured them, so nothing runs
    twice. Arrows, Home/End and Esc come from the event subscription either way (the input doesn't
    report them). Unit tests cover the routing.
  - What the source says (libcosmic 03d7dcb, its iced fork, cosmic-panel b5cc19e, read locally from
    the cargo cache; cosmic-applets was not fetched, since only cargo may use the network): key
    events go to whichever Wayland surface has keyboard focus, tagged with that surface's window
    id, whether or not a widget has focus. `get_popup_settings` asks for a grab; cosmic-panel gives
    an embedded applet's new popup keyboard focus when it's created and forwards the grab. So a
    focused widget is probably not what decides it at the Wayland level, and the fix above may not
    be enough on its own. Likely suspect: the polkit dialog, which opens right after the first
    popup, takes keyboard focus away from the grabbed popup, and a client can't take focus back.
    If keys still fail after the password prompt but work after reopening, the next step is to
    re-create the popup (new grab) after the first list, or to list before opening it.
  - `APSIS_DEBUG_KEYS=1` logs key events (with capture status and window id), popup
    Focused/Unfocused events and dropped keys to stderr, to find where keys are lost.
  - Result (user, installed in the panel): j, k, ? and Esc work, both after the password prompt
    and on reopen. So the focused, always-active input fixed it, whatever the reading of the
    source suggested; no popup re-creation needed.

- 2026-09-25 - Right-click menu on the panel button (Refresh, About Apsis, Panel settings…).
  - How COSMIC does it, from what's in the cargo cache (libcosmic 03d7dcb, cosmic-panel b5cc19e;
    cosmic-applets itself wasn't fetched, only cargo may use the network): libcosmic has no
    ready-made context menu for applets, and cosmic-panel just forwards the right button to the
    applet. libcosmic does ship the pieces the system applets build their menus from:
    `applet::menu_button` (the `AppletMenu` button style), `applet::padded_control` for dividers,
    and `process::spawn` for launching settings. The applet button only handles the left
    button, so a `mouse_area(...).on_right_release` around it takes the right one.
  - The menu is a second popup (`menu: Option<Id>`), with its own 240 px width. Only one popup
    has the grab at a time, so opening either one first destroys the other (`chain`, so the
    destroy happens before the create). `view_window` picks by id.
  - The menu uses the normal COSMIC look (`text::body` in `menu_button`), not monotext: it's a
    system menu, and it then matches other applets' menus.
  - Refresh and About open the popup, since that's where their output is shown: Refresh starts
    a list (the same as `r`, still a password prompt), About sets a new `Overlay::About`.
    Esc goes back to the list, as for help and details.
  - About: name and version, the app comment, license and a repository link, all from
    `Cargo.toml` via `env!("CARGO_PKG_*")`. The link is a `Button::Link` (accent text) that runs
    `xdg-open <repository>`.
  - `cosmic-settings panel` and `xdg-open` go through `cosmic::process::spawn` (double fork +
    setsid, no zombie, survives a panel restart). This needed libcosmic's `process` feature,
    which pulls in `libc` and `rustix`, both already in the dependency tree. A missing program
    is logged to stderr, nothing else.
  - Keys: the event subscription also runs while the menu is open; only Esc does anything there.
  - Sandbox note: cargo only builds here with `HOME` pointed at a temp dir (and `--offline`):
    with `~/.gitconfig` unreadable, libgit2 treats the libcosmic git db as broken and cargo
    tries to re-create it in the read-only cargo home.

## Open

- ~~App ID~~ - resolved 2026-09-25, see above.
- ~~License~~ - resolved 2026-09-25, see above.
- ~~Icon artwork~~ - resolved 2026-09-25, see above.
- Phase 4 helper vs. shipping a narrow pkexec wrapper script — decide after Phase 3.
- ~~`--snapshot-device`: device path or UUID?~~ - resolved 2026-09-25, see above.
- ~~Does `--scripted` change the `--list` format?~~ - resolved 2026-09-25, see above.
- No real fixture yet for "configured device, zero snapshots" or for multi-tag rows (`BD` etc.);
  both are covered by synthetic tests built from the real layout.
- Phase 2: check whether Timeshift prints the `--list` table on stdout or stderr, and what exit codes
  it uses on failure (Apsis treats any non-zero exit as failure). Apsis parses stdout only; if the
  popup says "unrecognised `timeshift --list` output", the table is probably on stderr.
- Phase 2: confirm on a real panel that the popup gets keyboard focus (keys were only reasoned
  about, not run, by Claude). ~~Check that `document-open-recent-symbolic` exists~~ - replaced by
  the Apsis symbolic icon, 2026-09-25.
