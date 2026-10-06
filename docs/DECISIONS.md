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

- 2026-09-25 - Phase 3.5 (superfile-style layout) added to PLAN.md at the user's request, after
  Phase 3. superfile is a visual reference only; no code from it.

- 2026-09-25 - Phase 3 create and delete (written and unit-tested by Claude; never run by Claude).
  - `apsis-core`: `create`/`delete` now refuse with `Error::NoSnapshotDevice` until a list has
    shown a device, instead of leaving `--snapshot-device` out and acting on Timeshift's configured
    default. Input checks still come first. The old test that created after an unconfigured list
    now checks the next `--list` instead. `validate_comment` is public so the applet can refuse a
    bad comment (e.g. a leading `-`) before any password prompt; `create` still checks it too.
  - `c`: the `>` line becomes `> comment: _`. The input keeps what's typed (capped at
    `MAX_COMMENT_CHARS` while typing), so `r`, `j` etc. are text there, not commands. Enter
    validates, then runs; a refused comment stays in the prompt with the reason above it.
  - `d`: `> delete <name>? [y/N] _` for the selected snapshot; the name is fixed when `d` is
    pressed. Only `y`/`Y` then Enter deletes (the answer is capped at 3 characters, so `yes` is a
    no); anything else shows `delete cancelled`.
  - Both need a list that showed a snapshot device, no list running and no other operation
    running; delete also needs a selected snapshot. The `[c]reate`/`[d]elete` hints are disabled
    otherwise. Only one pkexec timeshift runs at a time: `r` and the menu's Refresh wait too.
  - While running, the `>` line's placeholder shows `creating snapshot… ⠹` / `deleting <name>… ⠹`,
    and only Esc works (it closes the popup; the root operation carries on). Closing the popup
    drops a half-typed prompt but not a running operation or its result.
  - Afterwards a result line above `>`: `snapshot created` / `deleted <name>` (dimmed), or
    `create failed: <reason>` (error colour), where the reason is pkexec's "not authorised" for
    126/127, else the last stderr line, else the exit code. It stays until the next create/delete.
  - Refresh after: only when Timeshift actually ran (success, or a failure other than pkexec
    126/127). Not after pkexec refused or a check stopped it, since nothing changed and a refresh
    is another password prompt. So a create is two prompts (create, then list) until Phase 4.
  - After a refresh the selection follows the snapshot by name; if it was deleted, it stays at the
    same position rather than jumping to the top.
  - Esc order is now: cancel the prompt, close details/help/about, close the popup.

- 2026-09-25 - Phase 3.5 superfile-style layout (look and feel only; written and unit-tested by
  Claude, not yet looked at on a real panel).
  - Panes are a libcosmic/iced `Stack`: the base layer is a bordered container pushed down by
    half a line, the top layer is the title on a small backdrop in the popup's own background
    colour (`background(transparent).base`, as `popup_container` uses), so the title cuts the
    border. No per-side borders exist in iced, so this is the simplest way to set a title into a
    border. On a translucent theme the border may show faintly through the title.
  - Borders: accent when the pane is active, else `background(transparent).divider` (the same
    colour as the popup's own border). Radius `corner_radii.radius_s`. No new colours.
  - The details view is no longer an overlay that replaces the list: the details pane is always
    there. `Overlay::Details` now only marks the details pane as active, so Enter, double-click
    and Esc keep doing what they did (toggle, open, go back). Help and About still replace the
    list, in the left pane, and rename its title.
  - Create/delete progress and results moved from the `>` placeholder and the status line into
    the activity pane (the status line is gone). The list spinner stays in the `>` placeholder:
    a list is not an operation, and showing it in the activity pane would replace a create's
    result with the refresh that follows it.
  - Popup 720 px wide; the list and details panes are a fixed 8 rows (192 px) high so the
    popup doesn't change height as snapshots come and go. Help, About, the error view and
    details scroll if they don't fit.
  - Rows use iced's `ellipsize` (one line, `…` at the end) with wrapping off, so a long comment
    can't wrap into a second line of a fixed-height row. The old 28-character cut stays as an
    upper bound.
  - Help strings shortened to fit the narrower left pane.
  - The `>` line moved below the activity pane; the key hints are now the last row (footer).

- 2026-09-25 - Phase 3.5 fixes after the user's first look in the panel.
  - Reported: every pane was see-through (the desktop showed through it), and the pane borders
    were square. The view was still wrapped in `applet.popup_container`, so the popup
    background hadn't changed. The only new rendering in Phase 3.5 was the `Stack`: its title
    layer is drawn with `renderer.with_layer`, above a `Border`-only container. The cause wasn't
    confirmed (Claude can't see the panel); the fix removes that construct.
  - Panes are now built from plain filled containers in the popup's own layer, with no `Stack` and
    no `Border` lines: each piece is a fill in the line colour (accent or
    `background(transparent).divider`) holding a fill in `background(transparent).base`, inset by
    1 px on the sides that show a line. The top row is a corner piece, the title, and a top-right
    piece, half a line high and bottom-aligned so the line runs through the title's middle. Outer
    corners use `corner_radii.radius_s`, the inner fill that minus 1 px. So each pane is opaque
    in the theme background, with the theme's corners, whatever the popup's own fill does.
  - Pane height: the snapshots pane fits the list, `clamp(len, 5, 8)` rows, then scrolls; help
    and errors use 8 rows, About 6, the other states 5. The details pane takes the same height.

- 2026-09-25 - Phase 4 privileged helper (`crates/apsis-helper`). Written and unit-tested by
  Claude; never installed or run as root by Claude. The user installs and tests it.
  - Install layout agreed with the user before any file was written: `/usr/libexec/apsis-helper`,
    D-Bus activation + bus policy under `/usr/share/dbus-1/`, a `Type=dbus` systemd unit (the
    user chose to keep it), and the polkit `.policy`. Details in ARCHITECTURE.md.
  - The user's changes to the plan: polkit subject is the caller's unique bus name
    (`system-bus-name`), not a PID; interactive only for create/delete; the helper never exits
    while Timeshift runs; a second call is refused with `Busy`, not queued; reload with the bus's
    `ReloadConfig` via `busctl` (works for dbus-broker and dbus-daemon) instead of a unit name;
    all shared names in one module, `apsis_core::helper::names`.
  - Long operations: `Create`/`Delete` return once Timeshift has started, and a `Finished(op, ok,
    message)` signal follows. The alternative was one blocking call with a long timeout. The
    signal was chosen because (a) a second call can get `Busy` straight away rather than wait
    behind a running create; (b) nothing depends on anyone's call timeout (`busctl`/`gdbus` use
    25 s); (c) closing the popup leaves no reply pending while the operation carries on. The
    signal is unicast (destination = the caller), so other users on the bus don't see snapshot
    names or errors. The applet also watches the helper's bus name, so a helper that dies
    mid-operation is reported instead of a spinner that never stops.
  - Checked in the zbus 5.19 source (cargo cache): no default method timeout; interface
    methods run in their own tasks by default (so a list or a password dialog doesn't hold up
    other calls); `SignalEmitter::set_destination` exists; signal streams on a well-known name
    follow owner changes (the helper only appears during the first call).
  - No new crates: zbus 5.19 (with tokio) was already in the tree via libcosmic;
    `futures-util` too. `zbus_polkit` wasn't in the cache, so the one `CheckAuthorization` call
    is written out (`apsis-helper/src/polkit.rs`). polkit errors count as "not authorised".
  - `List` returns parsed data, `(sssa(sss))`, not Timeshift's raw text: the helper parses it
    anyway to find the device, and the applet re-checks names and tags (`helper::from_wire`).
  - Create/Delete in the helper run a fresh `--list` first and use the device it reports, since a
    helper started by D-Bus has no earlier list. Delete also refuses a name that list doesn't have
    (`Error::NoSuchSnapshot`).
  - Timeshift failures cross the bus as one message: a header line with the exit code, then the
    last 20 stderr lines (`helper::encode_failure` / `failure_error`). Other failures are plain
    text and show as-is.
  - `HelperClient` is async, not a `Backend` (ARCHITECTURE.md had planned it as one): waiting for a
    signal doesn't fit a blocking trait, and the applet already runs on tokio. The pkexec path still
    goes through `Backend` on a blocking thread.
  - Applet: before each list/create/delete it asks the bus whether the helper is activatable or
    running. If so it uses the helper, else pkexec. An installed helper that fails is shown as an
    error, not silently replaced by pkexec. New `CliError::NotAuthorized` (polkit said no through
    the helper), handled like pkexec's 126/127: no refresh afterwards.
  - The runner clears the environment and uses a fixed `PATH`, so nothing from D-Bus activation's
    environment picks the program run as root. `HOME`, `USER` and `LOGNAME` are set like pkexec
    sets them, in case Timeshift reads them.
  - The systemd unit isn't sandboxed: Timeshift mounts devices and rsyncs `/`.
  - Not verified here (no root, no way to install): that polkitd accepts the
    `CheckAuthorization` reply shape as deserialised; that dbus-broker delivers the unicast
    `Finished` under the installed policy; `systemd-analyze verify` on the unit (read-only
    filesystem in Claude's sandbox). A staged `just --set rootdir <tmp> install`/`uninstall` ran
    fine as a normal user, which skips the system reloads.
  - pedantic clippy (`just check`) still flags one pre-existing `match_same_arms` in
    `AppModel::prompt` (Phase 3.5).

- 2026-09-25 - Phase 4 test round (user tested the installed helper): fixes before commit.
  - **Disk drop.** A create through the helper worked (journal: started 14:51:57, done 14:53:53),
    but the refresh after it failed with only `timeshift exited with code 1`. The USB backup disk
    had dropped off. `sudo timeshift --list` said `E: Device not found: '/dev/sda1'`,
    `E: Failed to remove directory`, `Ret=256`. Apsis never showed that because **Timeshift prints
    its `E:`/`W:` lines on stdout**, and Apsis only kept stderr (empty).
    - Now a failed run keeps the `E:`/`W:` lines from stdout plus stderr, the last 5
      (`MAX_OUTPUT_LINES`), in `Error::Failed { code, output }` (renamed from `stderr`). The same
      text reaches the popup through the pkexec path and through the helper
      (`helper::encode_error`/`decode_error`).
    - `E: Device not found: '<device>'` becomes `Error::DeviceNotFound`. The helper sends it as its
      own D-Bus error (`...Error.DeviceNotFound`). The popup shows `backup disk not connected (UUID
      1a2b…): plug it in and press r`, with the UUID from the last good list (first 4
      characters), or Timeshift's name if no list has been seen. No automatic refresh after it: a
      list would only fail again.
    - Device targeting was already UUID-first (`--snapshot-device <UUID>`, the path only if a list
      had no UUID, and a failed list keeps the previous one). It's now pinned by tests in core and
      the helper, and the helper logs which device each list targets. The `/dev/sda1` above came
      from the user's manual run without `--snapshot-device`. What Apsis's own refresh passed
      wasn't logged at the time; the new log line shows it. A freshly started helper has no
      device yet, so its first list uses Timeshift's configured device, as before.
  - **Parser decision.** After the disk was back, a list succeeded but ended with an extra
    `E: Failed to remove directory` (probably a stale `/run/timeshift/<pid>/backup` mount), and the
    parser failed (`line 17: unexpected snapshot row`). Now `E:`/`W:` lines anywhere are
    diagnostics: they go into `SnapshotList::warnings` and are shown in the activity pane
    (`list: E: ...`, theme warning colour). A table line that isn't a snapshot row is also a
    warning (`line N: not a snapshot row: ...`) instead of `Error::BadRow` (removed). A list fails
    only on a non-zero exit, or when there's neither a table nor `No snapshots found`. The
    trade-off: a real format change would show up as warnings and missing rows, not as a failed
    list; the warnings make that visible. New fixtures: `list-rsync-stale-mount.txt` (the redacted
    device fixture plus the trailing line) and `list-device-not-found.txt` (rebuilt from the three
    lines the user quoted).
  - Wire change: `List` returns `(sssa(sss)as)`, with the warnings added. The interface isn't
    released yet, so it stays `Helper1`.
  - **Logging.** Every `List`, `Create` and `Delete` is logged with its result: `list for :1.42
    (device <uuid>): ok, 5 snapshots` (plus warnings), or `failed, exit code 1: E: ... | E: ...`, or
    `refused: not authorised` / `busy ...`. Create logs the comment quoted and cut to 40 characters.
    Delete logs the name quoted. polkit errors are logged before they count as "not authorised".
  - **[c] needed ~3 presses, and clicks didn't focus the `>` line.** Cause, from libcosmic's
    `text_input` (03d7dcb): Esc or Tab in the input sets its internal `is_read_only`, and for a plain
    input the widget copies that state back on every rebuild. A click only clears it when the input
    isn't focused, but `always_active` keeps it "focused" permanently. So after any Esc (closing
    help, cancelling a prompt) the input ignored typing and clicks until something called
    `text_input::focus`, which does clear it. On top of that, the prompt row added a label widget
    before the input when a prompt opened, which moved the input to a new spot in the widget tree
    (a new, fresh state), racing with the focus task.
    - The row is now always `>`, label, input (the label is a space when there's no prompt), so
      the input never moves. `on_unfocus` sends `InputUnfocused`, which refocuses at once (clearing
      read-only). `focus_input()` is now focus followed by move-cursor-to-end. The `c` or `d` that
      opens a prompt is handled as a command, not typed: in command mode the input's value stays
      empty, and the next keypress is the first character of the comment.
    - Not verifiable in unit tests (no widget tree there); the tests cover the model side (one `c`
      opens an empty prompt, a second `c` is text, losing focus keeps what was typed).
- 2026-09-25 - Window mode (`apsis --window`), after the user reported that starting Apsis from
  the app launcher showed only a tiny square with the panel button. Written and unit-tested by
  Claude; the user tested the window and it works.
  - **Panel detection**, from libcosmic 03d7dcb: `applet::Context::default()` reads
    `COSMIC_PANEL_NAME` (set by cosmic-panel for its applets) into `panel_type`, and libcosmic's
    own `autosize_window` treats `PanelType::Other("")` (unset or empty) as "not in a panel".
    `main.rs` uses the same test: outside a panel, or with `--window`, Apsis runs
    `cosmic::app::run` instead of `cosmic::applet::run`. So `just run` now opens the window.
  - One `AppModel` with a `Mode` flag, not a second app. In window mode `popup` holds the main
    window's id, so key routing, the spinner and the `>` line focus work exactly as in the popup.
    Esc's last step is `window::close` instead of `destroy_popup`; closing the main window quits
    (libcosmic's `exit_on_main_window_closed`). `view()` shows the popup's contents
    (`surface()`), the panes fill the height instead of 5-8 rows, and the window keeps libcosmic's
    opaque default style instead of the applet's transparent one.
  - Size 720 x 520 with min = max and `resizable(None)`, so COSMIC floats it. Header bar kept
    (title, close, drag to move), without maximize and minimize.
  - Launcher entry `resources/launcher.desktop`, expanded by `build.rs` like the applet's entry
    and installed as `io.github.atraxsrc.Apsis.Window.desktop`. The applet's entry keeps its file
    name (panel configs refer to it). The window's app ID is still `io.github.atraxsrc.Apsis`;
    `StartupWMClass` in the launcher entry points the dock at it. If the dock picks the applet's
    entry instead, launching from there still opens the window (outside the panel).
  - The applet's entry already had `NoDisplay=true` (from the template), in the source and in
    `target/xdgen/app.desktop`. If the launcher still showed it, the cause is elsewhere (an older
    installed copy, or a launcher ignoring `NoDisplay`); not verified by Claude.
- 2026-09-25 - Phase 4.5 Settings (written and unit-tested by Claude; nothing written to the
  real `/etc/timeshift/timeshift.json` by Claude). Read from Timeshift's source
  (linuxmint/timeshift e7e54ab, cloned read-only into `.scratch/` with the user's OK) and the
  user's real settings file (pasted, redacted).
  - **Placement.** "After Phase 4, before the superfile phase" couldn't both hold (3.5 comes
    before 4); the user chose after Phase 4.
  - **File format.** Every value is a JSON string (`"true"`, `"5"`); Timeshift reads them with
    `get_string_member`, which a JSON bool or number breaks. Apsis refuses to edit a file where a
    Timeshift field isn't a string, writes only strings, and writes like json-glib (2-space
    indent, `"key" : value`, no final newline), so an unchanged file stays byte for byte the
    same. Fields Apsis doesn't edit (including unknown ones) stay in place. The legacy
    `include_btrfs_home` field wins over `include_btrfs_home_for_backup` in Timeshift; when it's
    there, both are written.
  - **Fixture.** `tests/fixtures/config-rsync.json` is the user's file with the device UUID
    zeroed and usernames replaced (`user1`, `user2`). `parent_device_uuid` was `""` before
    redaction (user confirmed; a plain partition, whose disk has no UUID). The user's redaction
    had `"key": value` on two lines; the fixture uses json-glib's `"key" : value` throughout.
    `tests/fixtures/lsblk.json` is synthetic (placeholder UUIDs).
  - **Schedule = cron.** Timeshift schedules through `/etc/cron.d/timeshift-{hourly,boot}`, not
    the JSON. Every CLI run calls `cron_job_update()` on exit (and never saves the JSON in CLI
    mode), so the helper runs one `timeshift --list` after writing. The GUI (`timeshift-gtk`)
    does save the JSON when it closes, so the view warns while it's open.
  - **User decisions** (2026-09-25):
    - `/home`: mirror Timeshift. btrfs mode: the `@home` flag. rsync mode: per user (root and
      uid >= 1000 except 65534, from `/etc/passwd`), three states written with Timeshift's exact
      patterns (`<home>/**` exclude, `+ <home>/.**` hidden only, `+ <home>/**` everything); the
      chosen one is appended, the other two removed. ecryptfs homes use other patterns and are
      shown read-only.
    - Retention counts 1-999, like Timeshift's spin buttons. 0 would make Timeshift's cleanup
      untag, and so delete, every uncommented snapshot of that level. The spec said
      "non-negative"; changed at the user's choice.
    - Filters: anything Timeshift's editor takes (not blank after an optional `+ `), plus no
      control characters or line breaks (they'd break rsync's filter file), no duplicates. Not
      "absolute paths only": Timeshift's own example is `*.mp3`.
    - Devices: `timeshift --list-devices` prints no UUIDs, so the helper runs `lsblk --json`
      (Timeshift runs lsblk too) and applies Timeshift's `has_linux_filesystem` list. Only
      unencrypted filesystems with a UUID are selectable (not LUKS/LVM/ZFS containers, not
      inside a `crypt` mapping). A LUKS device already in the file is kept as is.
  - **Device rule.** A *changed* device must be connected; an unchanged one may be unplugged
    (Timeshift keeps it too), so the schedule can be edited with the USB disk away. Switching to
    btrfs mode needs the device connected and btrfs. `parent_device_uuid` comes from lsblk when
    the device changes, as Timeshift sets it, and stays when it doesn't.
  - **Found while building.** The helper remembers the last listed device and passes it as
    `--snapshot-device`; after a device change, lists and creates would have kept targeting the
    old disk until the helper exited. It now forgets the device after a write
    (`TimeshiftCli::forget_device`).
  - **Concurrency.** `WriteSettings` sends the file text the view read; the helper refuses
    (`Changed`) if the file differs, checked again just before the rename. It holds the
    single-operation lock throughout, and refuses while a list, create or delete runs.
  - `ReadSettings` uses the `list` polkit action (no password for the active session): the
    file is world-readable anyway. There's no pkexec fallback for settings.
  - Keys `s`, space, `+`/`-`, `e`, `a`, `x`, `w` are new; `x` was the tests' example of an
    unmapped key, now `z`. In iced 0.14 space arrives as a character, not `Named::Space`.
- 2026-09-25 - The `--window` window is resizable (user request), replacing the fixed 720 x 520.
  It opens at 720 x 520, minimum 640 x 440 (the settings footer, the widest, still fits), no
  maximum, 8 px resize border (`cosmic::app::Settings::resizable`). The panel popup is
  unchanged.
  - **Starting floating** (user asked, "if possible"). From cosmic-comp's source (e5010c2,
    cloned read-only into `.scratch/` with the user's OK): `Shell::map_window` decides floating
    or tiled once, when the window first maps. It floats a dialog (`layout::is_dialog`: a parent,
    or min size == max size), a floating exception (app ID and title regexes in COSMIC's window
    rules), or anything when tiling is off. Nothing re-checks later. So the window maps with
    min = max = 720 x 520 (floats, like the old fixed window), and on its first `Focused` event
    (or 1.5 s after start, if focus never comes) Apsis drops the maximum and sets the minimum to
    640 x 440 (`window::set_max_size` / `set_min_size`). It stays in the floating layer and
    becomes resizable. If focus and the timer both came before the map, it would tile as any
    window does; not seen, and not testable in unit tests.

- 2026-09-25 - Phase 5 v1: native rsync backend (`apsis-core::native`, helper
  `NativeList`/`NativeDryRun`/`NativeCreate`). Written and tested by Claude on files it
  created. Never run as root by Claude. Nothing written to the real
  `/etc/timeshift/timeshift.json` or to real snapshots. btrfs is Phase 5.1, scheduling 5.2.
  - **Source.** Everything below is from linuxmint/timeshift **e7e54ab (tag 26.09.0)**, the
    shallow clone in `.scratch/timeshift`. **The user runs v24.01.1**, and the clone has no
    history, so differences between the two versions **could not be checked**. The references
    are `file:line` in that clone.
  - **User decisions** (2026-09-25): the user mounts the ext4 test image and Claude runs the
    tests (loop devices need root); the helper runs the native backend, with dry run on by
    default; `app-version` in a native snapshot is `apsis <version>`, not Timeshift's version
    (Timeshift only stores the field, `Snapshot.vala:226`).
  - **Layout copied from Timeshift:**
    - Folders: `<mount>/timeshift/snapshots/<name>/` (`SnapshotRepo.vala:159-174`), where `<name>`
      is the local start time, `%Y-%m-%d_%H-%M-%S` (`Main.vala:1588-1591`). The copy of `/`
      is in `localhost/` (`:1593-1594`). `snapshots/` is created if missing (`:1133-1137`).
    - `info.json` (`Snapshot.vala:396-436` writes it, then `set_tags` rewrites it through
      `update_control_file`, `:341-393`, `Main.vala:1711-1718`, `:1835-1867`). It has nine
      string members, in this order: `created` (Unix seconds, UTC), `sys-uuid`, `sys-distro`,
      `app-version`, `file_count`, `tags`, `comments`, `live` (`"false"`), `type` (`"rsync"`).
      json-glib pretty print, indent 2. On-demand: `tags` is `"ondemand"` (`initial_tags` is
      `""` for on-demand, `Main.vala:1697`, then `set_tags` adds `ondemand` because no
      `--tags` was given). An empty comment is `""`.
    - The format is Apsis's existing json-glib writer (`settings::write_object`): `"key" :
      value`, 2 spaces, no final newline. That writer reproduces the user's real
      `timeshift.json` byte for byte (Phase 4.5), and json-glib writes both files the same
      way.
    - Reading (`Snapshot.read_control_file`, `:190-290`): every member via
      `get_string_member`, missing ones default (`created` 0, `type` `rsync`). `tags` is split
      on single spaces (`taglist`, `:147-154`). A snapshot is valid only with a parseable
      `info.json` and an `exclude.list` (`:205-213`, `:285-288`, `:293-320`). `.sync` is
      skipped (`SnapshotRepo.vala:314`). Sorted by `created` (`:335-339`).
    - `sys-distro`: `LinuxDistro.full_name()`, `ID RELEASE (CODENAME)` from `lsb-release`, else
      `os-release` (`LinuxDistro.vala:45-149`). `sys-uuid`: the UUID of the filesystem mounted
      at `/` (`Main.vala:3738-3760`).
    - `--link-dest`: `<newest valid snapshot with the same sys-uuid>/localhost/`
      (`get_latest_snapshot("", sys_uuid)`, `SnapshotRepo.vala:376-406`, `Main.vala:1632-1640`).
    - rsync, as argv (Timeshift writes a bash script, `RsyncTask.vala:184-260`, with the
      options from `Main.vala:1656-1673`): `rsync -aii --recursive --verbose --delete --force
      --stats --sparse --delete-excluded [--link-dest=<prev>/localhost/]
      --log-file=<snap>/rsync-log --exclude-from=<snap>/exclude.list --delete-excluded /
      <snap>/localhost/`. `--delete-excluded` appears twice, as in Timeshift. `LC_ALL=C.UTF-8`
      (`RsyncTask.vala:186`).
    - Success check: a `total size is N  speedup is X` line with N > 0 (`RsyncTask.vala:154-155`,
      `:511-513`; `Main.vala:1691-1695`). `file_count` is the number of `\n` in `rsync-log`
      (`Main.vala:1711`, `TeeJee.FileSystem.vala:91-113`). rsync's `--log-file` also gets the
      stats lines (checked with rsync 3.2.7), so Apsis reads the total from `rsync-log` and
      discards stdout.
    - `exclude.list` (`create_exclude_list_for_backup`, `Main.vala:809-917`; written by
      `save_exclude_list_for_backup`, `:996-1014`, one pattern plus `\n` per line, blank
      patterns skipped). The order: the user filters from `timeshift.json` (minus defaults and
      home entries, `:3516-3529`), the defaults (`:660-685`, `:721-731`), `<mount>/*` for each
      non-standard `/etc/fstab` mount point (`:687-718`, `FsTabEntry.vala:59-121`), the fixed
      extras (`:735-754`), `<home>/**` for ecryptfs homes and private folders (`:840-867`,
      `SystemUser.vala:74-198`), then `/root/**`, `/home/*/**` (`:760-761`), then `/timeshift/*`
      if it's missing. Duplicates are skipped at each step except the ecryptfs one, as in
      Timeshift.
    - Tag folders (`create_symlinks`, `SnapshotRepo.vala:903-944`): the six
      `snapshots-{boot,hourly,daily,weekly,monthly,ondemand}/` are deleted and re-created, and
      each valid snapshot gets `snapshots-<tag>/<name> -> ../snapshots/<name>` per tag.
    - Timeshift's lock: `/var/run/lock/timeshift/lock`, `<pid>;<mode>`, and it counts as held
      only if `/proc/<pid>/exe`'s name contains `timeshift` (`AppLock.vala:33-60`,
      `TeeJee.Process.vala:290-301`).
  - **Deliberate differences:**
    - **Staging folder.** Timeshift builds a snapshot in `snapshots/<name>/` and relies on its
      lock. A scheduled Timeshift run removes every snapshot folder without `info.json` or
      `exclude.list` as incomplete (`SnapshotRepo.vala:801-811`). Timeshift's lock treats any
      process not called `timeshift` as stale, so Apsis can't hold it. So a native create
      builds in `timeshift/apsis-staging/<name>/`, which Timeshift never reads, and renames the
      finished folder into `snapshots/`. The final layout is the same. A failed create removes
      its staging folder; a crash leaves one, which the native list reports as a warning.
    - **rsync exit code.** Timeshift ignores it and only checks the total size. Apsis also
      accepts only 0, 23 (some files unreadable) and 24 (files vanished). Anything else (e.g. 11
      for a full disk) fails, even when a total size was printed.
    - **Retention and `.sync-restore`.** Not done natively in v1. Timeshift applies retention
      on its next run. `.sync-restore` (link against the restored snapshot after a restore,
      `Main.vala:1600-1630`) is ignored: native always links against the newest snapshot. That
      only affects how much gets hard-linked, never correctness.
    - **I/O priority.** For command-line runs Timeshift starts rsync at idle I/O priority
      (`AsyncTask.vala:138-141`, `Main.vala:1673`). Apsis doesn't (no `ionice` in the argv).
    - **Mount.** The helper mounts the device itself at `/run/apsis/backup` by UUID, with
      `nosuid,nodev`, read-only for list and dry run. Timeshift uses
      `/run/timeshift/<pid>/backup` with no options (`Device.vala:1571-1640`). Encrypted
      (LUKS) backup devices are refused: Timeshift unlocks them; the native backend doesn't.
    - **Symlinks.** The native backend refuses to write if `timeshift/`, `snapshots/` or
      `apsis-staging/` on the backup device is a symlink.
  - **Guesses, flagged (not checkable from the source alone):**
    - **24.01.1 vs 26.09.0**: see Source above. Also, the fixture snapshot tree
      (`tests/fixtures/native-repo/`) was **written by hand from the 26.09.0 source**, not taken
      from a real Timeshift snapshot. Its `app-version` is `24.01.1` and it has a second tag
      (`ondemand daily`) to exercise parsing. A real `info.json` + `exclude.list` from the user's
      machine (redacted) should replace or back it up.
    - **Non-ASCII in `comments`**: Apsis writes it as raw UTF-8 with only `"`, `\` and control
      characters escaped (serde_json's rules, the same the settings writer uses). That json-glib
      does exactly the same is assumed, not verified against a real file with non-ASCII
      comments. Control characters can't occur (Apsis refuses them in comments).
    - **The ecryptfs exclude order with several users**: Timeshift iterates a hash map (order
      undefined); Apsis uses `/etc/passwd` order. It only shows with two or more ecryptfs
      homes or private folders.
    - **Timeshift's second per-user step** (`Main.vala:869-899`): it changes the settings list,
      not the list being built, so it only affects a list built twice in one run. Apsis builds
      the first-call list (reasoning in `native/exclude.rs`). It's the same whenever every user
      has a home filter in the settings.
    - **`sys-uuid` via findmnt**: Timeshift takes the device lsblk shows mounted at `/`,
      skipping loop devices. Apsis asks `findmnt` for the UUID of `/`. They should agree
      (same filesystem UUID), but it's not verified on LVM/LUKS roots.
  - **Mistake caught while checking citations**: at first Claude read Timeshift's
    `parse_line_passwd` as always returning `null` (so no users, so no per-user excludes). The
    excerpt it was reading skipped lines 126-139; the `return null` is in the `else` branch
    for malformed lines. The ecryptfs step is ported.
  - **Settings storage**: Apsis's cosmic-config (`native_backend`, default false;
    `native_dry_run`, default true), per user, saved as soon as it changes in the settings
    view. Timeshift's file is never used for it (no invented fields).
  - **Applet**: native list and create need the helper (no pkexec path). Delete always goes
    through Timeshift. A dry run's plan is shown in the left pane and logged to the journal.
  - **Tests** (`crates/apsis-core/tests/native.rs`, real rsync): each scenario runs in
    `target/tmp/native/`, and again on the ext4 image when `APSIS_EXT4_MNT` is set (the test
    checks, through `findmnt`, that it is ext4 on `/dev/loop*`). `just ext4-image` builds the
    128 MB image without root (`mkfs.ext4 -E root_owner`); the user mounts it; `just
    test-ext4` runs them.
  - Sandbox note: when the shell's working directory was inside `crates/`, the tool harness
    created empty `.claude/.cc-writes` folders there, which broke the `crates/*` workspace glob.
    They were removed with `rmdir` (empty only).
  - 2026-09-26: `just test-ext4` passes (11/11) on the loop-mounted image, mounted by the user.
    The first run failed in the test's own guard: inside Claude's sandbox `findmnt` lists the
    mount twice (same `/dev/loop0`, two mount IDs, the sandbox re-binding the repo). The guard
    now accepts several lines as long as every one is ext4 on `/dev/loop*`.

- 2026-09-26 - Phase 5: native backend re-checked against **Timeshift 24.01.1**, the version the
  user runs (tag `24.01.1` checked out in `.scratch/timeshift`, diffed against `26.09.0`).
  Apsis now follows 24.01.1, and every native `file:line` citation points at 24.01.1 (the
  Phase 5 entry above keeps its 26.09.0 numbers as history). `settings.rs` (Phase 4.5) still
  cites e7e54ab; its code paths weren't part of this check.
  - **Same in 24.01.1** (checked line by line): `info.json` members, order and format
    (`Snapshot.vala:394-434`, `:339-386`); the rsync argv, including the doubled
    `--delete-excluded` and `LC_ALL=C.UTF-8` (`RsyncTask.vala:175-255`, `Main.vala:1538-1559`);
    `--link-dest` = newest valid snapshot with the same `sys-uuid`, plus `/localhost/`
    (`Main.vala:1510-1520`, `SnapshotRepo.vala:372-402`); the total-size success check
    (`Main.vala:1577-1581`); `file_count` (`wc -l`, `TeeJee.FileSystem.vala:91-97`, the same
    count of `\n`); tag folders (`SnapshotRepo.vala:929-991`, made with `ln` instead of GIO,
    same links); mount by UUID with no options; the folder layout; `sys_root`
    (`Main.vala:3523-3545`); filters loaded from `timeshift.json` (`Main.vala:3345-3360`);
    `SystemUser.vala` and `FsTabEntry.vala` are identical.
  - **Different, and changed in Apsis:**
    - **exclude.list order.** 24.01.1 (`Main.vala:702-807`): defaults, fstab mounts + fixed
      extras, ecryptfs entries, **then the user's filters**, then `/root/**`, `/home/*/**`,
      `/timeshift/*`. 26.09.0 puts the user's filters first. rsync takes the first matching
      rule, so this changes what's backed up: in 24.01.1 a `+ /home/x/**` include can't win
      over the default `/home/*/.cache` exclude.
    - **The per-user step** (`Main.vala:751-781`) runs *before* the user's filters are copied
      in 24.01.1, so it counts from the first list: each non-system user (root included)
      whose home has neither `+ <home>/**` nor `+ <home>/.**` gets `<home>/**` appended to
      the user filters (`/home/.ecryptfs/<name>/***` for an ecryptfs home). The old note
      that this step "only shows in a second list" was about 26.09.0 and no longer applies.
    - **sys-distro without lsb-release** (`LinuxDistro.vala:60-153`): 24.01.1 uses
      lsb-release whenever it exists (even empty) and only reads `DISTRIB_*` keys there; its
      os-release fallback reads `ID` and `VERSION_ID` with the quotes kept and no codename
      (Fedora: `fedora "42"`). On Pop!_OS both give `Pop 24.04 (noble)`.
  - **Different, kept as is:**
    - **Lock.** 24.01.1 treats the lock as held while *any* process has that PID (`ps --pid`,
      `AppLock.vala:39-50`, `TeeJee.Process.vala:294-313`); 26.09.0 only when that process is
      a Timeshift. Apsis keeps the stricter "is a Timeshift" test, since the point is not to
      run alongside Timeshift, and a stale lock whose PID was reused shouldn't block it.
      Note: 24.01.1 would honour a lock written by Apsis, 26.09.0 wouldn't, so the staging
      folder stays.
    - **ionice.** 24.01.1 has the `ionice` line commented out (`RsyncTask.vala:179-181`), so
      neither runs rsync at idle I/O priority. The "I/O priority" difference in the Phase 5
      entry only applies to 26.09.0.
    - **`rsync-log-changes`.** Both versions write `<snapshot>/rsync-log-changes` (a digest of
      created/deleted lines). Apsis doesn't; Timeshift's log viewer builds it from `rsync-log`
      the first time it's opened (`RsyncTask.vala:257-280`, `MainWindow.vala:707`).
    - 26.09.0-only features (post-backup hooks, snapshot pause) don't exist in 24.01.1.
  - **Real fixtures.** The user supplied `info.json` and `exclude.list` from a snapshot made by
    Timeshift 24.01.1 (redacted by the user: UUIDs zeroed, homes renamed to `/home/userN`).
    They replace the hand-made pair in `tests/fixtures/native-repo/` byte for byte; the
    snapshot's folder name, `localhost/` and `rsync-log` are still hand-made (the tests link
    against them), and `snapshots-daily/` went with the old second tag. New tests: Apsis's
    writer reproduces the real `info.json` exactly, and the exclude builder reproduces the
    real `exclude.list` exactly, from settings read back from that list (user filters `+
    /root/**`, `+ /home/user1/**`, `/var/lib/libvirt/**`, `+ /home/user2/**`; `/recovery` in
    fstab). The two `+ /home/userN/**` lines must have been two homes before redaction: the
    builder drops exact duplicates. Checking against the user's actual `exclude` array in
    `/etc/timeshift/timeshift.json` would confirm the inferred settings.
  - With the old 26.09.0 order, the builder would **not** have produced the real list: the
    four user filters would have come first.

- 2026-09-26 - **Phase 5 v1 done.** Real-disk results, run by the user as root on their machine
  (Claude ran nothing as root):
  - Dry run: the plan matched Timeshift: the 60-line exclude list, `--link-dest` to the newest
    snapshot, the rsync argv and `info.json`.
  - Real native create `2026-09-26_07-31-23`, comment "native test 1": `sudo timeshift --list`
    lists it as one of its own.
  - `du`: previous snapshot 235G, the new native snapshot 3.3G, so unchanged files are
    hard-linked.
  - Left over (recorded in PLAN.md): native retention, idle I/O priority for the native rsync,
    Btrfs (Phase 5.1), and recognising Timeshift's `Ret=NNN` lines as diagnostics.

- 2026-09-26 - **Phase 6a: file-level restore** (design in PLAN.md, approved with changes 1-12,
  the `rustix` dependency and `r` reload / `R` restore). Written and tested by Claude on files
  it created; nothing run as root, nothing restored on the real system. Full-system restore
  (6b) not started.
  - **User amendments** (2026-09-26): (A) folder mode adds `--no-devices --no-specials`, so
    root never makes a device node or FIFO owned by the user; (B) the hand-over never follows
    a symlink (rsync's `--chown` for the contents, one `fchown` on the top folder's descriptor),
    every component of the destination is opened from `/` with `O_NOFOLLOW | O_DIRECTORY`,
    `~/Apsis-restored` must be the caller's, and rsync writes through the held descriptor
    (`/proc/self/fd/<n>/`, the descriptor inherited by rsync); (C) polkit split:
    `restore` (folder) `auth_admin_keep`, new `restore-original` `auth_admin` (every time),
    dry runs on `browse`.
  - **How rsync writes through the descriptor.** `FD_CLOEXEC` is cleared on the pinned
    folder's descriptor for the length of the rsync run only (`dest::Pinned::inherited`, a
    guard that sets it back), so rsync has it at the same number, and rsync's destination is
    `/proc/self/fd/<n>/`. In rsync's process (and the receiver it forks) that magic link is its
    own copy of the descriptor: the folder the helper made, wherever it's been moved. The test
    (`rsync_writes_through_the_held_folder_not_the_path`) renames `Apsis-restored/` away and
    puts a symlink to a decoy in its place right before rsync starts; the files land in the
    moved folder and the decoy stays empty. The helper runs one operation at a time, so no
    other program is started while the descriptor is inheritable.
  - **rsync checked by hand first** (3.2.7, in the scratchpad): a `--dry-run` into a missing
    folder prints `created directory` but makes nothing; `--no-specials` skips a FIFO with
    `skipping non-regular file` on stdout; `--chmod=ug-s` drops setuid; `--backup --suffix`
    renames the replaced file; `--filter=-x <pattern>` drops matching xattrs (checked with
    `user.*` names, since only root can set `security.*`).
  - **Choices made while building:**
    - The plan is read from `--out-format='APSIS %i %l %n%L'` (itemized changes plus the
      size), not `--itemize-changes`, so the marker tells plan lines from rsync's other
      messages. Items that are already the same aren't printed by rsync, so the plan doesn't
      list them (the design's example had a `same ... (skipped)` line; dropped).
    - Browse's D-Bus type is `(a(sstxuuussstx)b)`; the design had one `t` too many.
    - D-Bus strings are UTF-8: a file name that isn't is listed with `�` and can only be
      restored with its folder.
    - `R` with nothing marked restores the selected entry.
    - `?` does nothing in the browser (help would replace the browser's pane); the help view
      from the list shows the browser's keys.
    - Original mode's missing-parent message says to restore the folder above instead.
    - New errors: `Error::InvalidInput` (refused before anything runs; over the bus as
      `InvalidInput`, and in `Finished` with a `refused: ` prefix) and `Error::Restore` (rsync
      failed). The client now maps every `InvalidInput` D-Bus error to `Error::InvalidInput`
      (settings refusals used to come back as `InvalidSettings`; the text shown is the same).
    - The helper's read-only mount now adds `noexec` for native list and dry run too.
  - **Not testable without root** (the user's manual tests cover them): ownership of restored
    files by another uid (tests run as the caller), `security.*` xattrs, device nodes (a FIFO
    stands in), the polkit dialogs, the real `/run/apsis/backup` mount.
  - **Sandbox note**: in this session cargo couldn't read `~/.gitconfig` (the sandbox denies
    it), so libgit2 treated the libcosmic git cache as broken and tried to re-clone it into
    the read-only `~/.cargo`. Claude ran cargo with `HOME` set to its scratchpad (and
    `CARGO_HOME`/`RUSTUP_HOME` set to the real ones); nothing in the repo changed for it. The
    `rustix` entry in `Cargo.lock` was added by hand for the same reason (1.1.5 was already
    locked, through libcosmic).

- 2026-09-26 - **Polish: browser marks.** Space marks or unmarks and keeps the cursor on the
  row, so the details pane shows what was just marked. `J` (vim-style, shift+j) marks and
  moves down, for marking a run; shift+space was the other option, but `J` needs no modifier
  handling in the key map and doesn't collide with anything (`K` stays unbound). Marked rows
  that aren't selected get the accent colour at 7% alpha (the selection uses 15%, so the
  selected row still reads as the cursor). The details pane adds `restore   marked`. The
  double-click on a file still marks in place.

- 2026-09-26 - **Phase 7 prep (v0.1.0), nothing tagged or pushed.**
  - libcosmic pin: tried `rev = "03d7dcb8..."` (the commit `Cargo.lock` already had), then
    dropped it at the user's request. `cosmic-panel-config` (pulled in by the `applet`
    feature) asks for libcosmic's git URL with no rev, so a `rev` put `cosmic-config`,
    `cosmic-config-derive`, `iced_core`, `iced_futures` and `build_helpers` in `Cargo.lock`
    twice (same commit, two sources), and `cargo vendor` refuses one name and version from
    two sources. A `[patch]` to the same URL with a rev is refused by cargo ("patches must
    point to different sources"). So the pin is `Cargo.lock` plus `--locked` in CI; bump with
    `cargo update -p libcosmic` on purpose.
  - CI (`.github/workflows/ci.yml`): ubuntu-latest, libcosmic's build deps from its README and
    CI (`pkgconf libexpat1-dev libfontconfig-dev libfreetype-dev libxkbcommon-dev
    libwayland-dev`), plus `rsync` for the native/restore tests. `--locked` so the pin holds.
    No test needs root; the ext4 tests only run with `APSIS_EXT4_MNT` set, which CI doesn't.
  - Metainfo: `<binaries>` inside `<provides>` (from the template) replaced by `<binary>`,
    which AppStream knows. `appstreamcli validate` still warns about the `COSMIC` category;
    kept, as COSMIC applets use it. Screenshot and release URLs point at the `v0.1.0` tag, so
    they resolve once it's pushed.
  - `just tag` isn't used for 0.1.0: the version is already 0.1.0 (empty commit) and it tags
    without the `v`. The manual steps are in `docs/RELEASE.md`.
  - The collection entries in `docs/RELEASE.md` are drafted without reading
    `applets.ron` / `applications.ron` (no network); their fields must be checked against the
    real files before the PR.
- 2026-09-26 - **Polish: `just vendor` writes `vendor.tar`.** The template recipe (noted
  2026-09-25 above) built `.cargo/config.toml` and then deleted it and `vendor/` without
  archiving, so `just build-vendored` failed at `tar pxf vendor.tar`. Now:
  `cargo vendor --locked > .cargo/config.toml` (its stdout is the source-replacement config,
  including the libcosmic git source, pointing at `vendor`), `tar pcf vendor.tar vendor .cargo`,
  then `rm -rf vendor .cargo`.
  - `--locked` so the tarball matches `Cargo.lock` (the libcosmic pin) and fails instead of
    updating it. The template's `--sync Cargo.toml` and `head -n -1` edit are gone: `--sync`
    only adds extra manifests, and the printed config is already complete.
  - `vendor-extract` removes `vendor/` and `.cargo/` before unpacking, so a stale config
    can't point at old sources.
  - The recipes delete `.cargo/`. The repo has no tracked `.cargo/` (only
    `/.cargo/config.toml` is ignored), so nothing of ours is lost, but a local
    `.cargo/config.toml` would be replaced.
  - **`--versioned-dirs` is required.** The user's first `just build-vendored` failed in
    `atspi-common` 0.13.0 (292 errors: E0119, E0432/E0433, E0308), while the normal build
    worked. Not a version or source mismatch: `Cargo.lock` has one version each of
    atspi/zbus/zvariant, the generated config covers every git source, and the vendored
    `atspi-common` matched its `.cargo-checksum.json`. Reproduced in a scratch copy, the
    first error is `#[validate(signal: ...)]` panicking with "File has no extension."
    `zbus-lockstep` 0.5.2 (`resolve_xml_path`) tries `$CARGO_MANIFEST_DIR/xml`, then
    `../xml` and more, and the last one that exists wins. In an unversioned vendor dir,
    `vendor/atspi-common/../xml` is the vendored `xml` 1.4.0 crate (via `xmltree`), so the
    macro reads that crate's folder, panics on `src/`, and drops the types it annotates
    (`ObjectRef` and others); the other errors follow from that. The registry cache names
    folders `xml-1.4.0`, so `../xml` doesn't exist there. With `vendor/xml` renamed to
    `xml-1.4.0`, `atspi-common` and then the whole workspace built `--release --frozen` in
    the scratch copy (1m 56s). `--versioned-dirs` names every folder `name-version`, so no
    vendored crate can be called `xml` again.
  - `build-vendored` is a bash recipe: it unpacks `vendor.tar`, builds
    `cargo build --release --frozen` (locked plus offline; the old extra `--offline` was
    redundant), and a `trap ... EXIT` removes `vendor/` and `.cargo/` whether the build
    passes or fails, so a vendored build never leaves source replacement active for plain
    `cargo` runs. `vendor-extract` still unpacks and leaves them, for packagers who want
    the tree.
  - The full `just vendor` (downloads every crate) wasn't run by Claude. The user retested:
    `just vendor` writes versioned dirs, `just build-vendored` builds release with no errors
    and leaves no `vendor/` or `.cargo/`, and the normal `cargo build` still works.

- 2026-09-26 - **.deb package (cargo-deb) and release workflow.** Target Pop!_OS / Ubuntu
  24.04, amd64.
  - `just deb [cargo args]`: `cargo build --release --workspace`, renders the two `.in`
    templates with the same sed and `libexec-path` as `install` into `target/deb-assets/`,
    then `cargo deb -p apsis --no-build` (`--no-build` because `apsis-helper` is another
    crate's binary). The asset list in `crates/apsis/Cargo.toml` mirrors `install` path for
    path; checked with `dpkg-deb -c` (11 files, plus cargo-deb's `copyright` and
    `changelog.Debian.gz`). cargo-deb 3.8 warns that `../../target/...` sources aren't cargo
    outputs it builds; expected, `just deb` makes them first.
  - Maintainer scripts in `resources/deb/` (reviewed by the user before the first build) do
    what `reload-system` and `uninstall` do: daemon-reload + bus `ReloadConfig` after
    install and remove, stop the helper before remove/upgrade. Each step tolerates a missing
    systemd or bus (chroots). The unit has no `[Install]`: it's D-Bus activated, nothing to
    enable.
  - `resources/deb/changelog` (Debian format) exists because lintian errors without one; it
    needs a new entry per release, like the metainfo `<release>`.
  - lintian 0 errors. Warnings kept: `desktop-entry-invalid-category COSMIC` and
    `desktop-entry-lacks-main-category` (the applet's `NoDisplay` entry, COSMIC applets use
    `Categories=COSMIC`, as for appstreamcli), `maintainer-script-calls-systemctl` (prerm's
    stop; the user chose to keep `systemctl` over `deb-systemd-invoke`),
    ~~`no-manual-page`~~ (gone since the man page, 2026-09-27), `initial-upload-closes-no-bugs`
    (Debian archive only).
  - `$auto` gives `libc6, libxkbcommon0`. `libwayland-client.so.0` is dlopen'd (winit /
    wayland-sys), so dpkg-shlibdeps can't see it; `libwayland-client0` is added to Depends
    by hand, at the user's request.
  - `.github/workflows/release.yml`: on `v*` tags, ubuntu-24.04, `permissions: contents:
    write` only, apt `just` + `dpkg-dev`, `cargo install cargo-deb --locked`,
    `just deb --locked`, then `gh release upload --clobber` with `GITHUB_TOKEN`; creates a
    draft release first if the tag has none yet. The tag name reaches the shell through `env`.

- 2026-09-26 - **Release 0.1.1 prepared** (the .deb, README "How to use", the vendor fix).
  The user tested the 0.1.0-1 .deb on Pop!_OS (installed with nala): 11 files, helper
  activatable, 7 polkit actions, create/delete, reinstall stops the helper, remove cleans up.
  - `resources/deb/changelog` starts at `0.1.1-1`: the `0.1.0-1` entry was dropped, since no
    0.1.0 .deb was ever published.
  - The metainfo screenshot URL stays on `v0.1.0`: that tag already has the current
    screenshot (the screenshot commits are before it), and it resolves today.
  - `Cargo.lock` bumped with `cargo update --workspace --offline`; only the three apsis crates
    changed.

- 2026-09-27 - **Polish: bulk delete, menu, `Ret=` lines.**
  - Bulk delete is one `Operation::DeleteMany { names, done }` that runs one helper `Delete`
    per step: `on_finished` starts the next step on success. No new helper method: `Delete`
    already checks the name against a fresh `--list`, takes the lock, and polkit's
    `auth_admin_keep` makes the password prompt a one-off. `running` stays set between steps, so
    no list or other operation slips in. The pkexec fallback works too, with a prompt per
    snapshot (said in UI.md rather than refused).
  - Marks are snapshot names (`BTreeSet`), not indexes, so a refresh keeps the right ones; the
    delete order is list order. After a failure the not-deleted ones stay marked, to retry.
  - The confirm line lists every name. The `>` row gives a bulk-delete label the width and
    lets it wrap, and shrinks the input (only `y` goes there) to a fixed 48 px.
  - Menu labels: looked at pop-os/cosmic-applets and a few community applets (read-only, with
    the user's OK for the network). The pop-os applets have no applet-level remove or settings
    item (cosmic-app-list's `Quit` quits apps, not the applet); community applets use
    `Settings…` and `Refresh`; the panel design issue (pop-os/cosmic-epoch#102) says an
    applet's menu should offer removing it. No common label, so `Remove or move applet…` as
    the user asked, and `Close` rather than `Quit`.
  - `Ret=<digits>`: Timeshift's `log_msg("Ret=%d")` after `Failed to remove directory`
    (`Main.vala`, from `.scratch/timeshift`). A bare `Ret=` or `Ret=25x` isn't matched.
    It is now also one of a failure's last lines, which shows the status next to the `E:` line.
  - Building in Claude's sandbox: `~/.cargo` is read-only there and the libcosmic git checkout
    can't update, so builds used the unpacked `vendor.tar` (via `--config` with an absolute
    `directory`) and a scratch `CARGO_HOME`. For `cargo clippy`, `--config` has to come after
    the subcommand; `cargo --config X clippy` loses it. Nothing in the repo changed for this.
  - After the user's test: delete text shows snapshot labels (`fmt::label`, `%m-%d %H:%M` plus
    the quoted comment), looked up in the current list when shown. `Operation` and the prompts
    still carry raw names, so the helper and its journal are unchanged. The format follows the
    user's example; the list rows themselves still show the year (`2026-09-27 09:12`). Single
    delete's prompt now also gets the wrapping label, since a comment can be long.

- 2026-09-27 - **Docs + small UI round:** README "Settings explained", man page, settings
  details that fit.
  - README: the settings table became a section per setting. The retention text comes from
    Timeshift 24.01.1's `SnapshotRepo.auto_remove` (`.scratch/timeshift`, lines 615-700): past
    `keep N` an older snapshot loses that level's tag, hourly/daily/weekly/monthly skip
    snapshots with a comment (boot doesn't), and a snapshot left without tags is deleted. The
    third filter example uses `***` on purpose: with `/home/you/**` excluding the rest of the
    home, `+ /home/you/Projects/**` doesn't match the folder itself, so rsync never enters it.
  - `docs/apsis.1`: plain `man` macros, no pandoc. `.nh` so polkit action names and paths
    aren't hyphenated. `groff -man -ww -z` gives no warnings. Installed as
    `/usr/share/man/man1/apsis.1.gz` by `just install` (and removed by `uninstall`) and in the
    .deb, both with `gzip -9n` (no name or time stamp, as lintian wants); `just deb` writes it
    into `target/deb-assets/`. lintian --pedantic: `no-manual-page` gone, no new tags.
  - Settings details: the text already wrapped (`WordOrGlyph`); what cut it off was height. A
    headless tiny-skia render (`renderer::Headless`, no window, nothing on the user's screen)
    showed the popup's fixed 8-row details pane cutting the backend note mid-sentence, and a
    narrow window needing its scrollbar. Now, in the popup's settings view only, the details
    pane is unscrolled with a zero-width spacer for the 8-row minimum, has a fixed width (the
    same 2/5 the portions gave, 275.2 px), the list pane is `Fill` high, and the panes row is
    `Shrink`. iced's flex then sizes the details first and stretches the list to match; with
    both panes `FillPortion`, the list was laid out before the details and got zero height. In
    a window the panes keep the window's height, so the notes were shortened to fit
    640 x 440 (the home one drops its "space cycles" line; the footer shows `[space]change`).
  - Test `settings_details_fit_without_scrolling_in_the_popup_and_a_small_window` lays out
    `surface()` for every settings row in both modes with the headless renderer and fails if a
    node sticks out of its parent in the details pane, the panes differ in height, or (popup)
    the height changes from row to row. Checked that it fails with each popup change switched
    off. **It only runs with `APSIS_LAYOUT_TEST=1`** (`APSIS_LAYOUT_TEST=1 cargo test -p apsis
    settings_details_fit`); otherwise it prints that it was skipped and passes. It measures
    real text with whatever fonts are installed, so on CI or another machine different font
    widths could fail it without anything being wrong in Apsis. Run it locally after changing
    settings notes or the panes' layout. If no headless renderer can start, it also skips.
  - After the user's test: the popup no longer changes height while moving between rows.
    `tallest::Tallest` (a small custom widget) lays out every row's details with the same
    limits, takes the tallest height, and draws only the selected one. iced has nothing that
    does this: `Stack` takes its base layer's size, and rows and columns add up. The details
    are text only, so the hidden ones need no events. The height is worked out on each layout
    from the rows there are, so it changes only when the rows do (a filter added, another
    device picked), not when moving.
  - The .deb here was built with a scratch `CARGO_HOME` whose `config.toml` points at the
    unpacked `vendor.tar`, so `cargo deb`'s metadata call works offline without writing
    `vendor/` or `.cargo/` into the repo.

- 2026-09-28 - **Disk usage line.** A bar of the backup disk's space under the panes (UI.md,
  "Disk usage").
  - **D-Bus: new methods instead of a changed `List`.** `ListWithUsage` and
    `NativeListWithUsage` return `((sssa(sss)as)a{st})`: `List`'s reply unchanged, plus a
    dict of byte counts. `List` and `NativeList` keep their signature, because after a .deb
    upgrade the panel keeps running the old applet until it restarts, while D-Bus starts the
    new helper; a changed `List` would fail every list there with a signature error. Adding
    methods is compatible in both directions: the new client falls back to `List` on
    `org.freedesktop.DBus.Error.UnknownMethod` (an old helper still running). The dict
    (`total`/`used`/`free` together, `reported-free` alone) means missing values need no
    sentinel numbers, and a later key doesn't change the signature. Same polkit action
    `list`, nothing new in the policy file. The interface now has 12 methods (test updated).
  - **Timeshift's mount path can't be used after the list.** Timeshift 24.01.1's `exit_app`
    calls `unmount_target_device` and `cleanup_unmount_devices` (`Main.vala`), so
    `/run/timeshift/<pid>/backup` is gone (or an empty folder, if `rmdir` failed) by the time
    `timeshift --list` returns. A `statvfs` there would describe `/run`'s tmpfs. So the helper
    looks for any live mount of the listed device instead: `stat` of
    `/dev/disk/by-uuid/<uuid>` (canonicalised; the UUID must be hex and dashes) for its
    `major:minor`, then `/proc/self/mountinfo` for a mount with that device number, or whose
    canonical source is that node (btrfs shows an anonymous `0:N`). Only mounts of the
    filesystem root count. Found: an automount under `/media`, or `/` in btrfs mode. Not
    found (a dedicated disk only Timeshift mounts): free space from Timeshift's line only.
    Mounting the device read-only just for `statvfs` was possible but left out: a mount per
    refresh for a bar, and LUKS disks (which Timeshift unlocks itself) wouldn't work anyway.
  - The free line: `_("%d snapshots, %s free")` with `format_file_size` (`SnapshotRepo.vala`,
    `TeeJee.FileSystem.vala`): decimal units (`B`, `KB`, `MB`, `GB`, `TB`), one decimal,
    `%'` grouping (none in the C locale both the helper and pkexec use). Only printed with
    at least one snapshot. Parsed before the table only; `SnapshotList::reported_free`.
  - Numbers as `df` shows them: `total = f_blocks * f_frsize`, `used = (f_blocks - f_bfree) *
    f_frsize`, `free = f_bavail * f_frsize`; the bar and the thresholds use
    `used / (used + free)`, so ext4's reserved blocks count as neither (as `df`'s `Use%`).
    Sizes on screen are binary (like `lsblk` and the settings view), so Timeshift's
    `123.4 GB` shows as `115G`.
  - Bar width: iced has no "fill with characters" text, so a `responsive` widget gets the
    width and the cell count is `width / 8.4` (monotext is 14 px; monospace fonts are about
    0.6 em). A headless tiny-skia layout (`disk_line_fits_...`, opt-in with
    `APSIS_LAYOUT_TEST=1` like the settings one) measured the installed mono font at exactly
    8.4 px: 31 cells in 266.8 px (popup), 22 in 186.8 px (640 px window). A wider font would
    only clip the bar's end (the container clips).
  - The line is hidden in the settings view: with it, the opt-in settings layout test failed
    at 640 x 440 (details need 248 px, got 224). Hiding it was simpler than shortening the
    notes again.
  - Refresh after a real restore waits until the browser is closed and no `Browse` call is
    in flight (`list_if_restored`, a count of open browse calls): both take the helper's
    single-operation lock, and a list refused as busy would replace the snapshot list with
    an error. Create/delete/bulk delete already listed afterwards.
  - Builds here again used the unpacked `vendor.tar` and a scratch `CARGO_HOME`, with
    `CARGO_NET_OFFLINE=true` in the environment (an `--offline` after `--` went to clippy).

- 2026-09-28 - **Full disk bar for Timeshift, progress and ETA, `just deb-install`.**
  - The user tests from the .deb only: `sudo just install` would overwrite files apt manages.
    `just deb-install` runs `just deb`, `sudo apt install --reinstall` on
    `target/debian/apsis_<version>-1_amd64.deb` (version from the root `Cargo.toml`'s first
    `version`, as `just tag` writes it; absolute path, so apt reads a file, not a package
    name), stops `apsis-helper.service`, and says to re-add the applet. CLAUDE.md says so.
  - **Disk usage, changed from the entry above:** when no mount of the backup disk exists
    after `timeshift --list`, `ListWithUsage` now mounts it `ro,nosuid,nodev,noexec` at
    `/run/apsis/backup` (`native::mount_listed`: lsblk must show the listed UUID with a Linux
    filesystem, then the same `mount()` as browse), `statvfs`, unmount (the guard). Under the
    single-operation lock, like every other use of that mount point. Only `ListWithUsage`
    does it; the old `List` never mounts. Timeshift's free line only if that fails (LUKS:
    lsblk shows `crypto_LUKS`, refused before `mount`). The journal line says which source:
    `disk usage: statvfs of /media/... (already mounted)`, `... of a brief read-only mount at
    /run/apsis/backup`, or `... failed (...): Timeshift's free line only`.
  - **Progress, Timeshift:** checked in `.scratch/timeshift` at 24.01.1 (`git describe`):
    `Main.vala` prints `"%6.2f%% complete (%s remaining)\r"` once a second while rsync runs;
    the time is `format_duration`, `hh:mm:ss`, or `???` before any progress. Only `\r`, so the
    helper's runner splits stdout on `\r` and `\n` while reading it. The fixture
    `create-rsync-progress.txt` is rebuilt from the format strings and the log lines around
    them (all checked against the source; a GUI-only `Syncing files with rsync...` was taken
    out again), not captured: a real create needs root.
  - **Progress, rsync `--info=progress2`:** captured locally with rsync 3.2.7 (no root, files
    in the scratchpad, `--bwlimit` to make it slow): fixture `rsync-progress2.txt`, only byte
    counts, rates and times. The time column is the time **left** on plain lines but the time
    **taken** on lines that end a file (`(xfr#N, to-chk=A/B)`: it counted up 0:00:02, 0:00:03
    ... while plain lines between counted down). Those lines are most of them with many small
    files, so they're not skipped: the time left is worked out from elapsed and percent
    there. With incremental recursion rsync's percent is of what it knows so far, so it can
    jump; `--no-inc-recursive` would fix that but keeps the whole file list in memory, so it
    stays off.
  - `--info=progress2` goes into the native create's rsync (the log file doesn't get it; the
    exact-argv tests changed) and into real restores only (dry runs keep their argv; they
    copy nothing). The restore runner takes the progress pieces out of stdout, so the
    itemized plan reads as before (a test checks no `%` line becomes a note).
  - `Runner::run_streaming` has a default that doesn't stream (pkexec, test fakes); the
    helper's `DirectRunner`, `QuietRunner` and `RsyncRunner` stream. A piece the callback
    takes (a progress line) isn't kept in stdout, so `failure_output` never shows a progress
    line as Timeshift's last words.
  - **Signal:** `Progress(s op, d percent, x eta_seconds, s text)`, `-1` for unknown (D-Bus
    has no optional). `ProgressSink` throttles to one per 500 ms (the `100%` one always gets
    through) and hands updates over an unbounded channel to a task that emits them, so the
    blocking work never waits on the bus. That task is awaited before `Finished`, so no
    progress arrives after it. The helper sends one number-less update when a create or real
    restore starts: that's how the applet tells "progress will come" (show `estimating…`)
    from "no progress from this helper" (pkexec, or an old helper that doesn't know the
    signal: a bare spinner, no error). Subscribing to a signal an old helper never sends is
    harmless.
  - Applet: `with_progress` runs the operation's future with a channel sender and turns the
    receiver plus the final message into one `cosmic::task::stream`; a `Progress` message
    after the operation ended is dropped. The bar is 20 cells, fixed (the activity pane is
    always wide enough; the disk bar is the one that adapts).

- 2026-09-28 - **Phase 5.2 design (native schedule and retention): written, not built.** The
  design is in PLAN.md, Phase 5.2, with the Timeshift 24.01.1 file/line references. Decided
  there (pending the user's OK):
  - The switch between schedulers is `timeshift.json`'s `schedule_*` flags, not the cron
    files: every Timeshift run, even `--list`, rewrites `/etc/cron.d/timeshift-hourly` and
    `-boot` from them on exit (`Main.vala:4323` -> `cron_job_update`, `:4208-4259`).
  - Apsis owns the schedule only while its own record says so *and* those flags are all off;
    the job checks both each run, so Timeshift turning its schedule back on pauses Apsis
    instead of doubling up. Switch steps are ordered so a crash between any two leaves one
    scheduler.
  - Timer + oneshot service run `apsis-helper --scheduled` directly (not over D-Bus), with a
    new `flock` shared with the D-Bus helper for the single-operation lock.
  - Not enabled by the .deb; `prerm remove` gives the schedule back to Timeshift first.
  - No new polkit action (`configure` covers moving the schedule).
  - Scheduled snapshots copy Timeshift's "tag a snapshot from the last hour instead of taking
    a new one" step, which is how multi-tag snapshots arise.

- 2026-09-28 - **Phase 5.2 rescoped: no native scheduler.** The user dropped the design above
  (timer, Timeshift-flag switching): users mostly snapshot by hand, and Timeshift already
  schedules and applies retention to scheduled snapshots. Scheduled snapshots stay with
  Timeshift's cron for both backends. Built instead:
  - **Keep last N manual** (`retention::manual`, pure). Counted like one Timeshift level: the
    newest N `O` snapshots stay; commented ones count and stay. Differences from Timeshift,
    on purpose: only snapshots whose *only* tag is `O` are deleted (the `timeshift` CLI has no
    way to take one tag off, and scheduled tags are Timeshift's retention's business, so both
    backends behave the same); tagless snapshots aren't touched (they aren't "manual"); the
    newest snapshot is never deleted (a guard; with N >= 1 it can't be reached anyway). It
    uses the list as shown, which covers every system's snapshots on the device (the
    Timeshift list has no system field).
  - Never automatic: `p`, or by itself after a successful create once the new list is in,
    and then only when something is past N. The question is its own prompt so Esc closes the
    preview too; `y` runs the existing bulk delete (helper `Delete` per snapshot, which checks
    each name against a fresh list).
  - **Reminder**: due when a list showed a device and the newest snapshot is more than N days
    old or there is none. The icon is the same symbolic icon drawn with the theme's
    `warning_text_color` through `applet.button_from_element` (what `icon_button_from_handle`
    does, with another colour). To know without opening the popup, the applet lists at
    start and every 6 h while the popup is closed, **only through the helper**
    (`background_list`: no helper, nothing runs; pkexec is never started without the user).
    The startup list marks `loading`, so opening the popup meanwhile doesn't start a second
    list that the helper's lock would refuse.
  - Settings: `BackendChoice` became `ApsisChoice` (it now holds all of Apsis's own settings:
    backend, dry run, keep manual, remind days); cosmic-config fields `keep_manual` and
    `remind_days` (missing ones read as the defaults, no version bump). The count prompt now
    takes a `Counted` (a schedule level, keep manual or remind) instead of a level.
  - **Low priority**: `QuietRunner::low_priority` runs rsync under `ionice -c 3 nice -n K`,
    both found on the fixed PATH, rsync by full path. `nice -n` adds to the current niceness,
    so K = 19 minus the current one (the test process here starts at -3 and first got 16).
    Set before exec, so the processes rsync forks inherit it; setting it on the child after
    spawn would miss those. rustix has no `ioprio_set`, hence `ionice`. Tested by running
    `sh -c 'nice; ionice -p $$'` through the runner (`19`, `idle`).
  - The ext4 image wasn't mounted in this session (mounting needs root): the native and
    restore suites ran on plain temp dirs only; `just test-ext4` is for the user.

- 2026-09-29 - **Keep last N manual: commented snapshots are pinned and not counted.** The
  user's change: N now counts only uncommented on-demand snapshots; a commented one always
  stays and no longer takes one of the N slots (before, it counted like Timeshift counts
  commented ones in a level). An uncommented `O` snapshot that also has another tag still
  counts (it's uncommented on-demand) but is still left to Timeshift if past N.
  - Sandbox note: cargo inside Claude's sandbox failed to open the libcosmic git cache
    ("failed to remove ... Read-only file system"), because libgit2 couldn't read the denied
    `~/.gitconfig`. Running cargo with a temporary `HOME` (and `CARGO_HOME`/`RUSTUP_HOME`
    pointing at the real ones, read-only) works offline.
  - `just test-ext4` on the user's machine, image mounted: native 14/14, restore 11/11. A
    first run with the image not mounted failed 12 native tests at the "is an ext4 loop
    mount" guard, as it should (nothing was written to the plain disk).
  - Seen once: `native::runner::tests::normal_priority_is_left_alone` failed in a full
    workspace run right after a heavy clippy build (a normal-priority child reported
    niceness 19); it passed 30+ times alone and in two more full runs. Guess, not verified:
    the OS scheduler reniced the test process. Left as is for now.

- 2026-09-29 - **New direction: 0.2.0 standalone, manual only** (the user's). Apsis drops
  Timeshift: the native rsync backend is the only backend, nothing runs `timeshift`, no
  schedule, no automatic retention. The Phase 5.2 native schedule is parked on branch
  `phase-5.2-schedule` (`a2a0629`, not pushed, not merged); main stays at `9295449` (keep last
  N, reminder and low-priority rsync from `90272f5` fit the new plan). Design in PLAN.md,
  "0.2.0 Standalone", approved with decisions 1-5 as recommended, plus the user's delete
  conditions: a snapshot name only, resolved under `timeshift/snapshots/` with no symlink on
  the way, with an `info.json`, never `snapshots/` itself; no-follow, same-filesystem removal;
  its tag links removed and nothing else; read-write mount only for the delete, under the lock,
  one journal line per snapshot. `config.toml` written atomically, 0644, `.bak` kept, and an
  encrypted or non-plain device refused as before; a first run without `timeshift.json` starts
  empty.
- 2026-09-29 - **0.2.0 standalone: built** (not committed; nothing run as root by Claude).
  - **Filters stay one ordered list** in `config.toml`, home patterns included (the design had
    a `[home]` table): rsync takes the first matching filter, so `*.mp3` before
    `+ /home/you/**` leaves MP3s out of a home that is otherwise kept, and after it keeps them.
    A table would lose that order. Test: an imported config builds exactly the rsync filters
    Timeshift's list does.
  - **Delete refuses a snapshot with a mount inside** (`/proc/self/mountinfo`, any mount point
    at or below its canonical path), before deleting anything. The no-follow walk
    (`prune::remove_at`, from the parked branch) stops at another filesystem, but a bind mount
    of a folder on the same filesystem has the same `st_dev`, so the walk alone would empty it.
    The walk opens `timeshift/`, `snapshots/` and `<name>/` with `openat(O_NOFOLLOW)` from the
    backup mount, so no symlink on the way is followed. `just nested-mount` prepares a real
    bind mount and a loop mount (the ext4 image) inside a snapshot for the opt-in test.
  - **Delete leaves the other tag links alone**: only `snapshots-<tag>/<name>` symlinks go
    (as Timeshift's delete); create still rebuilds all links, as Timeshift's create does.
  - **Removed**: `TimeshiftCli`, the `--list` parser, `PkexecRunner` and the pkexec fallback,
    Timeshift's lock check, the `timeshift.json` editor, Timeshift's progress and free-space
    parsers, the dry-run setting and `NativeDryRun`, the schedule, mode and backend rows. Helper
    methods: `NativeListWithUsage`, `NativeCreate`, `Delete` (native now), `ReadConfig`,
    `WriteConfig`, `Browse`, `Restore`; the seven polkit actions are unchanged.
  - **Keep last N** ignores tags now (Timeshift's schedule tags counted like any snapshot's).
  - Without the helper the applet shows `Apsis needs apsis-helper, which the .deb installs`.
  - `timeshift` is no longer recommended by the .deb; purge removes `/etc/apsis/`.
  - The `toml` crate (0.5.11) was already in `Cargo.lock`; no download.

- 2026-09-29 - **0.2.0 tested and prepared for release.** The user's 8 real-machine checks
  passed (old schedule unit removed, `just deb-install`, the import shown and saved with `w`,
  list/create/delete/bulk delete with Timeshift not running, rsync at nice 19 and idle I/O, tag
  links, a real bind and loop mount inside a snapshot left alone, keep-last-N, restore, disk
  unplugged). Release prep: version 0.2.0, README, man page, UI.md, SECURITY.md (0.2.x
  supported), ARCHITECTURE.md, metainfo and CHANGELOG. TIMESHIFT-CLI.md is kept, marked
  historical, because CLAUDE.md lists it.
  - **New tagline**: "Simple system snapshots and file restore for the COSMIC™ desktop" in the
    README, metainfo summary, desktop entries, app comment and the collection drafts in
    RELEASE.md, since the old one said "Timeshift-style". `CLAUDE.md` still has the old listing
    line; it's the user's file, so left for the user.
  - The CHANGELOG's 0.2.0 section also covers `36c094f` and `90272f5` (disk bar, progress,
    keep last N, reminder, low-priority rsync), which were never in a release.
  - **`normal_priority_is_left_alone` fixed** (the failure noted on 2026-09-29 came back once,
    again right after clippy): it asserted the child isn't at nice 19, but the test process
    itself can be reniced from outside. Nothing in Apsis sets a priority in-process (checked).
    It now compares the child's niceness with the spawning thread's own, retried if that
    changes meanwhile. Two full runs pass after it.

- 2026-09-29 - **UI polish, phase 1: status model.** `apsis_core::status` adds `ApsisStatus`
  (last, remind, disk), `Due`, `DiskStatus`, `Severity`; `apsis::status::StatusView` is the one
  formatter for the panel label, tooltip and icon severity (the strip and popup will reuse it).
  Inputs are the existing `remind_days` setting and the `SnapshotList` (snapshots, device,
  `usage`); no new helper call. Disk thresholds (`DISK_LOW` 10%, `DISK_CRITICAL` 5%) and
  `size`/`size_short` moved from `apsis` into core; the bar's `Space` uses the same constants.
  Icon: warning for a due reminder or free < 10%, destructive for free < 5% (the worse wins);
  colour on the icon only. New setting `show_label` (off by default, settings row `panel
  label`, horizontal panels only) shows `12h · 62%` beside the icon.
  Conflicts with `docs/APSIS-UI-PROMPT.md` (the contract is untouched):
  - Section 12 has `next: Option<NextSnapshot>`. The scheduler was dropped (5.2), so there is
    no `next`; the tooltip has no `next` line. The task's `remind` and `Due` are not in the
    contract's struct; added.
  - `DiskStatus::Mounted { device, used, total }` became `{ device, usage: DiskUsage }`, so the
    percent and the warning limits match the disk bar and `df` (share of usable space, `used +
    free`, not `total`, which counts blocks kept back for root). `used_pct` rounds up like `df`.
  - The contract writes an unknown side as an em-dash; the global rule bans them, so it is `-`
    (`12h · -`, `- · 62%`, `- · -`).
  - The tooltip changed from `Apsis - last snapshot 3h ago · 483G free` to the contract's
    lowercase `last` / `disk` lines. Low space reads `disk  28G free  (under 10%)`.
  - With no backup device set the reminder stays off and the disk reads `not connected`
    (today's behaviour, since there is nothing to judge by). A failed list gives
    `Apsis` + `disk  not connected`.
  - `PLAN.md` has no UI polish phase yet; this slice is logged here only.
  - Build note: in this session cargo could not update its git cache (`~/.cargo` is read-only
    and `~/.gitconfig` is blocked in the sandbox), so tests and clippy ran with a scratch
    `CARGO_HOME` and `HOME` under the session's scratchpad. Nothing in the repo changed for it.

- 2026-09-29 - **Direction: the panel is an overview, the window does the work.** From the
  user's mockups (`panel.jpg`, `app.jpg`, in their Downloads, not in the repo). Decided:
  - The panel popup is **read-only**: time (last, next), disk bar, recent snapshots, and a way
    to open the window (`o`). No create, delete, restore or settings there. This changes
    `APSIS-UI-PROMPT.md` section 5 (popup with list, activity and key hints); the contract is
    not edited, this entry overrides it.
  - The window (`apsis --window`, Layout D of the contract) is where create, delete, browse and
    restore happen: apsis strip on top, snapshots and details, activity, four rooms.
  - `next` is `manual only` everywhere for now (tooltip line added). Scheduling stays on the
    roadmap; when it returns, `next` and the mockups' last/next timeline can follow.
  - Mockup differences kept out: per-row cyan dots and `idle` labels (the contract forbids status
    dots), the timeline slider (needs `next`), the literal cyan (live COSMIC accent instead) and
    uppercase `TIME` / `DISK` (lowercase pane titles).
  - Open: whether the popup can show a running job depends on the helper exposing its state;
    not checked yet.

- 2026-09-29 - **UI polish, phase 2: the apsis strip in the window.** `StatusView::strip()`
  gives the strip's text (`Strip`, `DiskStrip`); `AppModel::strip()` lays it out as an `apsis`
  pane above the split, `--window` only and not in the settings view. The window drops its
  old disk line (the popup keeps it until slice 4). Reuses `pane()`, `disk_bar()` and
  `Space`; colours are theme roles only (accent labels, warning and destructive phrases).
  - The pane is drawn as an inactive one (dim title), like details; the mockups' all-accent
    panes are a chrome change for the window-chrome slice, not done here.
  - The mockup's `sda1` line puts the device beside the sizes; device is accent, sizes text.
  - The layout test that checked the disk line in both modes is now popup-only; a new one
    checks the strip against the smallest window.

- 2026-09-29 - **UI polish, phase 3: the activity line.** Most of it existed (`36c094f`:
  helper `Progress`, percent, time left). Now: `creating snapshot · 58% · 3m 12s left` and a
  bar that fills the rest of the line; `creating snapshot · working · 1m 08s elapsed ⠹` while
  there is no number (elapsed from `run_started`, set in `run()`, cleared when the job ends);
  `restoring · ...` for a real restore. `fmt::eta` (`~3 min`) became `fmt::duration`
  (`3m 12s`, `42s`, `1h 02m`), used for both time left and elapsed. The disk bar and the
  progress bar are one widget, `bar(fraction, class)`; `disk_bar` picks the colour by `Space`.
  The line is shared, so the popup's activity pane changed wording too (its layout did not).
  Deviations from the contract, section 8:
  - No `2.1G / 3.2G`: rsync's progress line has bytes copied but no reliable total.
  - Failure stays `create failed: <reason>`, not `create failed · open log`: there is no log
    room until the rooms slice.
  - Done stays `snapshot created`, not `created <name>`: the new snapshot's name is not known
    until the refreshed list arrives, and matching it would be a guess.
  - Delete progress untouched (no percent). Disk used does not tick while busy: the strip
    stays as the last list showed it.

- 2026-09-29 - **UI polish, phase 4: the panel popup is a read-only overview.**
  `AppModel::read_only` (true for the panel applet, false for the window and for the test
  model) gates the keys and swaps the popup's content for `overview()`: `~/apsis $ status`, an
  `apsis` pane (the strip's two columns stacked, via the shared `strip_columns`), the newest 5
  snapshots as plain rows (`fmt::overview_row`, `fmt::older_count`), and hints `[o]pen apsis
  [r]efresh [esc]`. 400 px wide (`OVERVIEW_WIDTH`), 388 px high in the layout test. Only `o`,
  `r` and Esc work there; the window's state machine is untouched, so its ~120 tests still cover
  it. The old popup code paths stay because the window uses them.
  - `o` and the menu start a new process, `apsis --window` (`--settings`, `--about` from the
    menu), from `current_exe()` with a ` (deleted)` suffix cut (a package upgrade under a
    running panel), through the existing detached `spawn`. `main.rs` treats those flags as
    window mode; `init` opens the window on that view (`startup_overlay`).
  - Menu: `Open Apsis` added; `Settings…` and `About Apsis` now open the window on that view
    instead of the popup.
  - Look: same pane widget and theme roles as the window (no new colours), no per-row dots,
    `idle` labels or timeline (see the direction entry above). The pane titles stay dim, as in
    the window strip.
  - Not done: a running create is not shown in the popup (it runs in another process; the helper
    would have to expose its state: roadmap). Several windows can be opened by pressing `o`
    repeatedly; the helper's single lock still serialises root work. README and its screenshots
    still describe the old popup; left for the copy pass.
  - Sandbox note: these tests and clippy ran with the scratch `CARGO_HOME`/`HOME`, as before.

- 2026-09-29 - **UI polish, phase 5: rooms and the dock in the window.** `Room` (snapshots,
  create, schedule, log), `AppModel::room`, `1 2 3 4` (`KeyAction::Room`) and clicks
  (`Message::Room`), a dock of four accent-outlined cells with the selected row's fill on the
  active one (`dock`, `dock_cell`), and `[1-4]rooms` in the footer. Window only; hidden in the
  settings view and the browser.
  - **Create** keeps the list and swaps the details pane for a create form. The text is still
    typed on the `>` line (one input, one focus; a second `text_input` would need its own focus
    handling); the pane mirrors it. `2` = `c` plus the room.
  - **Schedule** has no scheduler behind it (dropped, roadmap). It holds what exists: `next
    manual only`, keep-last-N and remind (`ApsisChoice::stepped`, saved through `set_backend`).
    This departs from the contract (enable tags + keep N); the tags come back with a scheduler.
  - **Log** is a new in-memory list (200 lines, this session only, not written to disk): the
    status line's changes and failed lists, recorded in a wrapper around `update` (`handle` is
    the old body) rather than at ~25 call sites. The contract's "helper / rsync / timeshift
    output" is not there: the helper does not stream its output to the applet.
  - Esc: prompt, then room, then marks, then window. Rooms are ignored while a job runs.
  - The layout test caught the schedule room's notes squeezing the dock to 0 px at 640 x 440;
    the room scrolls now, and the test asserts the dock's height.
  - `create failed · open log` (phase 3 deviation) can now be done: the log room exists. Not
    done here; the copy pass.
  - Sandbox note: as before, tests and clippy ran with a scratch `CARGO_HOME` and `HOME`.

## Open

- ~~App ID~~ - resolved 2026-09-25, see above.
- ~~License~~ - resolved 2026-09-25, see above.
- ~~Icon artwork~~ - resolved 2026-09-25, see above.
- ~~Phase 4 helper vs. shipping a narrow pkexec wrapper script — decide after Phase 3.~~ - resolved
  2026-09-25: the D-Bus helper, see above.
- ~~`--snapshot-device`: device path or UUID?~~ - resolved 2026-09-25, see above.
- ~~Does `--scripted` change the `--list` format?~~ - resolved 2026-09-25, see above.
- No real fixture yet for "configured device, zero snapshots" or for multi-tag rows (`BD` etc.);
  both are covered by synthetic tests built from the real layout.
- Phase 2: check whether Timeshift prints the `--list` table on stdout or stderr, and what exit codes
  it uses on failure (Apsis treats any non-zero exit as failure). Apsis parses stdout only; if the
  popup says "unrecognised `timeshift --list` output", the table is probably on stderr.
- Phase 5: ~~a real `info.json` and `exclude.list` from one of the user's Timeshift 24.01.1
  snapshots (redacted) as a fixture, to check the hand-made one~~ - done 2026-09-26, see above.
  ~~And `timeshift --list` on a
  device with a native snapshot, to confirm Timeshift lists it (the tests only mirror
  Timeshift's reader).~~ - done 2026-09-26, see above.
- Phase 2: confirm on a real panel that the popup gets keyboard focus (keys were only reasoned
  about, not run, by Claude). ~~Check that `document-open-recent-symbolic` exists~~ - replaced by
  the Apsis symbolic icon, 2026-09-25.
- ~~Phase 5.2 (waiting for the user)~~ - moot, the scheduler was dropped 2026-09-28: (1) Timeshift's boot retention ignores comments
  (`SnapshotRepo.vala:625-634`), so a commented boot-only snapshot past `count_boot` is
  deleted; the user's rule was "commented snapshots kept". Proposed: Apsis keeps them (a
  manual `timeshift --create` would still delete them). (2) "Never delete the newest": not a
  Timeshift rule; its rules only reach the newest if it has no tags. Proposed: Apsis adds the
  guard. (3) Timeshift's "tag a recent snapshot" and its retention consider every system's
  snapshots on the device, not just this one's. Proposed: copy that exactly (it's what
  Timeshift itself will do on its next run anyway). (4) Retention after a native on-demand
  create, as Timeshift's `--create` does: proposed yes, once the schedule is Apsis's.

## 2026-09-29 - UI polish slice 6: theme audit and copy pass

- **Theme audit: clean.** No literal colours in `crates/apsis/src`. Every colour comes from
  `theme.cosmic()` (accent, divider, background, warning, destructive); the only derived ones are
  alpha fades of those (selected row 0.15, marked row 0.07, dim text 0.7).
- **Known, not fixed: translucent theme tints the active pane.** A pane is a border-coloured fill
  with the theme background fill on top. With a theme whose background has alpha below 1 (seen in
  the window with a custom RON theme), the accent shows through the whole active pane. Changing the
  theme fixed it for the user. Forcing the inner fill to alpha 1.0 would hide it but also make the
  panel's panes opaque, so it waits for the libcosmic fix (the `text_tint` issue).
- **Copy pass:** `i18n/en/apsis.ftl` needed no changes; the tagline is the deliberate 0.2.0 one.
  One string outside the ftl file is rough: the leftover-staging warning in
  `apsis-core/src/native/mod.rs` shows the raw staging path (`.../timeshift/apsis-staging`) in the
  activity pane. Left as is (tests may match it); reword when convenient.

## 2026-09-29 - The panel popup is a read-only overview (user decision)

- **Target.** The popup shows the apsis strip (last / due / disk bar), a read-only list of the
  newest few snapshots, the activity line (progress from jobs started in the window), and one
  way to open the window: Enter, or a click on the title or a row (the window opens with that
  snapshot selected). Esc closes. The right-click menu keeps Refresh, Close and "Remove or move
  applet...". Settings and About live in the window. Error and setup states (disk not connected,
  first run, create failed) show one muted line plus "open apsis".
- **Doc changes:** `APSIS-UI-PROMPT.md` sections 5 and 11 got dated update notes (the popup no
  longer has `s`, the four-cell dock or details on Enter).
- **Today's popup (checked in `app.rs`):** read-only already (`read_only` for `Mode::Applet`).
  Keys: `o` opens the window, `r` refreshes, Esc closes; everything else is ignored, Enter too.
  Buttons: `[o]pen apsis`, `[r]efresh`, `[esc]`. Rows and the title are not clickable. Nothing
  creates, deletes, marks, restores or opens settings. Right-click menu: Open Apsis, Refresh,
  Settings..., About Apsis, Remove or move applet..., Close.
- **Gaps** are listed as slices in the session report (Enter, click title, click row with
  `--select`, one-line states, activity line across processes, menu trim, About in the window).

## 2026-09-29 - 0.3.0 release prep

- **Version 0.3.0**, not 0.2.1: the popup lost create, delete and settings, and the window,
  status model and panel label are new. `v0.2.0` is already tagged. Bumped in `Cargo.toml`,
  `Cargo.lock` (three apsis crates), metainfo, `resources/deb/changelog`, the man page, `CHANGELOG.md`
  and `SECURITY.md` (0.3.x supported).
- **Staging warning reworded** in `apsis-core/src/native/mod.rs`: one plain line ("leftover from an
  interrupted snapshot, safe to delete") however many leftovers; the path goes to stderr, which
  is the helper's journal (`journalctl -u apsis-helper`). The core crate has no i18n (`fl!` is
  the applet's), so the line is a plain English string like the other list warnings.
- **Screenshots to retake** (none were invented): `docs/screenshot.png` and `docs/screenshot1.png`
  (0.1 all-in-one popup; used by README, the metainfo `<screenshot>` and the collection entries,
  which point at the `v0.1.0` copy). Wanted: the window (strip, snapshots, details, activity,
  dock), the panel popup overview, and the panel button with the label on. Then point the URLs at
  `v0.3.0`.
- **Left as is, flagged:** `CLAUDE.md` still has the old "Timeshift-style" listing line (the
  user's file); `UI.md`'s right-click menu section still lists Settings and About (they change
  with the read-only-popup slices); the window's `schedule` room name is kept although there is
  no scheduler (the pane says so).
- **Not run in the sandbox:** `cargo test`, `clippy` (cargo can't write `~/.cargo`); `cargo fmt
  --check` and gitleaks passed. The user runs the first two.

## 2026-09-29 - Popup gap slices: not planned

The user decided the read-only popup is fine as shipped in 0.3.0 (`o`, `r`, Esc; strip and
newest snapshots). The seven gap slices (Enter / title / row click open the window, `--select`,
one-line states, activity from window jobs, menu trim, About in the window) are not planned.
The update notes in `APSIS-UI-PROMPT.md` sections 5 and 11 describe that target, not the build.

## 2026-09-30 - 0.3.1: fixes from the first clean-machine test

From the user's test of the v0.3.0 .deb on an HP laptop and its helper journal. UI and packaging
text only: no helper code, D-Bus method, polkit action or root-run change.

1. **One window.** libcosmic's `single-instance` feature (already in `Cargo.lock`: `ron` and
   zbus's blocking API were there, so no new crate and no lock change besides the versions).
   `run_window` uses `cosmic::app::run_single_instance`; the window owns
   `io.github.atraxsrc.Apsis` on the session bus, and a second start calls its
   `org.freedesktop.DbusActivation` and exits. `Flags { mode, view }` replaces `Mode` as the
   app's flags so `--settings`/`--about` travel as the activation action (`StartView`). The open
   window switches view only when nothing is in progress (no job, prompt, browser, plan, prune
   preview or settings edit); otherwise it only comes forward. The panel process never claims
   the name (it isn't run single-instance).
   - Focus on Wayland needs an activation token. The launcher passes one; for the panel's
     `open apsis` the applet now requests one (libcosmic's `applet-token`, already enabled)
     and starts the window with `XDG_ACTIVATION_TOKEN`; without the token channel it starts at
     once, as before. libcosmic then activates the open window with the forwarded token.
   - Not verified here (no COSMIC session in the sandbox): that COSMIC actually raises the
     window on the second start. On the HP.
2. **Home line in the create room.** Asked the user whether "first-run asks whether to include
   home files" meant a new first-run question; no answer in the session, so built the
   recommended reading: no new question, only the muted `home  included` / `hidden files only`
   / `excluded` line (a third word for the third mode). Read from `ReadConfig` (polkit `list`,
   no password, no helper lock) when the create room opens, matched to the user by `$HOME`
   (`ConfigInfo::home_state_of`, `HomeState::summary`). **Open:** if a first-run question is
   wanted, it's a separate slice (it would write through the existing `WriteConfig`, so a
   configure password before the create password).
3. **Excluded home in the browser.** `Listing::is_excluded_home`: an empty `/root`,
   `/home/<name>` or config-listed home. The snapshot's own `exclude.list` sits outside
   `localhost/` and `Browse` can't read it, so this is inferred; a really empty home reads the
   same. Exact answer: **0.4.0** (the helper would have to expose the snapshot's filters).
4. **Folder restore place.** `Restored::from_result` reads the `to <folder>` line of the
   helper's existing result text (the real folder, `-2` suffix included) plus the requested
   paths. Key `o`: free in the restore result view (popup `o` is "open apsis", but the popup
   never restores; `[o]riginal` is prompt text, not a command). `KeyAction::OpenWindow` became
   `KeyAction::Open` for both uses. `xdg-open` runs as the user through the existing detached
   spawn.
5. **Disk presence.** `status::disk_connected` looks at `/dev/disk/by-uuid/<uuid>` (no root,
   no mount) every 5 s and on popup/window open. Gone: `ApsisStatus::disk_gone` (disk
   `not connected`, time side kept, §7 "time and disk fail independently"), `c` waits. Back:
   one list, only on the gone-to-back change, so a disk the helper refuses isn't mounted every
   5 s. Also a list that failed with `DeviceNotFound` is watched by that UUID, so plugging in
   lists without `r`.
   - **Icon colour:** asked the user (the contract names only stale-snap and low-free); no
     answer in the session, so the recommended option: the icon keeps the theme colour when
     the disk is missing. **Open** for the user to reverse.
6. **Busy.** `CliError::Busy` (was folded into `Other`). A busy list no longer becomes a failed
   list (which made the tooltip say `disk  not connected` and the window look idle); the
   listing stays as it was, the activity says `busy · a job is running in the background`,
   and while the popup/window is open the list is retried every 3 s until it goes through,
   then back to idle. Create, delete and restore answered busy say the same and start the
   retry. The job's progress can't be shown in a window that didn't start it: **0.4.0** (the
   helper would need a running-job query or broadcast signal).
7. **Create failed, disk removed.** `CreateFailure::of`: the disk's link left
   `/dev/disk/by-uuid` and the helper didn't report `DeviceNotFound` -> `create failed ·
   backup disk removed`; the helper's own reason goes to the log room (`push_log`). A disk
   missing when the create started keeps its existing `plug it in and press r` line (an
   existing test caught that). A distinct error kind from the helper would make this exact:
   **0.4.0**.
8. **Unit description** `Apsis snapshot helper`; the unit's comment no longer mentions
   Timeshift or btrfs.

- Tests: core (home mode lookup and words, excluded home, restore place, disk presence on a
  temp by-uuid folder, `disk_gone`, `CreateFailure`); app (activation, token env, create room
  line, restore place and `o`, unplugged/replugged disk, busy retry, disk-removed copy). The app
  tests use a by-uuid stand-in under `target/tmp/`, since the fixtures' UUID is on no machine.
  The room layout test (`APSIS_LAYOUT_TEST=1`) still fits at 640 x 440 with the home line.
- Sandbox note: cargo ran offline with a scratch `HOME` (the real `CARGO_HOME`/`RUSTUP_HOME`),
  as before.

## 2026-09-30 - Product decision (owner): Apsis does four things

The owner decided what Apsis is. It does exactly this, nothing else:

1. snapshot the system, with two include choices, `/root` and `/home` (either, both or
   neither); the system is always included;
2. a simple include/exclude filter list (like Timeshift's Filters tab);
3. delete snapshots;
4. restore the whole system (0.5.0, PLAN Phase 6b).

Consequences, all planned for 0.4.0 (design in `docs/PLAN.md` "0.4.0", waiting for the
owner's approval; no code yet):

- **File-level restore is removed** (Phase 6a, shipped 0.1.x to 0.3.x): the browser, folder and
  original modes, `.apsis-before-<snapshot>` backups, `~/Apsis-restored`, the `Browse` and
  `Restore` helper methods, the polkit actions `browse`, `restore` and `restore-original`, and
  the 0.3.1 additions for it (the browser's "home not included" line, the `[o]pen folder` key).
  Files users already restored stay where they are; Apsis doesn't touch them.
- **Per-user home modes are removed** (excluded / hidden files only / everything, Timeshift's
  Users tab): two booleans, `/root` and `/home`, plus the filter list. Old configs convert
  without changing what a snapshot holds (one documented exception; see PLAN); "hidden files
  only" becomes a visible `+ <home>/.**` filter row.
- New in 0.4.0 around those four: job status from the helper (any window or the popup sees a
  running job), stop for a running create, cleanup of interrupted creates' staging folders, a
  real "backup disk removed" error, the duplicate `--delete-excluded` removed.
- 0.3.1's notes that pointed at 0.4.0 (exact excluded-home answer in the browser, job progress
  in a window that didn't start it, a distinct disk-removed error) are answered by this:
  the first is moot (no browser), the other two are D and G of the design.
- **`docs/APSIS-UI-PROMPT.md`**: dated update notes (not a rewrite, per its section 17) in
  section 1 (file restore and home modes gone), section 3 (the one settings screen, text
  toggles, `+`/`-` rows), section 8 (stop is now in PLAN), and sections 9 and 11
  (double-click no longer browses).
- Open for the owner: the seven decisions at the end of PLAN "0.4.0" (default includes,
  leftovers, stop authorisation, interface version, keep-N/reminder/label, filter order keys,
  tagline).

## 2026-09-30 - UI direction (owner): Timeshift's UI, standard widgets

Overrides the earlier UI decisions (terminal look, read-only-popup details, rooms and dock,
the 2026-09-29 contract).

- **Direction:** take Timeshift's UI and ease of use over to Apsis; start simple and add
  features only when users ask. Main window: a toolbar of labelled buttons (Create, Restore,
  Delete, Settings), the snapshot list (date, comment), a status area at the bottom (last
  snapshot, backup disk used/free; a progress bar and Stop while a job runs). Settings as
  Timeshift's tabs: Location, Include (`/root`, `/home`), Filters (`+`/`-` list with Add
  Folder, Add File, Add Pattern, Remove, Move Up, Move Down). Standard libcosmic widgets and
  theme colours only; no terminal styling, key-hint footer, dock/rooms or prompt line.
  Keyboard shortcuts may stay, documented only in the man page and README. Kept: the panel
  applet (read-only popup and tooltip) and the single window.
- **`docs/APSIS-UI-PROMPT.md` replaced** by a short contract describing this. The old one (and
  its dated notes from earlier today) stays in git history.
- **Answers to the 0.4.0 design questions:**
  1. Fresh installs: `/root` on, `/home` off. The 0.5.0 restore confirm must say whether home
     folders are rolled back or untouched (added to PLAN Phase 6b).
  2. Leftovers of interrupted creates: removed automatically at the next create, and shown as
     a dimmed row whose Delete removes it through the normal `Delete` path.
  3. Stop: the uid that started the job may stop it without a prompt; anyone else needs
     `auth_admin_keep` (new `stop` action).
  4. Helper interface `Helper2` with clean method names.
  5. Keep-last-N and its pruning are removed; the reminder and the optional panel label stay.
  6. Filters: Move Up / Move Down buttons, new rows at the top.
  7. No tagline; the app is "Apsis"; "System snapshot and restore" where a summary is
     required (AppStream, Debian); "Timeshift-style" removed everywhere.
- **Plan:** PLAN.md "0.4.0" updated with these answers and mockups of the main window and
  settings. Build order after the owner's OK: removals, then the helper (Helper2, Stop, Job
  status, leftovers, DeviceRemoved), then the UI; tests, clippy and fmt after each slice; no
  commits. Three small questions left in PLAN (where the reminder and label are set, the
  Restore button before 0.5.0, the popup's look).
- Same day, the owner's answers to the three small questions, all as recommended: a fourth
  settings tab **Misc** for the reminder and panel label; no Restore button until 0.5.0; the
  popup keeps its content in standard libcosmic widgets. Slice 1 (removals) started.

## 2026-09-30 - 0.4.0 slice 1: removals

Built, not committed. `cargo test --workspace`, `clippy --all-targets -D warnings`, `cargo fmt`
and the opt-in layout tests (`APSIS_LAYOUT_TEST=1`) pass; `gitleaks dir` finds nothing.

- **H:** the second `--delete-excluded` is gone from the rsync argv (tests updated).
- **A, file-level restore:** `apsis-core/src/restore/` and `tests/restore.rs`,
  `Error::Restore`, the browse/restore wire types and client calls, the `Browse`/`Restore`
  names and three polkit action names; `Progress::part_of`. Helper: `restore.rs`, the two
  methods, `unix_user`, `native::mount_backup`; the policy file has four actions and the bus
  policy comment lists them. Applet: `browser.rs`, the browser, plan and result views, `R`,
  `[o]pen folder`, the restoring progress label. `just test-ext4` runs `--test native` only.
  `unix_user` comes back in slice 2 for Stop (the owner's per-uid rule).
- **Keep-last-N:** `apsis_core::retention`, `p`, the prune preview and its automatic offer
  after a create, the settings row, `Counted::KeepManual`, `ApsisChoice::stepped` and `Step`;
  `keep_manual` is no longer read from cosmic-config (left in old configs, harmless).
- **Window chrome:** rooms, dock, schedule and log rooms (and the log itself), the help
  overlay and `?`. Enter and Tab make the details pane active (Enter used to open the
  browser); a double-click on a row does nothing until the new UI. The digit, `h`, `l`,
  Backspace, `R` and `p` keys are gone.
- **0.3.1 leftovers that only served removed things:** the create room's home line
  (`HomeState::summary`, `ConfigInfo::home_state_of` and their test), the browser's
  "home not included" line, the disk-removed reason that went to the log room (the status line
  still says `create failed · backup disk removed`; the helper's real error is slice 2).
- **i18n:** the 95 strings nothing used any more removed; `browse-marked` renamed `marked`.
- Left on purpose for later slices: the settings view's per-user home rows (config v2 in slice
  2), the terminal look, the apsis strip, details pane, prompt line and footer (slice 3).
- Process note: once in this slice `git rm --cached` touched the index; it was undone at once
  with `git reset -- <paths>` (unstage only), and the files were deleted with plain `rm`. The
  index is empty again.

## 2026-09-30 - 0.4.0 slice 2: the helper (Helper2, config v2, stop, jobs, leftovers)

Built, not committed. Tests (applet 121, core 74, config 16, native 27, helper 30, layout),
`clippy --all-targets -D warnings` and `cargo fmt` pass.

- **Config v2** (`apsis_core::config`): `include_root`, `include_home`, signed filters.
  `Config::read` gives `Stored::Current` or `Stored::Legacy` (version 1); `convert` does the
  PLAN rules; `effective` converts a v1 file or a Timeshift import and adds notes. Fresh
  config: `/root` on, `/home` off. `ConfigInfo` lost `users` (`notes` replaces `imported`,
  `saved_device()` replaces `saved()`). `validate` takes the saved device, not a whole config.
- **Proved on real rsync:** a test runs `rsync --dry-run` on a temp tree with the old 0.3 list
  (from the real redacted settings) and with the converted v2 list; both copy exactly the same
  files.
- **Filter builder** (`exclude::for_backup(filters, include_root, include_home, fstab,
  users)`): the PLAN order; parent `+ dir/` lines before a `+` filter (none for the folder a
  trailing `/***` already covers, none past a `**`); no per-user home step.
- **Stop:** `native::Cancel` (armed / stopping / committed), rsync in its own process group
  (`process_group(0)`), `SIGTERM` then `SIGKILL` after 10 s from a thread, `waitid(WNOWAIT)`
  before reaping so a signal never reaches a reused pid. Tested with real processes: a group
  with a child dies on `SIGTERM`; a script that ignores `SIGTERM` dies on `SIGKILL` after the
  grace period; a stop before rsync copies nothing. Deviation: the journal says
  `stopping, SIGTERM to rsync's process group (SIGKILL after 10 s)` at the start; whether
  `SIGKILL` was needed isn't logged separately (the `Cancel` has no logger).
- **Staging removal** (`remove_staging`) uses the delete's rules; every failed create now uses
  it too (was `remove_dir_all`). Leftovers: `SnapshotList::leftovers`, removed (logged) at the
  start of every create; anything in `apsis-staging/` that isn't a snapshot-named folder is a
  warning and never removed. `Delete(name)` removes a leftover through the same rules when
  `name` is one and no snapshot; everything else takes the snapshot delete with all its
  refusals (a symlinked `timeshift/` still reads as such).
- **Helper2:** `List`, `Create`, `Delete`, `Stop`, `Job`, `ReadConfig`, `WriteConfig`;
  `JobChanged` (broadcast) and `Finished`. `Progress` is gone: the client's
  `create_with_progress` reads the create's `JobChanged` instead, so the current applet's
  progress line works unchanged. `JobChanged` also goes out for `list` and `configure` jobs
  (the UI must only re-list after a create or delete ends, or when it was waiting on busy, to
  avoid two windows listing each other forever). New client calls: `job()` (asks
  `NameHasOwner` first, so it never starts the helper), `stop()`, `job_changes()`.
- **Stop authorisation:** the starter's uid (`GetConnectionUnixUser`, kept with the job) stops
  without a prompt; any other uid needs `io.github.atraxsrc.Apsis.stop` (`auth_admin_keep`).
  The target is checked again after the password dialog. polkit now has five actions; the bus
  policy names `Helper2` and allows the broadcast signal.
- **DeviceRemoved:** after a failed create or delete the helper looks at
  `/dev/disk/by-uuid/<uuid>`; gone means `Error::DeviceRemoved { device, reason }`
  (`backup disk removed: <uuid>: <reason>` on the bus). The applet's 0.3.1 guess
  (`CreateFailure`) is removed. A failed unmount tries `umount --lazy`.
- **Applet on Helper2, old look kept:** settings rows are now device, `[x] /root`,
  `[x] /home`, filters (space or `+`/`-` sets the sign, typed filters without a sign are
  excludes, new ones go on top), remind, panel label. `create stopped`, `delete failed · backup
  disk removed` added. The file-chooser, tab and Create-dialog helpers in `settings_view.rs`
  have a module-level `allow(dead_code)` until slice 3 uses them.
- Removed: `HomeState::next`, `User::set_home_state`, `User::owns` (only reading home states
  is left, for the converter); the Timeshift-builder `exclude.list` test (the rsync equivalence
  test replaces it).
- Needs the HP (root): everything in PLAN "J" items 1-8.

## 2026-09-30 - 0.4.0 slices 3 and 4: the Timeshift-style UI, docs, version

Built, not committed. `cargo test --workspace` (applet 68, core 74, config 16, native 27,
helper 30), `clippy --all-targets -D warnings`, `cargo fmt --check`, the layout tests
(`APSIS_LAYOUT_TEST=1`) and `groff -ww` on the man page pass. `appstreamcli` and
`desktop-file-validate` only report what they reported before (the `COSMIC` category, the app
ID's capitals).

- **The applet was rewritten** (`app.rs` state and update, `app/view.rs` drawing,
  `app/tests.rs`), about 2,100 lines in place of about 5,000. Carried over: the panel button,
  tooltip, label and icon colours, the right-click menu, starting the window with an activation
  token, single instance, disk presence, the background list for the reminder. Everything
  drawn is a standard libcosmic widget: buttons with symbolic icons, `list_column` rows,
  `dialog`, `tab_bar`, `settings::item` (radio, checkbox, toggler), `spin_button`, linear
  progress bars, `about`. Colours are theme roles only.
- **libcosmic features added:** `about` (the About page) and `xdg-portal` (the file chooser for
  Add Folder / Add File). Both build offline; `Cargo.lock` didn't change except the three apsis
  versions.
- **Deviations from the mockups in PLAN:**
  - The header's "menu (☰)" is two icon buttons, Refresh and About, with tooltips: simpler,
    and a menu for two items didn't earn its place.
  - Esc no longer closes the window (Timeshift's doesn't); it closes a dialog, then leaves
    Settings or About, then clears the selection.
  - The busy fallback poll stays, slower (5 s instead of 3 s), only while this window was told
    busy and no `JobChanged` ended the job: in case the broadcast doesn't reach this process
    (bus policy, an older helper). Normally `JobChanged` ends it.
  - Settings has Cancel beside Save (drops the changes) and asks Save / Discard / Cancel when
    leaving with changes; an import or conversion doesn't ask (it's the file's content, shown).
  - Double-click on a row does nothing (0.5.0: restore).
- **Screenshots without a COSMIC session:** `APSIS_SCREENSHOTS=<dir> cargo test -p apsis
  screenshots` renders the real views with the headless tiny-skia renderer (a fresh renderer
  per shot: a reused one keeps what it drew before). libcosmic's bars animate from empty on
  redraw, so a single frame shows them empty. Not a substitute for the owner's screenshots.
- **Copy:** `i18n/en/apsis.ftl` rewritten in sentence case; every key is used. The desktop
  `Comment=`, the metainfo `<summary>`, the crate and .deb descriptions say "System snapshot
  and restore"; "Timeshift-style" and "file restore" are gone from everything but history
  (CLAUDE.md's listing line changed, as the owner asked).
- **Docs:** README, man page, UI.md, ARCHITECTURE.md, SECURITY.md (0.4.x supported, the stop
  and job-status paragraphs, five actions), CHANGELOG 0.4.0 (with the upgrade and downgrade
  notes), RELEASE.md (0.4.0 checklist, collection drafts), metainfo 0.4.0 release,
  `resources/deb/changelog`. Version 0.4.0.
- **Not done / for the owner:** new screenshots (README no longer shows the 0.3 ones; the
  metainfo still points at the `v0.3.0` copies); the HP checks in PLAN "0.4.0" J; the 0.3.1 work
  and everything since is uncommitted.

## 2026-09-30 - Filters tab: checkboxes instead of the sign button

From the owner's first look at the new window (a theme with a monospace interface font): the
fixed-width `+ Include` / `- Exclude` button wrapped to two lines. Each filter row now has a
checkbox, as on the Include tab (checked keeps the path, unchecked leaves it out), then the
`+` or `-` in the accent colour in a fixed narrow column so the patterns line up, then the
pattern; heads "Keep" and "Pattern" above the list. Clicking the row still selects it for
Remove / Move Up / Move Down. The screenshot test can render with the monospace face
(`APSIS_SCREENSHOTS_MONO=1`) and has a Filters shot with a converted config's notes.

## 2026-09-30 - 0.4.0 released; the HP checks; screenshots

- **Released:** `v0.4.0` tagged and published. 0.4.0 works on apsis-test; edge-case checks 3-6
  not run, left to issue reports. (PLAN "0.4.0" J, items 3 to 6: stop from another user's
  session, stop escalation, a killed helper's leftover, the disk unplugged mid-create.)
- **Screenshots:** `docs/1.png` (window), `2.png` (settings, Location), `3.png` (settings,
  Include), `4.png` (panel popup), added in `0da47b9`, after the tag. The README uses them by
  relative path instead of GitHub user-attachments links, so they're versioned with the code.
  The metainfo needs absolute links and the `v0.4.0` tag doesn't have them, so it points at
  that commit's copies (`raw.githubusercontent.com/.../0da47b9.../docs/N.png`), which never
  change. The 0.3 captions (activity, keep last N) are gone.
- **CI:** `cargo test` failed on the runner with "File exists" (os error 17): the app tests
  shared one `by-uuid` stand-in folder and raced to create its symlink on parallel threads.
  Each call now gets its own folder (pid and a counter); fixed in `7ee73fb`, test-only, no
  retag.

## 2026-09-30 - 0.5.0 full restore: design approved with changes (owner)

Design in PLAN.md, "0.4.1" and "Phase 6b". No code yet.

- **Apsis's own design, not Timeshift's behaviour.** Timeshift restores an rsync snapshot
  live and restarts. It copies whatever the snapshot has under `boot/efi`, and its boot refresh
  is GRUB-oriented. Three parts of Apsis's restore are its own, and the README must present
  them that way (never "restores like Timeshift"):
  1. **Next-boot apply**: the copy runs in systemd's offline-update mode (`/system-update`,
     `system-update.target`), not on the running desktop. Condition: check 0.3 on apsis-test
     passes; otherwise the design comes back for review.
  2. **The ESP skip**: the restore never writes `/boot/efi` (nor `/recovery`) with rsync.
     Create keeps copying `/boot/efi` into snapshots, so a Timeshift restore of an Apsis
     snapshot doesn't wipe the live ESP (Timeshift runs rsync `--delete` without `-x`).
  3. **The kernelstub refresh and check**: `update-initramfs -u -k all` and `kernelstub` run on
     the restored system. Before that, the ESP's kernel files are backed up. Afterwards they're
     compared byte for byte with `/boot`, and put back if the check fails, with the running
     kernel's files kept until the check passes.
- **0.4.1 before 6b**: create adds `-A -X --numeric-ids` (not `-H`: Timeshift doesn't, a
  full-system hard-link table costs memory and time, and the in-system hard links are few and
  harmless as copies). `info.json` gets `apsis-rsync-flags`. Old-format snapshots are restored
  without `-A -X`, since `-X` would strip live file capabilities from unchanged files.
- **Keys** (answer 5): Enter and double-click stay as they are. The review cites
  APSIS-UI-PROMPT §9/§11, which are the 0.3 contract's (git `25b1bbb`), where double-click is
  reserved for details/browse. Full restore gets its own key: proposed Ctrl+Shift+R, clear of
  0.4.0's Ctrl+N, Delete, Ctrl+R, F5, Ctrl+, (settings), Ctrl+A, Up, Down and Esc. Ctrl+E is the
  alternative. Pending the owner's pick.
- **Conflict with 0.4.0, open**: the review asks for a boxed Restore button "in details", and
  for old-format snapshots to be marked "in details". 0.4.0 has no details pane (removed with
  the terminal look), and the 6b brief says 0.4.0 wins over the contract. Proposed: the
  toolbar Restore button is the boxed button, and the old format shows in the row's tooltip
  and the Restore dialog. Waiting for the owner.
- Other answers: one dialog (date, name, comment, one summary line; no typing the name); safety
  snapshot on by default, space checked before the restart with a one-line refusal; home kept
  by default, "keep" = `/home` fully excluded; Pop!_OS + kernelstub only, and the snapshot's
  root UUID must be the current root's; live `fstab`/`crypttab` kept; `/etc/apsis/` protected.
- **Test baseline on apsis-test**: taken only after Wi-Fi, the `sudoers.d` entry and the
  test-only polkit rule exist, so a test restore can't cut the SSH link. Those files stay on
  apsis-test only.

## 2026-09-30 - 6b UI review (owner), from the throwaway preview

The owner reviewed the 6b screens in a preview (branch `preview-6b-ui`, debug builds only,
`apsis --window --preview <state>`, made-up data, no helper). PLAN 6b.8 is the reference now.
This resolves the two open items of the entry above.

- **Details conflict, resolved:** no details area. The old-format mark goes in the Restore
  dialog (the main place) and the list row's tooltip; no glyph or column. The toolbar's Restore
  is the one Restore button.
- **One Restore, no choice step:** file restore went in 0.4.0, so "Files… / Whole system…" was
  dropped after the preview showed it. The title says "Restore the system?".
- **No key for Restore:** it's reached by a click, or Tab to the button and Enter. This closes
  the key question of the entry above (Ctrl+Shift+R and Ctrl+E dropped). 0.4.0's rule stays:
  keys only in the README and man page, none in tooltips.
- **Armed only at Restart now** (reverses the owner's earlier "waiting for restart" point):
  the safety snapshot is taken while preparing, so anything changed between then and a restart
  hours later would be in no snapshot, and the restore would wipe it silently. The ready
  prompt has only Cancel restore and Restart now, plus "Save your work and close your apps
  first." (title and text as in the preview). Closing the pop-up counts as Cancel restore:
  Esc, Cancel restore or closing the window cancel and clean up. No waiting state, no waiting
  line in the status area.
- **No forced focus:** dialogs open with libcosmic's default focus, which is none. In libcosmic,
  Enter presses only the focused button, so Enter in a freshly opened dialog does nothing.
  Restore and Restart now need a click or a deliberate Tab. The password prompt is the second
  guard.
- **Dialog fits the window:** the libcosmic dialog is centred over the whole window and doesn't
  limit its own height. Its body now scrolls within the window height minus 216 px (measured
  chrome of 196 px, plus 8 px margins), so the buttons always show. The window tracks its
  height from resize events. A layout test checks every state at 720 x 520 and 640 x 440.
- **Results after login:** in the window's status line only, where 0.4.0 shows job results
  (full text in the tooltip; there's no Log since 0.4.0). No desktop notification, and no change
  to the panel icon's colour or its tooltip. Roles on the phrase only: done plain; boot-kept
  "still boots the previous kernel" in warning (it runs, but boot needs attention); failed
  "incomplete" in destructive.
- **Restore again** opens the normal dialog for the same snapshot, with the normal defaults
  (home kept, safety snapshot on).
- **Refusal dialog:** titled "Can't restore this snapshot", the reason in the error colour (as
  in the first preview), one line on what to do in plain text, and Close.
- **Building in Claude's sandbox:** cargo reads `~/.gitconfig` (libgit2) and writes to
  `~/.cargo`, and the sandbox denies both. Claude builds with `HOME` and `CARGO_HOME` in its
  scratch folder (the registry linked read-only, the git cache copied). The pinned libcosmic
  commit was fetched once from github.com, as PRIVACY.md allows. The owner's own builds are
  unaffected.

## 2026-10-01 - 0.4.1 core: ACLs and xattrs in snapshots, the format key

Done as planned (PLAN, 0.4.1): create's rsync gets `-A -X --numeric-ids`, no `-H`; `info.json`
ends with `"apsis-rsync-flags" : "-aAX --numeric-ids"`. What the plan didn't say:

- **The format is read from the flags, not from the key being there.** A snapshot is new format
  only when the recorded short flags include both `A` and `X` (`-aAX` or `-a -A -X`; a long
  option like `--AX` doesn't count). A later Apsis that records other flags is then judged by
  what it kept, not by having written a key.
- **An odd value doesn't make the snapshot invalid.** The nine Timeshift members are still
  strict (a non-string one makes the snapshot invalid, as in Timeshift). Timeshift never reads
  Apsis's key, so a missing, empty or non-string value there only means the old format.
- **The xattr and ACL test** runs real rsync on temp trees: a `user.*` xattr set with
  `setxattr`, an ACL set with `setfacl` for the test's own uid. `u:0` was tried first and is
  refused with "Invalid argument" in a user namespace where uid 0 isn't mapped. Each part skips
  with a message where the filesystem or `setfacl` is missing. It also checks `--link-dest`:
  an unchanged file with an xattr or ACL is linked in the next snapshot, and a changed xattr
  alone makes rsync copy the file again.
- **No UI in 0.4.1 (owner).** The row tooltip needs the format in `List`. A snapshot on the bus
  is `(sss)`, and a 0.4.0 panel rejects an unknown tag letter, mode or name, so every way to
  add it to `List` breaks a running 0.4.0 panel, which by `names.rs` means a new interface.
  Options were: `Helper3` now, an extra method on `Helper2` (a second read-only mount per
  refresh), or wait. The owner chose to wait: 0.4.1 changes no interface and needs no log out,
  and nothing in 0.4.1 can act on the mark. The carrier is decided in 6b's helper slice, with
  `CheckRestore`'s `old_format` for the dialog.
- **Building in Claude's sandbox, simpler:** a temporary `HOME` with the real `CARGO_HOME` and
  `RUSTUP_HOME` is enough. git needs `GIT_CONFIG_GLOBAL=/dev/null`. Claude can't commit there
  (no identity, and it must not set one), so the owner runs the commit commands.

## 2026-10-01 - 0.4.1 released

The owner ran the four apsis-test checks of PLAN's 0.4.1 section and reported all passed.
Version 0.4.1 in `Cargo.toml`, CHANGELOG, metainfo and the deb changelog; tag `v0.4.1`. The
version was set by hand, not with `just tag`: that recipe tags the bare version, and the
repo's tags are `v`-prefixed.

## 2026-10-01 - 6b core, part 1: the restore filter

`apsis_core::restore::filter` (PLAN 6b.2, 6b.3), on branch `restore-6b-core`. No root, nothing
outside the repo. What the plan didn't say:

- **Excludes are written once.** Groups 1 to 10 skip a rule that's already in the list (the ESP
  and `/recovery` are both a fixed rule and a mount; `/dev`, `/proc`, `/sys`, `/run` are both
  mounts and rule 6). The first match wins, so a repeat changes nothing. The snapshot's own
  `exclude.list` (group 11) is appended line for line, repeats included; only empty lines are
  dropped.
- **Mount points are escaped.** `*`, `?`, `[` and `\` in a mount point get a `\` in front, so
  the rule matches only that path. Checked with rsync 3.2.7 on temp folders: the escaped folder
  was kept and a lookalike that the unescaped pattern would also match was deleted. A mount
  point with a line break can't be written as one line of a filter file (rsync ends a line at
  `\n` and `\r`), so the filter isn't built and the restore is refused.
- **The protected kernel's version is checked** before it goes into a rule: letters, digits and
  `.-_+~` only, so it can't be a path or a pattern.
- **`has_home`**: any `exclude.list` line that starts with `+ /home/`. That covers Apsis's
  `+ /home/**`, Timeshift's per-user lines and "hidden files only", and also a single kept
  folder under `/home` (its `+ /home/` parent line).
- **The package test** reads the assets from `crates/apsis/Cargo.toml`'s
  `[package.metadata.deb]` and checks each installed path against the protect list.
- **Open, for the owner: a mount point that is a file.** `- <mount>/***` only matches a folder
  (checked with rsync 3.2.7: a plain file at that path was deleted). A file bind mount (a
  container's `/etc/resolv.conf`, some sandboxes) is then not excluded: rsync would try to
  replace or delete it. `- <mount>` matches a file or a folder, and rsync never enters an
  excluded folder, so it protects the same things plus that case. The code follows the plan
  (`/***`) until decided.

## 2026-10-01 - 6b core: mount rules are the plain path (owner), the restore argv

- **Rule 5 is `- <mount point>`, not `- <mount point>/***` (owner).** This closes the open
  point of the entry above. Checked with rsync 3.2.7 on temp folders, `-a --delete --force
  --exclude-from`:
  - `- /etc/bound/***`: a plain file `/etc/bound` on the receiver was deleted. `/***` only
    matches a folder.
  - `- /etc/bound`: the file was kept. `- /data`: the folder and everything in it were kept
    (rsync never enters an excluded folder).
  - rsync then says `cannot delete non-empty directory: etc` for a parent the snapshot doesn't
    have, and still exits 0.
  The fixed rules (2, 3, 6 to 10) keep their `/***` as in the plan. A mount is now listed
  beside them (`- /dev` and `- /dev/***`); the first match wins, so the pair is harmless.
- **Escaping follows from that.** rsync reads `\` as an escape only in a rule that has a
  wildcard character. A mount rule no longer ends in `/***`, so a mount point is escaped only
  when it has `*`, `?` or `[` itself (then those and each `\` get a `\`); otherwise it's written
  as it is. Checked with rsync 3.2.7: `- /srv/a\*b` kept `a*b` and deleted `aXb`; `- /srv/c\d`
  kept the folder named `c\d` and deleted `cd`.
- **The restore argv** (`restore::argv::rsync`, PLAN 6b.6 step 3) came forward from later in
  the slice, for its tests: exact for both formats; never `--delete-excluded`, `-L`, `-H`,
  `--link-dest` or the `--copy-*` options. One test holds both sides: the restore has
  `--delete` without `--delete-excluded`, and create still has `--delete-excluded`.
- **The target's own mount**: `/` never gets a rule, also when the mount table lists it more
  than once (a stacked root). The filter has no other target: its rules are anchored at the
  transfer root, so the same file works against `/mnt/` in the recovery steps (6b.11).
- **A separate `/home`**: the plan decides it (6b.2 rule 5, 6b.3). Kept: excluded like any
  other disk, never entered. Restored: `/home` is the one mount without a rule, so rsync
  restores into that partition; mounts below it keep theirs. Not a refusal (6b.7 lists
  `/boot`, `/usr` and `/var` only). Two things the plan doesn't say are with the owner: the
  space check for a restored separate `/home`, and a `/home` that isn't mounted at apply time.

## 2026-10-01 - 6b: a separate `/home` that is restored (owner)

Two gaps in the plan, both decided by the owner; in PLAN 6b.4, 6b.6 step 2, 6b.9, 6b.10 and
6b.12. Not built yet: each comes with its part of the core slice (space parsing, the plan
file, the apply state machine).

- **Space is checked on every destination partition.** When home is restored and `/home` is
  its own mount, what lands under `/home` is checked against that filesystem's free space, the
  rest against `/`. A separate `/home` is not a refusal.
- **A missing `/home` is "never started".** The plan file records the separate `/home`'s
  filesystem UUID. At apply time, when home is restored and `/home` was a separate mount at
  plan time, `/home` must be mounted with that UUID. If not, the apply stops before rsync
  runs: otherwise the home files would land on the system disk under `/home`, or in another
  disk mounted there.

## 2026-10-01 - 6b: the refusals (core)

`restore::refusal::check(system, snapshot)` gives the first failed check of PLAN 6b.7 as a
`Refusal`, or `Ok`. It opens and runs nothing: the helper reads mountinfo, lsblk, `findmnt`'s
root UUID, the names in `/boot/efi/EFI`, the snapshot's `info.json`, its kernelstub
configuration, its `boot/vmlinuz` link and the names in its `boot` and `usr/lib/modules`, and
passes them in. What the plan left open:

- **Not here:** the two space lines (they come with space parsing, 6b.4) and Busy (the
  existing error). `Refusal` has no text: the lines are the UI's strings (6b.8), and how a
  refusal crosses the bus is the helper slice's.
- **Order:** the table's, with one change: the snapshot must be readable (`info.json`, rsync,
  `localhost/`, `exclude.list`) before its UUID is compared, since the UUID comes from
  `info.json`. So the computer's reasons come first, then the snapshot's, the pending update
  last.
- **The root device** is the lsblk device whose UUID is `findmnt`'s root UUID, and its `TYPE`
  must be `part`. A root lsblk doesn't show is refused with the same line. An empty root
  UUID matches no device (a disk without a filesystem has an empty UUID too), so it can never
  pass by comparing empty with empty.
- **The root's filesystem type** is the last mountinfo line mounted at `/` (a later mount
  covers an earlier one), read by the new `usage::fstype_at`. With no line for `/` the
  restore is refused as "root not ext4" (`RootFilesystem`), with `unknown` as the type.
- **Split system:** something mounted at exactly `/boot`, `/usr` or `/var`. A mount below one
  (`/var/lib/docker`, `/usr/local`) or a separate `/home` isn't one.
- **The snapshot's kernelstub configuration** is parsed as JSON (serde_json, already a
  dependency), and only `kernel_options` of its `default` and `user` sections is read, as a
  list or as one string. Any `root=` option that isn't `root=UUID=<live root UUID>` refuses
  as another installation: another UUID, `PARTUUID=`, `LABEL=`, a device path. kernelstub
  adds `root=UUID=` itself from the mounted root, so most configurations have no `root=`,
  and that passes. A snapshot with no configuration file also passes (the plan asks only
  for the `kernelstub` program in the snapshot). A file that isn't a JSON object is refused
  as incomplete kernel files: kernelstub can't read it either, so the boot refresh would
  fail after the copy.
- **The snapshot's kernel** is the file name its `boot/vmlinuz` link points to, less
  `vmlinuz-`. It must read as a kernel version (the filter's rule), the file itself must be
  in the snapshot's `boot/`, and a folder of that name in its `usr/lib/modules`. The initrd
  isn't asked for: 6b.7's row lists the kernel, its modules and the two tools, and step 5
  runs `update-initramfs` after the copy.
- **Verified** with apsis-core's tests and clippy only; the workspace test and clippy run is
  the owner's.

## 2026-10-01 - 6b: snapshot format and the Apsis in a snapshot (core)

- **Format detection needs no new code.** `Info::is_old_format` (0.4.1) is what 6b asks: old
  means `info.json` has no `apsis-rsync-flags`, or the flags name neither `-A` nor `-X`
  (Timeshift's snapshots, Apsis 0.4.0's and older). `restore::argv::rsync` already takes it.
  As PLAN 6b.6 step 3 says, an old-format snapshot is restored with `-a --numeric-ids` and
  the rest, without `-A -X`, so the live extended attributes aren't stripped; the dialog
  shows the "Older format" line (6b.8). It isn't a refusal. A snapshot whose `info.json`
  can't be read has no format: it's refused first (6b.7).
- **The Apsis in a snapshot:** `restore::apsis::in_snapshot(dpkg_status)` reads the text of
  the snapshot's `var/lib/dpkg/status` (PLAN 6b.2) and gives one of the table's four cases:
  `Current` (no line), `NotInstalled`, `NoRestore { version }` (0.4.x), `OldSettings
  { version }` (0.3.x or older). It has no text: the lines are the UI's strings (6b.8), and
  how the helper's `apsis_note` carries the case is the helper slice's.

What the plan left open:

- **"This Apsis version" is any 0.5 or later**, not only the running one: such an Apsis can
  restore and show the result, so there's nothing to say. A different 0.5.x or a newer one
  gets no line.
- **No `var/lib/dpkg/status` in the snapshot** reads as no Apsis, as does a database without
  an `apsis` stanza. An Apsis installed without dpkg (`just install`) isn't seen: the plan
  reads dpkg's record only.
- **Installed** means the stanza's state (the last word of `Status`) is anything but
  `not-installed` or `config-files`: unpacked and half-configured have the files on disk.
  The first such stanza for the package `apsis` counts.
- **The version shown** is dpkg's upstream part: an epoch (`1:`) and a revision (`-1`) are
  dropped, so `0.4.1-1` is shown as `0.4.1`. Only the first two numbers are compared.
- **A version that can't be read** (no `Version` field, not two numbers) counts as 0.3.x or
  older, with the text as written, which can be empty. That's the cautious line (install
  Apsis again). How an empty one is shown is the UI slice's.
- **Verified** with apsis-core's tests and clippy only; the workspace test and clippy run is
  the owner's.

## 2026-10-01 - 6b: disk space (core)

6b.7 refusals (baa071a) and Apsis version reading (38e51c9): workspace test, clippy and fmt
passed on MAIN.
Correction to the format entry: old means the recorded flags lack -A or -X (either), not
neither.

`restore::space` holds 6b.4's numbers, and `Refusal` has the two space cases, `BackupSpace`
and `SystemSpace`, each with `needs` and `free` in bytes and no text. They aren't in
`refusal::check`: 6b.4's order is the refusals, then both dry runs and space, so the helper
calls `check` first and the space checks once it has the dry runs' output.

- `transfer_size(stats)`: "Total transferred file size" from rsync `--dry-run --stats`.
- `backup_needs(transfer)`: the safety snapshot's size and 1 GiB.
- `system_needs(transfer, total)`: the transfer and 1 GiB, or 2% of the partition if more.
- `check_backup(needs, free)`, `check_system(needs, free)`: refuse when `free < needs`.
- `split(transfer, under_home)`: what lands on `/` and what on a separate `/home`.

What the plan left open:

- **The size must be a plain byte count**: digits, with or without rsync's commas, then
  ` bytes`. Anything else (`1.23M` from `-h`, dots as separators, no such line) is unknown
  (`None`), never a smaller number. What the helper does with an unknown size (it should
  fail the preparation) is the helper slice's. The test text is rsync 3.2.7's real output.
- **The safety snapshot's size** is read the same way, from the create's own dry run.
- **GiB is 2^30 bytes; 2% is the partition's size / 50** (statvfs's total, `DiskUsage::total`).
  The same margin for `/` and for a separate `/home`.
- **Exactly enough is enough** (`free == needs` passes). Sums that overflow stay at the
  largest number, so they refuse.
- **`needs` includes the margin**, and it's the number the refusal carries (the line's
  "needs {n}") and the one to record in `request.json`. The check at "Restart now" is the
  same `check_system(needs, free)` with a fresh `statvfs`.
- **A short separate `/home`** gives `SystemSpace` with that partition's numbers (the plan's
  "with the system-disk line"). The caller checks `/` first.
- **The part under `/home`** is a number the helper passes in. How it gets it (a second dry
  run limited to `/home`, most likely) is the helper slice's. Two dry runs can disagree by a
  file that changed between them, so `split` never lets the home part be more than the
  whole, and `/` never needs less than nothing.
- **Verified** with apsis-core's tests and clippy only, run through a scratch workspace that
  holds only apsis-core (the workspace commands can't run in the sandbox); the workspace
  test and clippy run is the owner's.

## 2026-10-01 - 6b: the plan and state files (core)

Three files in `/var/lib/apsis/restore/` (`restore::file::DIR`), each with a type that
writes it, reads it back and validates it:

- `request.json`: `restore::plan::Plan` (PLAN 6b.9's list, field for field).
- `state.json`: `restore::state::State` (`attempts`, `written`; PLAN 6b.6).
- `result.json`: `restore::state::Report` with an `Outcome` (PLAN 6b.6 step 8, 6b.9, 6b.10).

Each has `parse`, `to_text`, `load(dir)` and `save(dir)`. `restore.filter` is the filter
module's text, `rsync-log` is rsync's, `esp-backup/` comes with the ESP step.

- **Core writes them.** The plan doesn't say who does. The apply state machine is core's
  (6b.13 step 1) and must save `state.json` before the step that depends on it, so the
  writer is core's too, and the helper has no second one. `save` is atomic: a temporary
  file (`<name>.apsis-tmp`, mode 0600) in the same folder, fsync, rename, fsync of the
  folder. A reader sees the old file or the new one. A temporary file left by a crash is
  removed by the next save. The folder isn't made here: it's the helper's (root, 0700), and
  a save into a missing folder fails. Tested in temp dirs.
- **A file that fails validation is refused whole** (`FileError::Invalid` with the reason,
  the file's name in front when it was loaded). No field of it is used. A missing or
  unreadable file is `FileError::Io`, so the caller can tell "not there" from "not
  trusted". `FileError` is the module's own type: `apsis_core::Error` is unchanged, so
  nothing in the helper or the applet has a new case to match.
- **Nothing is written that wouldn't be read back.** `to_text` and `save` run the same
  validation as `parse`, and fail with nothing written.
- **Versioning.** Every file starts with `"version": 1`. A reader takes only the versions it
  knows, which is 1 alone for now. Anything else is refused whole: older, newer, missing, or
  not a whole number (`"1"`, `1.0`), with "version 2 (this Apsis reads 1)", the config's
  wording. The version is looked at before the fields. What that means per file:
  - `request.json` and `state.json` live from "ready" to the end of the apply. The apply runs
    the helper copy made on arm, so it always reads its own version. A plan that isn't armed
    and was left by another Apsis version (an upgrade while the prompt was up) is refused,
    and is removed like any stale plan (6b.5).
  - `result.json` stays, and the Apsis reading it can be the snapshot's (6b.2). Another
    version's result isn't shown: there's no result line until an Apsis that reads it is
    installed. An Apsis that changes the format bumps the version and decides then whether
    it also reads 1, as the config does with 1 and 2.
- **Strict fields.** Every field must be there, and an unknown one refuses the file, also
  inside `separate_home`. "No value" is `null`, never a missing key. A same-version file
  with other fields isn't one this Apsis wrote.
- **Format.** One JSON object, indented, `version` first, a final newline. Sizes and times
  are JSON whole numbers (never strings or floats; `u64::MAX` round-trips). This is Apsis's
  own format, not Timeshift's all-strings one. A file over 64 KiB, one that isn't UTF-8 and
  a symlink in its place are refused unread.

What the plan left open:

- **`request.json`'s field names:** `snapshot`, `backup_uuid`, `home` (`"keep"` or
  `"restore"`), `old_format`, `safety_snapshot` (name or `null`), `root_uuid`,
  `running_kernel`, `root_needs`, `separate_home` (`null`, or `{"uuid", "needs"}`),
  `starter_uid`, `prepared_at`.
- **"Space needed per destination partition"** is `root_needs` and `separate_home.needs`,
  both with the margin (the space entry above). The separate `/home`'s UUID and its needs
  are one object, so one can't be there without the other.
- **Cross-field rules:** `separate_home` only with `home: "restore"` (a kept `/home` is
  never entered); its UUID isn't the root's; the safety snapshot isn't the snapshot being
  restored. The backup UUID may equal the root UUID (a backup on the system disk, filter
  rule 7).
- **Field checks:** snapshot names are `YYYY-MM-DD_HH-MM-SS` with a real date; UUIDs pass
  `usage::is_plain_uuid`; the kernel passes the filter's `is_kernel_version`; `starter_uid`
  fits 32 bits; times are Unix seconds after 1970.
- **Times are Unix seconds (UTC)**, not local time: the 30 minutes must not depend on the
  time zone.
- **Too old** (`Plan::is_too_old(now)`): more than 30 minutes (`MAX_AGE_SECS`) after
  `prepared_at`; exactly 30 minutes still passes. A plan made after `now` (the clock was
  set back) has an unknown age and is too old as well.
- **`state.json`:** `attempts` is 0 to 3 (`MAX_ATTEMPTS`). `attempts` and `written` must
  agree, since step 3 sets both before the copy: 0 with `written`, or an attempt without
  it, is refused. The arm writes `State::default()`.
- **`result.json`'s fields:** `outcome`, `snapshot`, `safety_snapshot`, `home`, `message`,
  `when`. The plan lists the outcome, the snapshot, the safety snapshot and the time;
  `home` is added because the result's tooltip says "Home folders were kept" and
  `request.json` is gone by then; `message` is the helper's reason (`RestoreResult`'s
  message), any text, empty for `done`.
- **Outcomes:** `done`, `problems`, `boot-kept`, `boot-broken`, `not-started`, `failed`.
  `ready` isn't one: `RestoreResult` answers it from `request.json`.
- **No kernel version in the result.** The `boot-kept` tooltip's "{version}" is the kernel
  the system runs when it's shown (`uname -r`), which is the kept one.
- **No `written` in the result.** A restore that never started after an earlier broken copy
  (6b.10) is `failed`, not `not-started`; choosing between them is the state machine's.

Left for later slices:

- The state machine: counting an attempt, and what an invalid `state.json` or
  `request.json` means at the arm check (never started; whether anything was written is
  then unknown).
- The helper: making the folder, removing a stale or refused plan, cutting `message` to a
  length worth showing.
- **Verified** with apsis-core's tests and clippy `-D warnings` through the scratch
  workspace. Each new test was seen failing first only as a compile error (the types
  didn't exist yet).

## 2026-10-01 - 6b: plan and state files, four fixes (core)

- **The state folder is protected, and tested.** It already was: `/var/lib/apsis/***` is on
  the protect list, and `restore::file::DIR` is under it. Two tests now hold that: `DIR` is
  under a protected `/***` path, and the restore's own argv with its own filter, run with real
  rsync over a temp "live root", deletes a folder the snapshot doesn't have but leaves
  `request.json`, `state.json` and `result.json` as they were. Nothing had to change in the
  filter.
- **A result's message is cut, never refused.** `Report::to_text` (so `save`) writes only the
  first `MAX_MESSAGE_BYTES` (2048) of `message`, ending on a character boundary; `parse` cuts
  the same way. The start is kept: the reason is Apsis's own words first, then rsync's lines.
  The worst case as JSON (every byte a `\u00XX` escape) is about 12 KiB, far under the 64 KiB
  read limit. So a failure report of any length saves and loads; tested with 1 MiB of plain
  text, of two-byte characters and of control characters. A report can still be refused for
  its other fields (a snapshot name that isn't one, a time before 1970): those come from the
  plan and the clock, not from rsync, and the state machine has to give them right.
- **Reads open with `O_NOFOLLOW`.** `load` no longer looks at the name and then opens it. It
  opens with `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` (rustix, already a dependency)
  and asks the open file whether it's a regular file. A link at the name fails the open
  (`ELOOP`) and is refused as "not a regular file", whatever it points to; a folder or a
  FIFO is refused by the check on the descriptor. `O_NONBLOCK` keeps a FIFO from holding the
  open up.
- **The temporary file is made with `O_EXCL`** (`create_new`, as before, now a function of
  its own with a test): a name that's taken, also by a dangling link, is an error. `save`
  removes a leftover first; a link planted at the temporary name is removed as a link and
  its target isn't written.
- **Two minutes of future skew.** `Plan::is_too_old(now)` passes a `prepared_at` up to
  `MAX_FUTURE_SECS` (120) after `now`: a clock corrected a little between preparing and the
  check. Further ahead, or more than 30 minutes old, is refused as before.
- **A hardware clock in local time** (dual boot with Windows) can make the clock at early
  boot differ by the zone's offset from the one the plan was made with, until the system
  corrects it. An armed plan can then look hours too old or future-dated at apply time. If
  the apply checks the plan's age, that must end as a clean refusal: never started, nothing
  written under `/`, `/system-update` removed, and `result.json` (`not-started`) saying why.
  It must never be a half-run restore or a boot loop. Whether the apply checks the age at
  all (PLAN 6b.5 asks for it at "Restart now" only) is the state machine's to settle; this
  is the constraint it works under.
- **`result.json` is read by the Apsis in the restored snapshot, which may be older** than
  the one that wrote it (PLAN 6b.2). What follows from the version rule:
  - Apsis 0.4.x and older don't know the file: no result line (the dialog says so before).
  - An older 0.5.x reads version 1 only, with exactly version 1's fields. A later Apsis that
    adds a field or bumps the version makes its result unreadable to that older Apsis, which
    then shows no result line after such a restore.
  - So `result.json` should stay version 1, with these fields, for as long as it can. A
    change to it costs the result line on every restore back to an older 0.5.x, and needs a
    decision of its own. `request.json` and `state.json` don't have this limit: the apply
    runs the helper copy that wrote them.
  - Not built: reading a newer file "as far as it's understood". That would be half-trusting
    it, which this slice rules out.
- **How the tests failed first.** With stubs that kept the old behaviour: the 1 MiB, the
  character-boundary and the control-character messages, the two-minute skew, the open that
  follows a link, and a temporary file opened without `O_EXCL` (the stub left it out) all
  failed on their assertions. The two protection tests and the planted-link test passed at
  once, since that behaviour was already there.
- **Verified** with apsis-core's tests and clippy `-D warnings` through the scratch workspace.

## 2026-10-01 - 6b: the message's end, no age check in the apply, the disarm timer (owner)

- **A cut message keeps its end** (this replaces "the start is kept" in the entry above).
  rsync's summary and exit code come last. A message over `MAX_MESSAGE_BYTES` (2048) is
  stored as `... ` and its end, 2048 bytes at most together, the end starting on a character
  boundary (the cut moves forward to the next one). A message that fits is stored as it is.
  Reading a cut message and writing it again changes nothing more. `parse` cuts the same
  way. Tests: 1 MiB ending in rsync's error line, two-byte characters, control characters.
- **The apply does no wall-clock age check** (PLAN 6b.6 step 1). This settles what the entry
  above left to the state machine. The clock in early boot can be hours off (a hardware
  clock in local time), so comparing `prepared_at` with it would refuse good plans, or pass
  old ones. `Plan::is_too_old` stays for "Restart now" (30 minutes, 2 minutes of future
  skew), where the clock is the same one the plan was made with.
- **What covers an old armed plan instead: a disarm timer** (helper slice; PLAN 6b.5, 6b.9).
  Arming also starts a transient systemd timer in the current boot. If no restart happened
  within 10 minutes, it disarms: the arm is undone, `request.json` is removed, the journal
  gets a line, and the window shows the restore as cancelled. A transient timer doesn't
  survive the restart, so it can't fire during the restore's own boot.
  - The owner's words were "removes request.json, journal line, UI shows the restore was
    cancelled". PLAN also lists undoing the arm itself (`/system-update`, the unit and its
    wants link, the helper copy, `state.json`): with only `request.json` gone, the next
    restart would still go to offline mode and stop at the arm check. To confirm with the
    owner if that reading is wrong.
  - The UI line is the existing "Restore cancelled".
- **A result that can't be saved gets a minimal one** (state machine slice; PLAN 6b.10): if
  saving the real `result.json` is refused, the apply writes the outcome and "result could
  not be saved, see journal" instead.
  - **Open for that slice:** version 1's `result.json` needs every field, and a valid
    snapshot name and time. If one of those is what was refused, the minimal report has
    nothing valid to put there. Either the format lets `snapshot` (and the rest) be `null`
    in a minimal report, or the checks on a report loosen. Version 1 isn't released, so the
    format can still change without a version bump; it should be settled before 0.5.0,
    given that `result.json` is the file an older Apsis reads (entry above).
- **Verified** with apsis-core's tests and clippy `-D warnings` through the scratch
  workspace. The four changed message tests failed on their assertions before the change.

## 2026-10-01 - 6b: the commit point, and result.json's times (owner; follow-up)

Follows the entry above. Docs only; no code changed.

- **`/system-update` is the single commit point** (PLAN 6b.5, 6b.9). With the link the next
  boot restores; without it nothing does. Arm creates it last, after the unit, its wants
  link, the helper copy and `state.json` are written and synced. Disarm removes it first.
  This confirms the reading in the entry above (disarm undoes the arm, not only
  `request.json`) and gives it an order.
- **Leftovers without the link** arm nothing, and are cleaned by the next arm or the
  helper's next start. So a disarm, or an arm, that's cut short at any point leaves either
  a complete arm or nothing that acts.
- **The disarm service has `Conflicts=shutdown.target`**: it can't run once a restart has
  begun. Without it the timer could fire while the system is going down and remove the
  link under a restart the user asked for.
- **For the state machine slice, to decide before 0.5.0:**
  - `result.json` must not cross-check its time against the plan's or any other time. A
    hardware clock in local time makes the apply's clock and the plan's disagree by hours,
    so `when` before `prepared_at`, or far after it, is normal there.
  - The minimal report (PLAN 6b.10) may have `null` for the snapshot and the time, and only
    with the outcome `failed`. That answers the open point of the entry above in outline;
    the exact fields and the change to the format are that slice's.
- **What the code does today** (unchanged): no time is cross-checked. `Report`'s validation
  checks the snapshot name, the safety snapshot's name, and that `when` is after 1970, each
  by itself. `Plan`'s checks that `prepared_at` is after 1970. `Report` never sees a plan.
  The only comparison of a time with a clock is `Plan::is_too_old(now)`, which the caller
  runs at "Restart now", not when a file is read. `null` for a report's snapshot or time
  is refused for every outcome today, so the minimal report needs the format change.

## 2026-10-01 - 6b: ESP backup, check and put-back (core)

`restore::esp` holds PLAN 6b.6's steps 4 and 6 as functions on paths (the ESP, the root, the
state folder), tested on temp trees with a fake ESP laid out as on apsis-test.

- `back_up(esp, state_dir, root_uuid)`: the four files to `state_dir/esp-backup/`.
- `verify(state_dir)`: the backup against its manifest.
- `check(esp, root, root_uuid)`: step 6's check; gives the kernel version or a `CheckFailure`.
- `put_back(esp, state_dir, root_uuid)`: the backed-up files back on the ESP.
- `remove(state_dir)`: step 8's cleanup.
- `esp_needs(on_esp, restored)`, `check_esp_space(needs, free)`: the ESP's space.

**The snapshot from before a kernel update** (the ESP boots a kernel whose modules the
snapshot lacks) is spelled out in the plan, so nothing was picked: rule 10 keeps the running
kernel's `/boot` files and modules through the copy; the boot refresh moves the ESP to the
snapshot's kernel; step 6 checks it; a failed check puts the ESP back to the kept kernel
(`boot-kept`); step 7 removes the kept kernel only after a passed check. Two tests walk that
snapshot through a refresh that works and one that fails.

What the plan left open:

- **Verification is the plan's byte compare, plus a size and SHA-256 on record.** Right
  after each copy, the copy is compared byte for byte with the original (PLAN step 4). The
  size and SHA-256 of what was read go into `esp-backup/manifest.json`. A byte compare proves
  the copy at that moment; the manifest is what lets a later step (a put-back, or a boot
  after a power cut) tell that the backup is still whole.
- **`sha2` is now a direct dependency of apsis-core** (`default-features = false`). It was
  already in `Cargo.lock` (0.11.0) and in the cargo cache, so nothing is downloaded and no
  package is added to the tree. `Cargo.lock` got one line by hand (`"sha2"` in apsis-core's
  list), since the workspace commands don't run in the sandbox: the owner's first workspace
  build confirms or rewrites it. The alternative was a hand-written SHA-256, which is worse
  to own.
- **The manifest** uses the plan and state files' rules (`restore::file`: version 1, exact
  fields, refused whole, atomic write). Fields: `root_uuid`, then one per file by its name
  (`vmlinuz.efi`, `initrd.img`, `Pop_OS-current.conf`, `Pop_OS-oldkern.conf`), each
  `{"size", "sha256"}` or `null`. It's written last, after the files and their folder are
  flushed, so a backup that a power cut stopped has none and counts as no backup.
- **The oldkern entry may be missing** (a machine with one kernel): recorded as `null`. The
  kernel, the initrd and the current entry must be there, or the backup fails (`Missing`).
- **A failed backup leaves no folder**, so "nothing to put back" (PLAN step 4) is what the
  state machine finds.
- **A backup is never made over an earlier one**: `back_up` fails with `AlreadyExists`. The
  rule for a boot after a power cut (take it again only if the ESP still matches the
  protected kernel, else keep the earlier one; PLAN 6b.10) is the state machine's, with
  `verify` and `remove` to build it from.
- **A link in place of a boot file is refused**, not followed (`O_NOFOLLOW`, as the state
  files). vfat has no links; this is for anything else mounted there.
- **The root UUID** must pass `usage::is_plain_uuid` before it's put in a path.
- **The put-back verifies the whole backup first**, so a damaged kernel is never written to
  the ESP and a refused put-back leaves the ESP untouched. A backup of another root UUID is
  refused the same way. Then each file: a temporary name in its folder
  (`<name>.apsis-tmp`), fsync, rename, fsync of the folder, and a byte compare with the
  backup (PLAN step 6). A temporary file left by an earlier put-back is removed first.
- **A file that wasn't backed up is left alone** by the put-back: an oldkern entry that
  kernelstub made since isn't Apsis's to remove.
- **An error after the put-back started writing** means the ESP's state is unknown: that's
  `boot-broken`. Mapping errors to `boot-kept` and `boot-broken` is the state machine's.
- **The check** reads only the last part of the `/boot/vmlinuz` and `/boot/initrd.img`
  links (`vmlinuz-<version>`, `initrd.img-<version>`) and looks the files up in the root's
  own `boot`, so an absolute link can't lead outside the root. The version must read as a
  kernel version (the filter's rule). A file that can't be read counts as one that differs.
  Order: the kernel link, the kernel, the initrd, the modules, the current entry. That both
  commands exited 0 stays with the caller.
- **ESP space: the plan has no rule, so this one is new, and the least certain pick here.**
  `esp_needs` is what the kernel and the initrd each grow by (the snapshot's `/boot` files
  against the ESP's, never less than nothing), plus the larger of the ESP's two (the
  put-back's temporary copy), plus 16 MiB (the initrd is rebuilt, so the snapshot's size is
  an estimate; FAT rounds up to clusters). Short: `Refusal::BootSpace { needs, free }`, a
  new case. It isn't in `refusal::check` (it needs sizes, like the other space lines). Its
  two dialog lines are the UI slice's; PLAN 6b.7 has the row.
  - It assumes kernelstub writes over the files in place. If it writes a new file first,
    the growth term is too small.
  - It isn't recorded in `request.json`, so it isn't checked again at "Restart now". The
    ESP changes only on a kernel update, which gets Busy while the prompt is up only if it
    goes through Apsis, which it doesn't. To decide with the helper slice.

For the owner:

- **The "previous" kernel on the ESP isn't in the plan's four files.** The oldkern entry
  boots its own kernel and initrd copies in `EFI/Pop_OS-<root-uuid>/` (kernelstub's
  `-previous` files, as far as the code was read from memory, not from the machine), and
  the boot refresh rewrites those too. After a put-back the oldkern entry is the old one,
  but the files it names are whatever kernelstub left. The current entry, which is what
  boots, is covered. Check 0.1's listing of the ESP folder settles the names; adding them
  is two more `BootFile`s and manifest keys.
- **The fake ESP is a folder on the test's filesystem, not vfat.** Rename-over and fsync
  behave the same for this code, but vfat's own limits (no links, case folding) aren't
  exercised until apsis-test.

- **Verified** with apsis-core's tests and clippy `-D warnings` through the scratch
  workspace. The new tests were first seen failing as compile errors; three deliberate
  breakages then failed on assertions and were undone: a verification that compares sizes
  only, a check that ignores the modules, and a put-back that doesn't rename.

## 2026-10-01 - 6b: the ESP file set, the previous pair, which tree is checked (owner; follow-up)

The owner listed a real ESP (Pop!_OS 24.04, kernelstub; apsis-test to confirm). It settles the
open point of the entry above ("the previous kernel isn't in the plan's four files") and
changes `restore::esp`.

- **Seven files, one list.** `esp::SET` is the only table of ESP files: file, folder, name,
  required. `BootFile::ALL`, `name`, `esp_path`, `is_required`, the manifest's keys and the
  backup all read it. Added: `cmdline`, `vmlinuz-previous.efi`, `initrd.img-previous` (the
  names differ in where "previous" sits; taken as listed). PLAN 6b.6 step 4 has the table.
- **Optional together.** The previous pair and `Pop_OS-oldkern.conf` are all there or all
  missing (one kernel installed). The rest is required, `cmdline` included.
  - **Picked here, not said by the owner: only part of the three fails the backup**
    (`EspError::PreviousIncomplete`), and a manifest with part of them is refused. So the
    apply ends `boot-kept` with nothing put back, as for a missing required file. The other
    reading (back up what's there) would let a put-back write an oldkern entry whose kernel
    isn't there. In the check, the same state is only reported (below).
- **The manifest** has seven file keys now, in the list's order. Still version 1: no
  released Apsis wrote the four-key one, and an old one is refused whole (unknown or missing
  fields), which counts as no backup.
- **The check gives more than a version.** `check` returns `Checked { version, previous }`.
  `previous` is `Absent`, `Good { version }` or `Wrong(CheckFailure)`: the previous pair
  against `/boot/vmlinuz.old` and `/boot/initrd.img.old`, and that version's modules folder.
  `Wrong` never fails the check (the current pair still boots): no refusal, no put-back. It's
  for the journal and the result. The oldkern entry's text isn't read, only that it's there.
- **Which tree.** `check(esp, root, root_uuid)` always took the root as a path; PLAN 6b.6
  step 6 now says which: the live system before arming, the restored tree after the boot
  refresh. PLAN didn't say the first before. **New with it: `Refusal::BootFiles(CheckFailure)`**
  from `esp::check_before_arming`, for a live ESP whose current pair isn't what `/boot` links
  to. Rule 10 protects "the kernel the ESP boots" by the running version, which only holds
  if they agree. PLAN 6b.7 has the row; its dialog lines are the UI slice's. It isn't in
  `refusal::check` (it reads files, and that one works on text already read).
- **Never touched:** `loader/entries/Recovery-*`, `EFI/Recovery-*`, `loader/loader.conf`,
  `loader/random-seed`, `loader/entries.srel`, `EFI/BOOT/`, `EFI/systemd/`. Nothing in the
  code names them: the list is closed, and the tests hold it (every path of the list is under
  `EFI/Pop_OS-<root-uuid>/` or is a `loader/entries/Pop_OS-*`; the backup folder holds the
  seven and the manifest; after a put-back those files have the same inode, mtime and bytes,
  and the whole ESP tree is what it was).
- **Space counts both pairs.** `esp_needs(on_esp, restored)` takes `EspSizes { current,
  previous }`: the growth of each of the four files by itself (a pair that shrinks makes no
  room for the other), the largest file on the ESP (the put-back's temporary copy), 16 MiB.
  A missing previous pair is zero bytes on either side. `cmdline` and the entries are in the
  margin. With the owner's sizes (17,273,344 / 214,307,600 and 17,056,256 / 212,169,876;
  361M free of 1020M): 231,084,816 bytes needed for a restore to the same kernels or to a
  rollback of the same sizes, so 147,451,120 bytes of growth fit, and one more refuses.
  With one kernel on that ESP and two in the snapshot: 460,310,948 needed, which fits in
  what's free then (the previous pair's bytes are free too).
- **Re-checked at "Restart now", not stored** (owner; settles "to decide with the helper
  slice" above): no ESP field in `request.json`. The helper works the needs out again and
  reads `statvfs` of `/boot/efi`. The core has the two pure functions; reading the sizes is
  the helper slice's.
- **Still assumed:** kernelstub writes over the files in place (else the growth term is too
  small), and it rewrites the previous pair from the `.old` links on a refresh. The
  kernel-rollback check on apsis-test (check 2) shows both.

- **Verified** with apsis-core's tests (256 unit, all green) and clippy `-D warnings`
  through the scratch workspace; the workspace run is the owner's. The new tests were first
  seen failing as compile errors on the missing API. The put-back test for the untouched
  files was then broken on purpose (a put-back that rewrites `loader/random-seed` with the
  same bytes) and failed on the assertion; undone.

## 2026-10-01 - 6b: a partial previous set refuses before arming (owner; follow-up)

- **`check_before_arming` refuses a partial optional set**: some but not all of
  `vmlinuz-previous.efi`, `initrd.img-previous` and `Pop_OS-oldkern.conf` on the live ESP.
  All three or none pass. So it's caught at arming and again at "Restart now" (the helper
  runs the same function at both), before anything is copied, instead of ending `boot-kept`
  in the apply.
- **The apply keeps its backstop**: `back_up` still fails with `EspError::PreviousIncomplete`
  (an ESP that changed between "Restart now" and the apply).
- **No new variant**: `Refusal::BootFiles(CheckFailure::PreviousIncomplete)`. `BootFiles`
  already carries which thing is wrong, and the UI slice words each `CheckFailure` anyway.
- **Unchanged**: `check` itself still only reports it (`Previous::Wrong(PreviousIncomplete)`),
  so after the boot refresh in the apply it's no failure and no put-back. A whole previous
  pair that isn't the `.old` links' still doesn't refuse.
- PLAN: a new 6b.7 row, and 6b.6 step 6's "previous pair" point.
- **Verified** with apsis-core's tests (258 unit) and clippy `-D warnings` through the
  scratch workspace; the workspace run is the owner's. The partial-set test was seen failing
  first (`Ok(.. Wrong(PreviousIncomplete))` where the refusal was expected); the full and
  empty set test passed from the start, as it holds what was already so.

## 2026-10-01 - 6b: the apply state machine (core)

`restore::apply::apply(paths, runner)` is PLAN 6b.6 as a state machine. `Paths` is the state
folder, the ESP and the root, so it runs on temp trees. `Runner` is everything that needs
root or a real machine: `is_armed`, `open_backup` (step 2), `copy` (pass 1), `back_up_esp`,
`refresh_boot`, `put_back_esp`, `remove_protected_kernel`, `disarm`, `now`, `say`, `restart`.
It returns `End::NotArmed`, `End::Retry { attempt }` or `End::Finished(Outcome)`. The plan,
the state, the result and the ESP backup go only through `plan`, `state` and `esp`: there's
no JSON in the module. The tests use a fake runner on the ESP module's temp-tree lab (its
test module is now shared inside `restore`).

The outcomes, as built (PLAN's names):

| what happens | outcome | ESP |
|---|---|---|
| step 2 fails, nothing written yet | `not-started` | untouched |
| step 2 fails after an earlier copy wrote | `failed` | untouched |
| the copy breaks, attempts 1 and 2 | retry, no result | untouched |
| the copy breaks on attempt 3, or the power cut attempt 3 | `failed` | untouched |
| the ESP backup fails (also `PreviousIncomplete`, the backstop) | `boot-kept` | untouched, no refresh |
| the refresh fails, or exits 0 and the check against the restored tree fails | `boot-kept` | put back |
| the put-back fails, or works onto a kernel whose files are gone | `boot-broken` | unknown |
| the check passes | `done`, or `problems` after exit 23 | refreshed |
| the check passes, the kept kernel can't be removed | `problems` | refreshed |
| `request.json` or `state.json` missing or refused | `failed` | untouched |

Picked where PLAN was open, or changed from it:

1. **`state.json` has two more fields: `step` and `problems`** (version 1, unshipped).
   `step` is `armed`, `copy`, `boot-files` or `end`. Without it a boot after a power cut
   can't tell a copy that ended from one that was cut, and `attempts` would need a fourth
   count for a re-run after the third copy ended. `problems` carries exit 23 over a power
   cut. Validation: `armed` only with no attempt; `copy` and `boot-files` only with one;
   `problems` only after the copy ended.
2. **A copy that ended is never run again** (PLAN said the next boot runs pass 1 again and
   "finds almost nothing to copy"). The price: `Runner::copy` must `sync` before it returns,
   since rsync doesn't. It's in the trait's doc and in PLAN step 3. It also saves a full scan
   of `/` on a resumed boot.
3. **The power cutting the third copy is `failed`**, with no fourth copy: `attempts` is
   saved before the copy, so 3 with step `copy` means the third never ended.
4. **The ESP backup is never taken twice** (owner: "never retake it once rsync has
   started"). A folder that's there and verifies (and is of this root UUID) is kept. This
   replaces PLAN 6b.10's "taken again only if the ESP still matches the protected kernel".
5. **A backup that's there and doesn't verify**: no retake, no boot refresh, no put-back.
   The outcome is read off the ESP as it is: it passes the check against the restored tree:
   `done` or `problems`, with step 7; else it's the kernel from before, whole
   (`esp::boots_kernel`, new): `boot-kept`; else `boot-broken`.
   - This covers the power going during the backup itself (no manifest), where a retake
     would in fact be safe (no refresh can have run). It ends `boot-kept` instead. Allowing
     that one retake is a small change if the owner prefers it.
   - In the "passes the check" branch nobody knows whether both commands exited 0 in the
     boot that was cut. The check is the same byte compare, so it's taken as passed.
6. **`boot-kept` is checked, not assumed**: after a put-back that worked, the ESP must boot
   the plan's running kernel with its `/boot` files and modules there. If not: `boot-broken`.
7. **The order of step 8**: `result.json`, then step `end`, then disarm (the link first),
   then `esp-backup/`, then restart. PLAN listed the removals first. With the result first,
   a power cut never leaves a finished restore without a result; a boot that finds the link
   and step `end` only cleans up.
8. **Every end restarts** (`Runner::restart`, once, last). "Boot normally" is a restart with
   the link gone. How the helper does it (logind, `reboot.target`) is its own.
9. **A state that can't be saved stops the step that depends on it**: before the copy,
   `not-started` (or `failed`); before the boot files, `boot-kept` with the ESP untouched.
   The `end` save is the only one that's allowed to fail.
10. **An unreadable `request.json`**: no apply, disarm, `failed` with `snapshot: null` and
    the reason, plus "nothing was written" if `state.json` reads and says so, else "the
    system may be partly restored". It can't be `not-started`, because only `failed` may
    lack the snapshot (item 12).
11. **An unreadable `state.json`** (the plan reads): the same, `failed` with the snapshot
    named and "whether anything was written isn't known".
12. **`result.json` version 1 changed** (unshipped): `snapshot` and `when` may be `null`,
    only with `failed`. `Report::snapshot` and `Report::when` are `Option`s.
13. **The minimal report is always `failed`.** PLAN said "the outcome is never lost"; the
    owner's rule says null only with `failed`. Both hold this way: the message is "result
    could not be saved, see journal (the restore ended: <outcome>)". It keeps the snapshot,
    the safety snapshot and a time after 1970; if that's refused as well, it's the message
    alone. The one way to get there with a good plan is a clock at or before 1970, where a
    finished restore is then shown as `failed`. That's the cost of the rule; say if `when`
    should be allowed to be null for every outcome instead.
14. **`End::Finished` gives the outcome that's in `result.json`**, so the journal and the
    window agree.
15. **Step 7 has a guard in core**: the runner is never asked to remove a kernel the ESP
    boots (the checked current or previous version), whatever the snapshot holds. A kernel
    that can't be removed is `problems`, not a failure.
16. **No clock decides anything.** `Runner::now` is asked once, for the result's time.
    `Plan::is_too_old` isn't called. Tested with a plan from 1970, one from the clock's
    future, and a clock set back.
17. **`CheckFailure` has words** (`Display`), for the result's message.
18. **Stale plan removal isn't in core**: PLAN 6b.5 gives it to the helper ("removed at the
    helper's next call", "the next arm or the helper's next start"). Nothing was built.

Not covered here:

- The real runner, and so anything about rsync itself: the real-rsync temp-tree tests are
  the next slice.
- A power cut inside one runner step (rsync half done, a put-back between two files) is the
  step's own to survive: rsync picks up, `esp::put_back` removes its leftover temporary
  file. The cut test stops before and after each thing asked of the runner, not inside.
- The cut tests make about 230 temp labs with fsyncs, so apsis-core's unit tests now take
  about 10 s instead of under 1 s.

- **Verified** with apsis-core's tests (300 unit, 35 of them the apply's) and clippy
  `-D warnings` through the scratch workspace; the workspace run is the owner's. The state
  and result tests, and `boots_kernel`'s, failed first as compile errors on the missing API.
  The apply's 35 failed first on a body that was `todo!()`. Since they then passed in one
  step, five deliberate breakages were run and undone, each failing on assertions: the
  attempt not saved before the copy (5 tests), a backup that doesn't verify taken again
  (4), the end not saved before the cleanup (6), a refresh that exits 0 taken as passed
  without the check (1), no limit on attempts cut by the power (1).

## 2026-10-01 - 6b: the apply, follow-up: backup retake, minimal report, cuts inside a step, the link (owner)

Five points from the owner on the entry above; they replace its picks 4, 5 (in part), 8, 12
and 13.

1. **A backup cut by the power is taken once more, while the boot refresh hasn't started.**
   - `state.json` has a fifth step, `boot-refresh`, saved after the ESP backup is whole and
     before the boot refresh runs. `boot-files` now means: the copy ended, the boot files
     are untouched.
   - At `boot-files`, a backup folder with no manifest is removed (`esp::remove_partial`)
     and the backup taken again. The path is validated: only a real folder named
     `esp-backup` in the state folder, never through a link, and never one that has a
     manifest, readable or not.
   - At `boot-refresh`, no backup is ever taken: not over a partial folder, not when the
     folder is gone. That ends as before (no refresh, no put-back, the outcome read off the
     ESP).
   - **Picked:** a backup that has a manifest and doesn't verify is damaged, not partial. It
     isn't removed or retaken at either step.
2. **`when` may be `null` with any outcome; `snapshot` only with `failed`.** `result.json`
   stays version 1 (unshipped). The minimal report keeps the real outcome, with the message
   "result could not be saved, see journal" and no time unless the clock gave one after 1970.
   - **Picked:** if that's refused too, the only field left that can be wrong is the
     snapshot's name. The report is then `failed` without a snapshot, and its message names
     the real outcome. A validated plan can't produce this; it's tested directly.
3. **Cuts inside a step**, with the fake runner: a backup with two of seven files copied and
   no manifest; a put-back with the kernel and initrd back, the rest not, and a temporary
   file on the ESP. The next boot ends `done` or `boot-kept`, and the seven files are all the
   refreshed ones or the whole ESP tree is what it was before.
   - **Found by the test:** a put-back that's cut leaves `<name>.apsis-tmp` on the ESP (up to
     an initrd's size). If the next boot's refresh then works, nothing removed it. New:
     `esp::clear_temporaries` removes exactly those names beside the seven files, before the
     boot refresh.
4. **The restart loop, and `systemd.offline-updates(7)`** (systemd 255, read on this machine).
   What the design rests on, quoted:
   - Point 5: "As the first step, an update service should check if the /system-update or
     /etc/system-update symlink points to the location used by that update service. In case
     it does not exist or points to a different location, the service must exit without
     error."
   - Point 6: "After completion (regardless whether the update succeeded or failed) the
     machine must be rebooted, for example by calling systemctl reboot."
   - Point 7: "If the system-update.target is successfully reached, i.e. all update services
     have run, and the /system-update or /etc/system-update symlink still exists, it will be
     removed and the machine rebooted as a safety measure."
   - Recommendation 2: "Make sure to remove the /system-update and /etc/system-update
     symlinks as early as possible in the update script to avoid reboot loops in case the
     update fails."
   - Recommendation 3: "Use FailureAction=reboot in the service file for your update script
     to ensure that a reboot is automatically triggered if the update fails."

   Checked against them:
   - **A link that can't be removed gets no restart.** `Runner::disarm` is split into
     `remove_link` and `remove_arm_files`. If `remove_link` fails, nothing else is removed,
     `Runner::restart` isn't called, and the apply returns `End::LinkStuck { outcome }` for
     the helper to exit 0. Point 7 then applies: systemd removes the link and restarts. The
     result and step `end` are saved before, so a boot that comes back with the link still
     there does no work and again doesn't restart.
   - **With attempts capped at 3**: the apply restarts over a link only as a retry, and
     `attempts` is on disk before each copy. Boots 1 and 2 restart; the third broken copy
     ends the restore; with a stuck link that boot, and any later one, ends without a
     restart. Tested: two restarts in six boots.
   - **A deliberate departure from recommendation 2**: the link is kept until the end, not
     removed early, so that a broken copy is retried at the next boot (PLAN 6b.10). The
     attempts are what bounds that.
   - **A conflict with point 5, fixed**: PLAN step 1 said to remove a link that isn't
     Apsis's, and the apply did. That link is another tool's pending update. Now it's left
     alone: the apply removes only its own leftover unit files, doesn't restart, and returns
     `End::NotArmed`. PLAN step 1 is changed, with the reason.
   - **Not covered, for the helper slice:** the apply can only bound what it returns from.
     If the helper dies before that (a panic, a kill) in a boot that counted no attempt, the
     unit's `OnFailure=reboot.target` restarts with the link in place, and nothing stops the
     next boot doing the same. PLAN's helper list now says the helper never exits non-zero
     with Apsis's link there. If systemd's own removal in point 7 fails too (a read-only
     `/`), the restart loop is systemd's, and each boot of it does no work in Apsis.
   - **Not read:** `system-update-cleanup.service` itself. The man page's point 7 is what's
     relied on; the unit's text and its condition are worth a look in the helper slice.
   - PLAN's unit has `OnFailure=reboot.target` where the man page recommends
     `FailureAction=reboot`: noted in the helper list, not changed.
5. **PLAN, helper slice**: the real runner's copy step calls `syncfs` on the restored
   filesystem (`/`, and a separate `/home` that's restored) after rsync exits; core saves
   step `boot-files` only after the copy step returns (6b.6 step 3, 6b.13 step 3).

- **Verified** with apsis-core's tests (314 unit, 46 of them the apply's) and clippy
  `-D warnings` through the scratch workspace; the workspace run is the owner's. The new
  tests failed first as compile errors on the missing API (the step, `remove_partial`,
  `clear_temporaries`, `remove_link`, `LinkStuck`). Deliberate breakages then failed on
  assertions and were undone: a backup taken again after the refresh started (2 tests), a
  restart over a stuck link (3), temporary files not cleared (1), the `boot-refresh` step
  not saved (3), the minimal report not keeping its outcome (3).

## 2026-10-01 - 6b: the apply, second follow-up: the boot count, the arming check, the temporary-file invariant (owner)

1. **Every offline boot is counted first.** `state.json` has a sixth field, `boots` (version
   1, unshipped; written last in the file). `MAX_BOOTS` is 5: three copies, and two boots to
   spare for power cuts.
   - **What runs before the count, exactly:** `Runner::is_armed` (the link is Apsis's), and
     `State::load` (open with `O_NOFOLLOW`, read, parse, validate), then the comparison with
     the cap. Nothing else: the plan is read after it, and the runner isn't asked for
     anything else before it.
   - **The count is the apply's first write.** If it can't be saved, the boot does nothing
     else and the restore ends (`not-started`, or `failed` if an earlier copy wrote), with
     the link removed. One exception: step `end` is already saved, where only the cleanup is
     left and the result isn't written again.
   - **Past the cap** (`boots` is 5 when a boot begins): `End::GaveUp`. The link is removed
     first, before anything that could stop the helper again; then `result.json` (`failed`),
     step `end`, the unit files. `Runner::restart` isn't called. If the link can't be
     removed it's `End::LinkStuck`, also without a restart.
   - **Picked:** the ESP backup is kept at a give-up (every other end removes it). If the
     boot refresh had started and the ESP boots neither the restored kernel nor the one from
     before, the message says so. The outcome stays `failed`, as asked.
   - **Picked:** `attempts` can't be more than `boots` (validation): a boot is counted
     first and starts one copy at most.
   - **Tested** with a fake runner that dies (unwinds, as a panic or a kill would) every time
     it gets to one thing, for each of ten: the backup disk, the copy, the ESP backup, the
     boot refresh, the put-back, the kept kernel's removal, the clock, the journal line, the
     unit files' removal, the restart. Each ends within 6 boots with the link gone, and no
     boot past the cap restarts.
   - **Not covered by the count:** a helper that dies in the link check, in reading
     `state.json`, or in removing the link. A test holds what that is: with a death at the
     link check, nothing is ever written.
   - **For the owner:** "no restart" at the cap is built as asked. With the link gone a
     restart would go to the normal boot and couldn't loop; without one the machine stays in
     `system-update.target`. The helper slice decides how it leaves; PLAN says so.
2. **Arming is refused while another update is pending.** `Refusal::PendingUpdate` and its
   6b.7 row were there already (`/system-update` exists, in `refusal::check`). No second
   variant was added. New: `refusal::check_arming(system_update, etc_system_update)`, pure,
   on what `lstat` found at each name (`UpdateLink`: nothing, a link, a dangling link, a file
   or folder). Anything at either name refuses, wherever it points, Apsis's own folder
   included. `/etc/system-update` is new: the man page says the generator reads both. The
   6b.7 row is rewritten.
3. **No end leaves a `.apsis-tmp` file on the ESP.** The test module's `boot` checks it after
   every apply that returns, so every existing test holds it. Making it true took one
   change: the temporary files are now also cleared when the apply ends (and at a give-up),
   not only before a boot refresh. A new test has a backup that doesn't verify and a
   leftover temporary file: no refresh runs, and the file still goes.
   - **Where it can't hold:** the plan is unreadable. The root UUID that names the ESP's
     folder is the plan's, so nothing is cleared then.
4. **PLAN's unit has `FailureAction=reboot`** in `[Unit]`, in place of
   `OnFailure=reboot.target` (`systemd.offline-updates(7)`, recommendation 3).
5. **Known limitation: a `/` that's read-only, or any state where systemd can't remove the
   link either.**
   - The apply can't remove `/system-update`, so it doesn't restart (`End::LinkStuck`) and
     the helper exits 0. `state.json` can't be written either, so no boot is counted.
   - systemd then does what the man page's point 7 says: it removes the link "and the
     machine rebooted as a safety measure". If its removal fails as well, whether it still
     restarts, and so whether the machine loops, is systemd's behaviour. It wasn't tested
     here and `system-update-cleanup.service` wasn't read.
   - In such a loop each boot does no work in Apsis: the apply reads the link and the state,
     fails to write, and returns. Nothing under `/` is changed.
   - The way out is by hand: the recovery partition or a live USB, then
     `rm /mnt/system-update` (PLAN 6b.11's steps already end with that line).

- **Verified** with apsis-core's tests (323 unit, 52 of them the apply's) and clippy
  `-D warnings` through the scratch workspace; the workspace run is the owner's. The new
  tests failed first as compile errors on the missing API (`boots`, `MAX_BOOTS`, `GaveUp`,
  `check_arming`). Deliberate breakages then failed on assertions and were undone: a cap
  that's never reached (3 tests), a restart at the cap (2), the count not saved (2), the
  temporary files not cleared at all (2), not cleared at the end (1).

## 2026-10-01 - 6b: a give-up restarts once the link is gone (owner)

Replaces "`Runner::restart` isn't called" at the cap, and the "for the owner" point, in the
entry above.

- **`End::GaveUp` restarts.** The link is removed before `GaveUp` is returned, so the restart
  can only reach a normal boot: no loop. Without it the machine would stay in
  `system-update.target` with nothing left to run.
- **`End::LinkStuck` still doesn't restart**, at the cap as anywhere else.
- **The rule is core's**, in `apply`, and the helper never decides it: restart whenever the
  link is gone (`Finished`, `GaveUp`), and over the link only as a retry (`Retry`, two at
  most, each with an attempt on disk first). Never for `LinkStuck` or `NotArmed`.
- **Tests count restarts by that rule.** The fake runner counts restarts made while the link
  is still there. The cap test: one restart, none over the link. The test that kills the
  helper at each of ten points: at most two restarts over the link, and a boot past the cap
  restarts exactly once if it got to the end and not at all if it died first. The stuck-link
  tests are unchanged: no restart past the two retries.
- PLAN: 6b.10's boot cap and a line on who decides the restart, 6b.12, and the 6b.13 helper
  list (the helper only exits 0 after `LinkStuck` and `NotArmed`).
- **Verified** with apsis-core's tests (323 unit) and clippy `-D warnings` through the
  scratch workspace; the workspace run is the owner's. The two cap tests were changed first
  and failed on their assertions (no restart where one is now expected), then passed with
  the one-line change.

## 2026-10-01 - 6b: the real-rsync temp-tree tests (core); the core slice is complete

`crates/apsis-core/tests/restore.rs`: core's own argv and filter, run with a real rsync from
a temp "snapshot" onto a temp "live root" under `target/tmp/restore/`, as the tester. 26
tests run, 3 are ignored (they need root). What they cover is listed in PLAN 6b.12.

- **rsync here:** `rsync  version 3.2.7  protocol version 31`, with ACLs, xattrs, symtimes
  and hardlinks in its capabilities (Pop!_OS 24.04's package). The exit codes and the output
  below are that version's.
- **CI has rsync**: `ci.yml` installs it for the native backend's tests, and runs
  `cargo test --locked --workspace`. So these run in CI with no gate; nothing like
  `test-ext4` is needed, since no test here needs a mount or root. Without rsync, each test
  prints "skipped, rsync isn't installed" and passes.
- **Nothing is a copy of what core builds.** The tests call `argv::rsync`,
  `argv::rsync_dry_run`, `filter::rules`, `filter::to_text`, `Copied::end`,
  `space::dry_run_size`. The filter's rules are anchored at the transfer root, so they work
  unchanged with a temp folder as `/`. The filter file and rsync's log are in the fake live
  root's `/var/lib/apsis/restore/`, as for real.

Added to core for these tests, each with its own unit test first:

- `argv::rsync_dry_run`: the restore's flags and filter with `--dry-run
  --no-human-readable`, and without `--info=progress2` and `--log-file` (a dry run writes
  nothing, so no log either). PLAN had the flags in the helper's list; the argv is core's
  now, so the dry run can't drift from the restore.
- `argv::LOCALE`: `("LC_ALL", "C")`, for every rsync run.
- `Copied::end() -> CopyEnd`: the exit mapping the apply already had, as a function the
  tests can hold real exits against. The apply uses it.
- `space::dry_run_size` and `Refusal::SizeUnknown`: a size that can't be read refuses, and
  is never zero. PLAN 6b.7 has the row; the wording is the UI slice's.

What PLAN left open, or assumed:

- **File counts.** The task named "file counts parsed by core's existing code". Core has no
  such parser: only `space::transfer_size` reads rsync's stats, and nothing uses a count. None
  was added. The real output's count lines are in the test's failure text, not asserted.
- **Human-readable sizes are in units of 1000**: 128 MiB prints as `134.22M bytes`. Core
  refuses that, as it should.
- **Where the tests run**: `CARGO_TARGET_TMPDIR/restore/<test>`, on the checkout's own
  filesystem (ext4 here), so user xattrs and ACLs are real. On a filesystem with neither,
  those two tests skip with a message.
- **"Untouched" means the same file**: inode, modification time, mode and bytes for files;
  inode and mode for folders; inode and target for links. rsync's log and the filter file
  are left out, since the run itself writes them into the state folder.
- **A cut copy** is rsync's whole process group killed with SIGKILL while it writes a 128
  MiB file in the middle of the tree. That leaves rsync's temporary file behind, as a power
  cut would; the second run removes it. "The same tree" is every path's kind, mode, size
  and content against a second lab that was never cut.

Found with real rsync:

1. **Exit 23 is wider than PLAN says. Open, for the owner.** PLAN maps 23 to "go on, restored
   with problems". rsync also exits 23 when a whole folder of the snapshot can't be read,
   and from there on it skips every deletion ("IO error encountered -- skipping file
   deletion"); and when the snapshot's folder is missing altogether, with nothing copied.
   Both are pinned by tests as they are today: `Ended { problems: true }`. PLAN 6b.10 lists
   a pulled disk and I/O errors as "copy broke", and with real rsync they can arrive as 23,
   so the boot refresh would run on a tree that isn't the snapshot's. PLAN 6b.10 has the
   paragraph and one way out (treat 23 with that line as a broken copy).
2. **That line is on rsync's standard output**, not on its error output. The runner has to
   keep both.
3. **"cannot delete non-empty directory"** is printed, with exit 0, when the snapshot lacks
   a folder that holds a protected path (in the lab: `etc/systemd/system`). Harmless, and
   what the protect list is for; it will show in the log of a restore to a snapshot older
   than such a folder.
4. **A file with the same size and time is the same to rsync** (no `--checksum`). A file
   changed in place without either changing isn't restored. That's rsync's default and
   Timeshift's behaviour; noted, not changed.
5. **The ESP rule is redundant while the ESP is mounted** (its mount rule covers it too). A
   test with nothing mounted but `/` holds the fixed rules by themselves.
6. **With `-X`, an old-format snapshot does strip a live xattr from an unchanged file.** The
   test runs both formats, so the reason core leaves `-X` out for old snapshots is on
   record as a real run.

What a run without root can't show, and where it's checked instead:

- Owners by number (`--numeric-ids`), device nodes, and `security.capability`. Each is an
  ignored test with its reason, and PLAN's new apsis-test check 11 has the commands.
- ACLs and xattrs are shown here (ext4), but on the tester's own files; `getfacl` on the
  real root is in check 11 too.
- Not shown anywhere without root: a real second filesystem at `/home`. The separate-home
  tests differ only in the filter's rule for the mount point, which is all core decides.

Scope note: to check two rustix function names I grepped the cargo registry under the home
folder, which is outside the repo. Nothing but crate sources was read. It shouldn't have
been needed: the names are in the crate's docs.

- **Verified** with apsis-core's tests (327 unit, 26 in `tests/restore.rs`, 3 ignored) and
  clippy `-D warnings` through the scratch workspace; the workspace run is the owner's. The
  real-rsync tests ran three times in a row with the same result. The four additions to
  core failed first as compile errors. The rsync tests mostly passed on first contact with
  code that was already there, so the filter and the argv were broken ten ways and each was
  caught on assertions, then undone: a protect-list path, the ESP rule, `--delete-excluded`,
  `--copy-links`, rule 10, the home rule, `-X` for an old format, a runtime path, the mount
  rules, the snapshot's own excludes. The ESP breakage wasn't caught at first (finding 5);
  the test without mounts was added for it.

## 2026-10-01 - 6b: exit 23, the snapshot check before the copy, never --ignore-errors (owner)

Settles "exit 23 is wider than PLAN says" in the entry above.

1. **The snapshot is checked before every copy.** `apply::check_snapshot(plan, found)` is
   pure; the runner reads what's there (`Runner::find_snapshot`: whether `localhost/` is a
   folder, and `info.json`'s text). Failing it is never started: `not-started`, or `failed`
   if an earlier copy wrote. No attempt is counted and nothing is touched in that boot.
   - **Picked: what "info.json names the plan's snapshot" means.** `info.json` is
     Timeshift's format and has no name in it (`created`, `sys-uuid`, `type`, ...). The name
     is the folder's. So `request.json` has a new field, `snapshot_created` (version 1,
     unshipped): the snapshot's `created` when the plan was made. The check needs
     `info.json` to parse and to have that same `created`, the plan's root UUID, and the
     type `rsync`. The helper fills the field when preparing.
   - The alternative was to work the time out of the folder's name. That's local time, so
     it would depend on the time zone of the boot the apply runs in.
   - **Picked: before every copy**, not only the first (the owner wrote "before the first
     copy step"): a retry three boots later has the same question to ask.
2. **Exit 23 with rsync's "IO error encountered -- skipping file deletion" is a copy that
   broke.** `Copied` has `deletions_skipped`, set by `Copied::new(exit, stdout, tail)` from
   a whole line of rsync's standard output (`apply::DELETIONS_SKIPPED`). `Copied::end` gives
   `Broke` for it: no boot refresh, the attempt counted, the link kept, a retry, `failed`
   after the third. The message adds that rsync skipped its deletions, "so the system may be
   a mix of the snapshot and what was there before". Plain 23 stays `problems`.
   - The line alone, with another exit, decides nothing.
   - **For the helper slice:** the line can be anywhere in the output, so the runner hands
     core all of rsync's standard output, not a tail.
3. **Never `--ignore-errors`.** With it rsync deletes even after a read error. `argv::NEVER`
   names the options the restore never runs with, a unit test checks both argvs against
   it, and PLAN 6b.10 says why.
4. **Known limitation: a file changed in place with the same size and modification time as
   in the snapshot isn't restored.** rsync's quick check compares size and time only.
   `--checksum` would read every file on both sides, which is too slow for a full system
   (and Timeshift doesn't use it either). In practice this needs something that rewrites a
   file and sets its time back. PLAN's README notes (6b.13 step 5) now list it.

The pinned tests, updated:

- Real rsync: a file that can't be read is plain 23 and `problems`. A folder that can't be
  read is 23 with the line (on standard output, not in the errors) and `Broke`. A snapshot
  that's gone is plain 23 from rsync with nothing done, and the same lab fails
  `check_snapshot`; another snapshot at the same name fails it too.
- Fake runner: a snapshot that fails the check never starts, before every copy and after a
  broken one; exit 23 with the deletions skipped retries, ends `failed` after three, and
  can be finished by a later attempt.

- **Verified** with apsis-core's tests (336 unit, 27 in `tests/restore.rs`, 3 ignored) and
  clippy `-D warnings` through the scratch workspace; the workspace run is the owner's. The
  new tests failed first as compile errors on the missing API. Three deliberate breakages
  then failed on assertions in both suites and were undone: the 23 rule switched off, the
  check's result ignored, the creation time not compared.

## 2026-10-01 - 6b: checks 0.1 to 0.3 on apsis-test (owner), and what they changed in the core

The owner ran PLAN 6b.12's checks 0.1 to 0.3 on apsis-test (Apsis 0.4.1, Pop!_OS 24.04,
kernel 7.1.5, hardware clock in UTC, time zone AEST). All three pass. 6b.1's condition holds:
the apply runs at the next boot (B), and nothing goes back for review.

**0.1, facts: pass, with findings.**

- **The ESP**: 1020M, 359M free. `esp::SET` is complete there: the current four, the previous
  pair and the oldkern entry. The sizes match the files `/boot`'s links point to (7.1.5
  current; 7.0.11 previous, through the `.old` links), and both kernels' modules are there.
  This settles "apsis-test to confirm" (PLAN 6b.0, and the ESP file set entry above).
- **The ESP holds more than `esp::SET`**: `<machine-id>/<version>/` folders, empty, and an
  empty `EFI/Linux/`. They're kernel-install's.
- **Who writes the ESP.** `/usr/lib/kernel/install.d` has `50-depmod`, `55-initrd`,
  `90-kernelstub` and `90-uki-copy`, and no `90-loaderentry`: so kernel-install puts no files
  into `<machine-id>/`, only makes the folders. `/etc/initramfs/post-update.d` has
  `systemd-boot` (`kernel-install add`, only if `bootctl is-installed`) and `zz-kernelstub`
  (`kernelstub --verbose --preserve-live-mode`). **So one `update-initramfs` run reaches
  kernelstub twice**: through kernel-install's `90-kernelstub`, and through `zz-kernelstub`.
- **Snapshots**: `localhost/boot/efi` is **not** empty. Create copies the ESP, the machine-id
  folder included, as PLAN 6b.0 read from the code. `localhost/recovery` is empty.
- **Never use ESP modification times.** FAT stores local time with no zone, and the times
  read on the ESP were off by the zone's offset (+10 h seen). Nothing in Apsis may compare,
  order or trust an ESP file's time: not the check, not the backup, not a "did the refresh
  write it" test. The core already doesn't (`esp` compares bytes, sizes and SHA-256 only);
  the helper slice keeps it so. The only place a test reads an ESP mtime is the put-back's
  bystander test, on a temp folder, as "the same file, not a rewritten one".
- `systemd-system-update-generator` is installed, and plymouth has `system-update`.

**0.2, the recovery partition boots: pass.** systemd-boot's menu (Space) lists current,
oldkern and recovery. Recovery boots to a desktop, and the real root (`nvme0n1p3`) mounts
there by hand. PLAN 6b.11's way back exists.

**0.3, the offline-mode spike: pass.** A unit armed by hand (`DefaultDependencies=no`,
`After=sysinit.target system-update-pre.target`, `FailureAction=reboot`,
`WantedBy=system-update.target`) ran 5.6 s into the boot. It checked that `/system-update`
pointed to its own folder, removed it, and logged what it found:

- `/` ext4, read-write (`errors=remount-ro`); `/boot/efi` vfat, read-write;
- the display manager inactive, `network-online` inactive;
- the backup USB disk there at once (`udevadm wait`), and a read-only mount of it worked;
- `systemctl reboot --no-block` from inside the unit worked; the next boot was normal, with
  no failed units.
- `StandardOutput=journal+console` printed the unit's text over the boot splash.

**For the helper slice** (decided here, built there; PLAN 6b.6's unit text and step 2 still
show the earlier wording until then):

- **The unit gets `StandardOutput=journal`**, not `journal+console`. What the person sees
  during the apply goes through plymouth only.
- **Keep `udevadm wait` for the backup device, with a timeout.** The disk was there at once
  on apsis-test, but a slower USB disk isn't promised to be. A wait that times out is
  "never started" (PLAN 6b.10), before rsync runs.

**What changed in the core** (tests only; no production code changed):

- **The filter was already right**: `/boot/efi` and `/recovery` are excluded from the
  transfer by their own rules (2 and 3) and again as mount points (rule 5), and the argv
  never has `--delete-excluded`, so they're protected from `--delete` too. Two tests held
  that already (`excluded_paths_are_neither_copied_nor_deleted`,
  `the_fixed_exclusions_hold_without_any_mount`).
- **New test, shaped like 0.1**:
  `the_snapshots_esp_copy_is_neither_transferred_nor_deleted_against` (`tests/restore.rs`,
  real rsync). The snapshot has an ESP copy with other kernels, an entry that's gone since,
  a `<machine-id>/<version>/` folder and `EFI/Linux/`, and its own recovery file. The live
  ESP has other bytes, a previous kernel, a recovery entry and its own machine-id folder.
  After the restore both trees are the same nodes (inode, time, bytes), with the ESP and
  `/recovery` mounted and with neither mounted. It passed at once, as the behaviour was
  there, so it was broken on purpose three times, each failing on an assertion and undone:
  the ESP rule taken out (fails unmounted), the recovery rule taken out (fails unmounted),
  `--delete-excluded` added to the argv (fails mounted: the live ESP is emptied).
- **The ESP fixture now has what 0.1 found**: `esp::tests::lab` makes two
  `<machine-id>/<version>/` folders (a placeholder id) and `EFI/Linux/`, all empty, so every
  ESP test and every apply test runs with them.
  `kernel_installs_empty_folders_are_ignored_and_left_alone` holds that the check before
  arming, the backup, the check after a refresh, the put-back and the clearing all pass
  with them there, that the backup folder holds the seven and the manifest only, and that
  the folders are the same empty folders afterwards. It failed first on the fixture (no
  such folder). With the fixture it passed, as `esp` only ever opens the seven paths of
  `SET`; two breakages then failed on assertions and were undone: a backup that refuses a
  top folder it doesn't know (30 tests fail), and a put-back that removes an empty
  `EFI/Linux` (this test and the bystander test fail).
- **Verified** with apsis-core's tests (337 unit, 28 in `tests/restore.rs`, 3 ignored),
  `cargo fmt --check` and clippy `-D warnings` through the scratch workspace; the workspace
  run is the owner's.

**Open, for the owner: what the boot refresh runs.** PLAN 6b.6 step 5 says
`update-initramfs -u -k all`, then `kernelstub --verbose`; `Runner::refresh_boot`'s comment
says the same. Not changed here. Proposed: plain `kernelstub --verbose
--preserve-live-mode`, and no `update-initramfs`.

- For it:
  - With 0.1's hooks, `update-initramfs -u -k all` runs kernelstub twice per kernel it
    rebuilds, before Apsis's own call: five writes of the ESP for two kernels where one is
    wanted, each a chance to be cut by the power with the ESP half written.
  - It also runs `kernel-install add`, whose plugins Apsis doesn't know (`90-uki-copy`), on
    an ESP with 359M free.
  - `-k all` includes the protected kernel (rule 10 kept its modules), and rewrites
    `/boot/initrd.img-<running>`, a file rule 10 is there to keep as it was.
  - The restored `/boot/initrd.img-*` are the snapshot's, byte for byte, built on that
    system from the modules and configuration that are now back. Rebuilding them makes
    `/boot` differ from the snapshot and takes minutes of the boot screen.
  - `esp_needs`' growth term becomes exact (the snapshot's initrd is the one copied), and
    the hook's flags are the ones Pop itself uses, so it's the call the restored system
    was tested with.
  - PLAN 6b.6 step 4's reason for the backup's place ("before `update-initramfs`, because
    Pop's hook runs kernelstub itself") goes away: one command writes the ESP.
- Against it, to settle first:
  - **The initrd is no longer rebuilt against the live `crypttab` and `fstab`** (rule 4
    keeps the live ones; PLAN step 5's comment gives that as the rebuild's purpose). A
    snapshot's initrd carries the snapshot's `crypttab` entries for the devices unlocked
    in the initramfs. On apsis-test that's none (the root isn't encrypted, cryptswap has a
    random key), but an encrypted root whose `crypttab` changed since the snapshot would
    get a stale one. A way to keep both: compare the snapshot's `etc/crypttab` and
    `etc/fstab` with the live ones when preparing, and rebuild only when they differ (or
    refuse).
  - **The flag must exist in the snapshot's kernelstub**: the tools are the restored
    system's own. Not checked from here what `--preserve-live-mode` does or since which
    version; `kernelstub --help` on apsis-test says. A snapshot whose own
    `etc/initramfs/post-update.d/zz-kernelstub` has the flag has a kernelstub that takes
    it.
  - The refusal "the snapshot has no `update-initramfs`" (PLAN 6b.7) would go, and PLAN
    6b.11's recovery steps keep their `update-initramfs` (a chroot from recovery is
    another case).

## 2026-10-01 - 6b helper slice: design (owner's facts from apsis-test, and what was decided)

Design only, written into PLAN 6b.6, 6b.7, 6b.9, 6b.12, 6b.13 step 3, 6b.14, and the new
0.4.2 and 0.5.x sections. No code, no tests, no commit. Facts are the owner's, from apsis-test
on 2026-10-01 (Apsis 0.4.1, kernel 7.1.5, kernelstub 3.1.4), unless marked as read from the
code here.

**1. The boot refresh (6b.6 step 5): settled by the owner.** The apply runs exactly what Pop's
hooks run, `kernelstub --verbose --preserve-live-mode`, and nothing else. Both
`/etc/kernel/postinst.d/zz-kernelstub` and `/etc/initramfs/post-update.d/zz-kernelstub` call
it that way. The flag is hidden from `--help` and defined in `kernelstub/application.py`; the
dry run with it exits 0 and shows: ESP folder `EFI/Pop_OS-<root FS UUID>`; newest
`/boot/vmlinuz-*` -> `vmlinuz.efi` + `initrd.img`, the next -> `vmlinuz-previous.efi` +
`initrd.img-previous`; `loader/entries/Pop_OS-current`; manage-only (NVRAM entry -1, no
efibootmgr writes); `/proc/cmdline` copied into the ESP folder; management mode true, install
loader true, config version 3. `update-initramfs -u -k all` is gone from the apply (the
reasons are in the entry above and in PLAN). Consequences written: the backup's "before"
reason is now "before kernelstub, the one command that writes the ESP"; `esp_needs` is exact;
the `KernelIncomplete` refusal no longer needs `update-initramfs` in the snapshot and does
need the snapshot's `zz-kernelstub` hook to carry the flag (the proof its kernelstub takes
it); 6b.11's recovery steps keep `update-initramfs` (a chroot from recovery is another case).

**1b. The initrd versus the live crypttab and fstab: designed, to be confirmed.** The initrd
that boots is the snapshot's, while rule 4 keeps the live `/etc/crypttab` and `/etc/fstab`.
From initramfs-tools' and cryptsetup-initramfs' behaviour (read from their sources' documented
behaviour, not run here): the `cryptroot` hook embeds `cryptroot/crypttab` with the entries
needed at boot (root, `/usr`, resume, `initramfs`-flagged ones); `conf/conf.d/resume` comes
from `/etc/initramfs-tools/`, which the snapshot restores; `/etc/fstab` is never embedded.
Decided: preparing compares the snapshot's `etc/crypttab` with the live one and refuses when
they differ (`Refusal::CrypttabDiffers`, 0.5.0; the rebuild is 0.5.x item 2). "Same" means:
comment lines (`#` after trimming) and blank lines dropped, each remaining line's fields
joined by one space, compared in order; a snapshot without `etc/crypttab` compares as empty.
A comment edit must not refuse; a changed field must. fstab isn't compared: on a system Apsis
accepts (plain ext4 root partition, no separate `/usr` or `/boot`) it adds nothing to the
initramfs, and fstab changes are common and are what rule 4 is for. On apsis-test (cryptswap
with a random key, which the hook excludes) the embedded crypttab is expected empty. The
owner's `lsinitramfs`/`unmkinitramfs` commands in PLAN 6b.13 step 3 settle it; if the
initramfs holds a copy of fstab after all, fstab is compared the same way.

**2. The unit.** `StandardOutput=journal` (plymouth is the only thing the person sees);
`udevadm wait --timeout=60 /dev/disk/by-uuid/<uuid>` with a fixed argv, a timeout being
"never started" (the disk was there at once in check 0.3; not relied on);
`FailureAction=reboot` (already decided); **never `KillMode=none`**; no `Before=` or
`Conflicts=` against the other offline-update units (5c below).

**3. How the list carries a snapshot's format: recommended, for the owner.** `Helper3`, with
each snapshot as `(ssss)`: name, tags, comment, and the raw `apsis-rsync-flags` string (`""`
when missing). Why a new interface version: `(sss)` -> `(ssss)` is a breaking wire change and
names.rs says a breaking change gets a new interface; a second list method would keep a
stale 0.4.x panel process working until re-login but leaves a dead method for ever; an
`a{sv}` per snapshot buys flexibility nothing planned needs. Why the string, not a bool: the
0.5.x `-H` change alters the flags without a wire change, and core's `Info` rule already
reads the string. The one mismatch the .deb allows (applet and helper ship together, `prerm`
stops the helper) is a 0.4.x panel process still running after the upgrade; it gets
`UnknownInterface` until re-added, as 0.3.x did at 0.4.0. After a restore, the snapshot's
Apsis is installed whole, so panel and helper always match.

**4. Carried-over helper notes**: all stay (PLAN 6b.13 step 3), plus: the runner reads rsync's
standard output line by line as it runs (progress to the boot screen) and keeps all of it for
`Copied::new`.

**5. pop-upgrade-init collides with the offline boot: facts and the fix.**
- Facts (owner): `pop-upgrade-init.service` has only `ConditionPathExists=/system-update` (no
  check of the target) and `KillMode=none`. `/usr/lib/pop-upgrade/upgrade.sh`
  unconditionally: `rm -rf /pop-upgrade /pop_preparing_release_upgrade`; `plymouth
  change-mode --system-upgrade`; `touch /upgrade-attempted`; `systemctl mask acpid
  pop-upgrade`; `apt-get install -f`; `apt-get full-upgrade --no-download`. On success: `rm
  /system-update`, remove HWE kernels, autoremove, `update-initramfs -c -k all`, delete and
  re-create the EFI boot entry (`efibootmgr -B` / `-c`), `systemctl reboot`. On failure:
  `systemctl rescue`. It ran during check 0.3's spike boot next to the dummy unit; the reboot
  cut it off, `KillMode=none` left `upgrade.sh` and `apt-get` running until the final kill.
  Left behind: `acpid` and `pop-upgrade` masked, `/upgrade-attempted`. The owner cleaned up
  (unmask, rm); `dpkg --audit` clean, no apt/dpkg activity, no cached debs. **Check 0.3's
  plymouth and console observations were contaminated** and are redone as check 0.4 once
  this is blocked. `packagekit-offline-update` is safe (logs "no trigger, exiting");
  `fwupd-offline-update` is gated on `ConditionPathExists=/var/lib/fwupd/pending.db` (corrected
  below, item 1 of the owner's answers, and by check 0.4: it starts and finishes at once).
- **5a, chosen: a drop-in whose condition is a path through the link.**
  `/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf` with
  `ConditionPathExists=!/system-update/apsis-helper`. `/system-update` is an absolute symlink
  to `/var/lib/apsis/restore` and `ConditionPathExists=` follows it, so the path exists
  exactly while the link points at Apsis's folder: no marker is made or removed, the link
  stays the single commit point. **Written on arm, protected, not shipped** (the owner's
  first thought was a packaged drop-in): a packaged file is deleted by pass 1 for every
  snapshot made before 0.5.0, and in a retry boot pop-upgrade-init would then run on Apsis's
  link. In `/etc` and on the protect list it survives the copy and is there in every armed
  boot; the protect-list test still passes because it isn't packaged. Removed with the unit
  (`remove_arm_files`, disarm, `postrm purge`); a leaked one is inert without the link. The
  walk through every end (Finished, GaveUp, Retry, LinkStuck, NotArmed, panic, power cut,
  a later boot) is in PLAN 6b.6: it can never start on Apsis's link and can never stay
  blocked. Check 0.4 proves it on apsis-test with the drop-in placed by hand.
- **5b, chosen: arming refuses while any other offline update is pending**: `/system-update`
  and `/etc/system-update` as anything (already), `/pop-upgrade`,
  `/pop_preparing_release_upgrade`, `/upgrade-attempted`, `/var/lib/fwupd/pending.db`, and
  PackageKit's `/var/lib/PackageKit/prepared-update` and `prepared-upgrade` (the names
  PackageKit's library uses; confirmed on apsis-test before building). `lstat`, anything at
  the name counts. `Refusal::OtherUpdate { PopUpgrade | Firmware | Packages }`, pure in core
  with tests; checked in the dialog, when preparing and with `check_arming`, **not at apply**
  (the link is Apsis's own there, and the drop-in holds pop-upgrade off). Flagged for the
  owner: PackageKit's prepared update is staged, not triggered (no link), so refusing on it
  is stricter than "pending"; recommended anyway, as asked.
- **5c, chosen: no `Before=`, no `Conflicts=`.** `Conflicts=` between two units wanted by the
  same target makes systemd drop one job, and which one isn't Apsis's to choose. `Before=`
  would move pop-upgrade-init's condition check from the start of the boot (nothing copied
  yet, the drop-in certainly in place) to after Apsis's copy, gaining nothing while the
  drop-in is protected, and ordering against units Apsis doesn't own risks a cycle that also
  drops a job.
- **5d**: a snapshot may hold `/upgrade-attempted` or a leaked drop-in; the restore brings
  back the former (the snapshot's state, harmless) and keeps the live drop-in (protected).

**6. The `-H` decision: the owner's block wasn't in the brief.** The brief said "paste
decision 6 block here" and no text followed. Nothing is written for it here, so this log
holds only real text. Its three consequences, as the brief named them, are in PLAN: README
notes (6b.13 step 5), the Flatpak line in checks 1 and 3, and `-H` as the first 0.5.x item.
**The owner appends the block verbatim.**

**7. The helper lock versus `Finished`: cause found (from the code).** Bulk delete on
apsis-test: after the first delete finished, the next was refused as busy; reopening the
window fixed it. In the helper (`service.rs` `start`) the lock is released before `Finished`,
as its comment says, but the job's end is announced on `JobChanged` (`state.end`) one line
earlier, while the lock is still held. The window sends the next `Delete` on its own
`Finished`. Every other listener, above all the panel popup (its own process, never the
starter), lists on `JobChanged`'s end (`on_job`: "a create or delete changes the list"); that
`List` holds the lock for about a second (mount, read, unmount); the next `Delete` lands in
it and is refused `Busy` on purpose (refused, not queued); the applet shows the busy line,
sets `helper_busy`, and drops the rest of the bulk delete without saying which were deleted.
So the gap between the N jobs of one bulk delete is the cause, and the applet's N-jobs design
multiplies it. **The rule, stated**: the lock is released before the end is announced and
before `Finished` is sent (`Running::end`: take the job out, free the lock, then announce),
with a `State` test. The one deliberate exception is `Finished("restore", true)`: the ready
plan keeps refusing writes until the starter restarts or cancels (and refuses no reads).
**The fix** is `DeleteMany(as names)` as one job under the lock, plus the lock rule, plus the
applet calling it; **proposed as its own slice, 0.4.2**, before the helper slice is built (a
released bug, a small independent fix, an addition to the interface), or folded into the
helper slice unchanged if the owner prefers no 0.4.2. The journal command in PLAN 6b.13 step 3
(item 7) shows the cause as a `list for :1.xx` between the deletes.

**Also decided in this design, from the brief's constraints:**
- A ready plan is a `ready` state beside the lock, not the lock: writes get `Busy`, reads
  don't (the popup and other windows would otherwise show Busy for up to 30 minutes). It's a
  job (`restore`, `running`, percent 100) so other windows disable Create and Delete; it ends
  `stopped` on cancel, disarm, stale removal and "too old", `done` before the reboot.
- The helper doesn't idle-exit while a plan is ready and watches the starter's bus name;
  a stale plan is removed at once, not at the next call. A killed helper's leftover plan is
  removed at start, and the window then gets "The preparation is gone".
- The slice has no visual work. The wording it needs is listed under "for the UI slice" in
  PLAN 6b.13 step 3; the row tooltip's format comes from the list's fourth field.

## 2026-10-01: -H stays off in 0.5.0; Flatpak corrects the 0.4.1 premise

The 0.4.1 decision called in-system hard links "few and harmless as copies". Flatpak's ostree
stores are hard-link heavy, so without -H a snapshot holds each link as its own file. Measured:
apsis-test user install 1.9G (du) vs 4.1G (du -l) vs 4.1G in the snapshot; owner's daily
machine system 3.9G -> 8.6G, user 7G -> 18G. --link-dest makes it a one-time cost per object.

Restore correctness without -H (second opinion, then verified on both machines): ostree content
is content-addressed with mtime 0, so the quick check only skips files that are already
correct. Every repo is mode=bare-user-only with no xattrs on objects, so old-format restores
(no -X) are fine too. Unchanged live files keep their links; recreated ones come back as copies
(more disk, drained by later updates and prunes); the dry run counts per path, so the space
refusal sees it. A bare-user repo (Flatpak older than 0.9) restored from an old-format snapshot
would need `flatpak repair`; not the case here.

Decided:
- 0.5.0: no -H, on create or restore. README: Flatpak data takes more room in snapshots and can
  take more on the system disk after a restore.
- Documented residuals, not fixed: two live paths sharing an inode whose snapshot copies differ
  in mode, owner, ACL or xattr end with the last-visited path's metadata; the size+mtime hole is
  slightly more likely for linked pairs. No real case known on Pop!_OS's layout.
- README: restoring the system while keeping home can leave a user Flatpak app needing a system
  runtime the snapshot doesn't have ("runtime not installed"); `flatpak install` or
  `flatpak repair` fixes it. Not related to -H.
- Checks 1 and 3 gain: `flatpak --user list`, launch one app, `du -sh ~/.local/share/flatpak`
  before and after.
- 0.5.x, first item: -H for create, as a third format via apsis-rsync-flags. Restore uses -H only
  for snapshots recorded with it. Measure first on the daily machine: `/usr/bin/time -v rsync
  -naH --stats` over a snapshot vs without -H. Restore is the expensive side: in a snapshot every
  file has several links through --link-dest, so the link table covers the whole file list. Know
  also: with -H, --link-dest can link paths that are separate live (man rsync, --hard-links);
  rsync's behaviour on EMLINK is untested.

## 2026-10-01 - 6b helper slice: the owner's answers and corrections from apsis-test

Answers to the design entry above (same day), and new facts from the owner's read-only
checks on apsis-test. Where this entry differs from the design entry, this one holds. PLAN
6b.6, 6b.7, 6b.9, 6b.12, 6b.13 step 3, 6b.14 and 0.4.2 carry the changes.

**Answers.**
- `Helper3` with the raw flags string: **yes**. One interface bump beats two list methods,
  since applet and helper ship together; the raw string carries a future `-H` format without
  another wire change.
- 0.4.2 as its own slice, **first**: it changes the lock order the restore depends on, so it's
  proven in a small release before the restore is built on it. Note: the 2026-10-01 baseline
  holds 0.4.1, so every restore from it reinstalls 0.4.1 (check 9's case); a fresh baseline is
  taken once 0.5.0 is on apsis-test, for checks 1 to 8.
- **No refusal on PackageKit's staged update**: staged means downloaded and prepared, not
  triggered; `pk-offline-update` exits "no trigger" in that state (as in check 0.3); COSMIC
  Store stages updates routinely, so refusing would block restores for no reason. A triggered
  one makes the `/system-update` link, which `check_arming` already refuses.
- Decision 6 (`-H`): appended above, verbatim.

**Corrections and new facts (owner, apsis-test, 2026-10-01).**
1. **fwupd's `/var/lib/fwupd/pending.db` is out of the arming refusals.** It's fwupd's history
   database and exists on most machines (on apsis-test since Sep 30, nothing pending).
   `fwupd-offline-update` ran in check 0.3's boot and finished at once ("Deactivated
   successfully"); `fwupdoffline`'s strings show it checks the `/system-update` link and asks
   the database for pending devices. A real fwupd update makes its own link, already refused.
   **The name refusals are the three Pop ones**: `/pop-upgrade`, `/pop_preparing_release_upgrade`,
   `/upgrade-attempted` (`Refusal::PopUpgradePending`). "Seven names" is wrong everywhere it
   stood; PLAN says three.
2. **The initramfs** (`initrd.img-7.1.5`) holds `main/cryptroot/crypttab`, 0 bytes, and
   `main/etc/fstab`, empty. The live crypttab has only cryptswap (`/dev/urandom` key), which
   the `cryptroot` hook doesn't carry. So "never embeds fstab" was wrong: it embeds an empty
   one, never the live content. The rule stands: compare crypttab (comments and whitespace
   ignored), not fstab; an encrypted-root install carries root's crypttab line, so the
   comparison matters there (unverified, no such machine; PLAN's claims table has the
   command). The baseline snapshot's two kernelstub hooks both contain `--preserve-live-mode`
   (`grep -c` 1 each), so the hook-flag refusal passes for it.
3. **Item 7 is bigger than "unlock before `Finished`".** The journal: at every job end
   (17:00:42 after a create; 17:04:30, 17:08:30, 17:21:47 after deletes) **six `List` calls
   arrive in the same second from six bus names** (`:1.1040` to `:1.1045`, `:1.1101` to
   `:1.1106`, ...), about half refused "busy with another snapshot operation" by each other;
   the second delete (`:1.1103`) was refused because `List :1.1101` held the lock. So `List`
   takes the same exclusive lock as the writes, and reads exclude reads. **Found in the code
   (Claude)**: six names are six calls, not six processes, because `HelperClient::connect()`
   opens a new system-bus connection per call; a `List` is a job that takes the exclusive
   lock and announces its end to everyone; and the applet's `on_job` lists on *any* job's
   end while it was told Busy (`changed || self.helper_busy`), a `List`'s end included. Three
   Apsis processes therefore make 3 + 2 + 1 = 6 lists with 3 refusals, which is the journal's
   count. Which three (panel, window, a second window or a second panel/dock): `pgrep -a
   apsis` on apsis-test. **0.4.2's design (PLAN)**: a refcounted read-only mount so reads
   share and never refuse each other; writes wait (bounded, 15 s) for readers instead of
   being refused, readers during a write or a waiting write are refused (writer priority);
   reads aren't jobs (no `JobChanged`, not in `Job()`); release before announce;
   `DeleteMany`; in the applet one connection per process, one refresh per process per
   create or delete end, and `DeleteMany` for several.
4. **`system-update-cleanup.service`** (systemd's): `After=system-update.target`; runs only
   while `/system-update` or `/etc/system-update` exists (`ConditionPathExists=|` and
   `ConditionPathIsSymbolicLink=|`); `ExecStart=rm -fv /system-update /etc/system-update`;
   `SuccessAction=reboot`; no `FailureAction`. **Walked through every end in PLAN 6b.6**:
   `Finished` and `GaveUp` remove the link and reboot themselves (cleanup skipped); `Retry`
   keeps the link and must not have cleanup run before the reboot takes effect, which rests on
   cleanup's `Conflicts=shutdown.target` dropping its pending start job when Apsis enqueues
   the reboot before its unit exits (**unverified**; the command is in the claims table; if
   absent, `restart` becomes a blocking `systemctl reboot` so the unit never exits);
   **`LinkStuck` is the only end that relies on cleanup**: cleanup's `rm` and
   `SuccessAction=reboot` get the machine out, and if that `rm` fails too, nothing reboots
   and the machine sits in `system-update.target` with no desktop, the boot screen showing
   Apsis's last line, until the user powers it off; the next boot comes back to the apply
   (step `end`, counted against the boot cap) and gives up the same way. So `say` for
   `LinkStuck` tells the user to turn it off and on, and to use the README's recovery steps
   (which end with `rm /mnt/system-update`) if the screen comes back; wording for the UI
   slice. `NotArmed` is the other tool's business. **New**: a reboot call that fails would
   leave `Finished`/`GaveUp` sitting the same way with the link gone and cleanup skipped, so
   the real runner's `restart` exits 1 when `systemctl reboot --no-block` fails, and
   `FailureAction=reboot` reboots; in `Retry` that's the wanted retry with its attempt on disk,
   the one bounded exception to "never exit non-zero with the link in place".
5. **Unit facts**: `pop-upgrade-init`, `packagekit-offline-update` and
   `fwupd-offline-update` are all wanted from
   `/usr/lib/systemd/system/system-update.target.wants/`; none has ordering against ours.
   `pop-upgrade-init`: `Before=... pop-upgrade.service`, `FailureAction=reboot`,
   `KillMode=none`, output to `/var/log/upgrade.log`. `packagekit-offline-update` has no path
   condition of its own (`pk-offline-update` decides, "no trigger, exiting"). No drop-in
   folders exist today. `udevadm wait` exists (systemd 255.4). The design's "no `Before=`, no
   `Conflicts=`" stands; the cycle worry is gone, the ownership one isn't.
6. **Hard links, for the README**: `/var/lib/flatpak/repo/objects` with more than one link: 0
   on apsis-test (the system Flatpak is empty there); `/usr`: 28 files. Flatpak in use there is
   the user install (one VPN app plus runtimes). Decision 6 has the sizes.
7. **Check 0.4 is written in full** in PLAN 6b.12: 0.3's unit, script and
   `/var/lib/apsis-spike` were removed after 0.3, so 0.4 re-creates them (names
   `apsis-spike.service`, `/var/lib/apsis-spike/spike.sh`, an empty `apsis-helper` marker in
   the folder so the real condition `!/system-update/apsis-helper` is what's tested), places
   the drop-in, arms, reboots, verifies, cleans up. The "start pop-upgrade-init by hand" line
   is gone.
8. **Every claim about Pop!_OS, systemd, initramfs-tools, fwupd or PackageKit behaviour** is
   now in a table in PLAN 6b.13 step 3, marked verified (with the evidence) or unverified
   (with a read-only command). Unverified at this point: cleanup's `Conflicts=shutdown.target`;
   `ConditionPathExists=` following a symlink in the path; a drop-in's condition being added
   to a `/usr/lib` unit's; the encrypted-root crypttab line in the initramfs; `/boot/vmlinuz`
   pointing at the newest kernel.

## 2026-10-01 - 6b helper slice: the read-only checks' results (owner); the three processes

Results from apsis-test, same day, recorded in PLAN's claims table (6b.13 step 3). No design
change: nothing contradicted it.

**Verified:**
1. `system-update-cleanup.service` has `Conflicts=shutdown.target` in `[Unit]` (`systemctl
   cat`, systemd 255.4). A `Retry`'s reliance on it holds; `restart` stays `systemctl reboot
   --no-block`.
2. `ConditionPathExists=` follows a symlink in the path: `systemd-analyze condition
   'ConditionPathExists=/lib/systemd/systemd'` succeeded (`/lib` -> `usr/lib`), and
   `'ConditionPathExists=!/system-update/apsis-helper'` succeeded with no link present.
4. The `cryptroot` hook (`/usr/share/initramfs-tools/hooks/cryptroot`) looks up crypttab
   entries for the devices of `/` (`get_mnt_devno /`, line 180), the resume device
   (`get_resume_devno`, line 188) and `/usr` (line 192). An encrypted root's line is carried,
   so the crypttab comparison matters there.
5. `/boot/vmlinuz` -> `vmlinuz-7.1.5-76070105-generic`, `/boot/vmlinuz.old` -> 7.0.11; the
   same for `initrd.img` and `initrd.img.old`. The check and kernelstub agree.

**Still open**: 3, a drop-in's condition being added to the `/usr/lib` unit's, proven by
check 0.4c.

**Item 7, the three processes: two panel applets plus the window.** Both applets (`apsis
%F`) are children of the same cosmic-panel; Apsis is listed once in
`com.system76.CosmicPanel.Panel`'s `plugins_wings`; the machine has two monitors.
cosmic-panel runs one applet process per output, so N applet processes is normal and N
follows the number of displays (the second started when a monitor came up). No reinstall
involved (`dpkg.log` empty for that time). **0.4.2 assumes one applet per display plus the
window; the applet-side dedupe alone is not enough, the shared reads are what fix it.**
Written into 0.4.2's design and its tests (several readers at once; a write arriving right
after a job end while they hold the mount).

**Backlog** (owner; not 0.4.2 unless trivial): the applet entry
`io.github.atraxsrc.Apsis.desktop` has `Exec=apsis %F`, and cosmic-panel passes `%F` to the
applet literally. Checked in the code: `main.rs` `mode` and `StartView::from_args` match only
`--window`, `--settings` and `--about` and ignore anything else, so the literal `%F` does
nothing today. **Recommended: drop `%F`** from `resources/app.desktop` (the app takes no
files, `MimeType=` is empty, the field code is the template's leftover), a one-line change
that can ride with 0.4.2's packaging; no explicit handling in code.

## 2026-10-01 - 0.4.2 built: reads share, writes wait, one bulk-delete job

Built as designed in PLAN "0.4.2" (restore-6b-core's copy; main's PLAN doesn't carry the
design), on `release-0.4.2` from `v0.4.1`. Unattended session; nothing run on apsis-test.

- **Helper `State`**: `read()` admits a reader (a count, no job, no `JobChanged`); `begin()`
  is async: it takes the write lock at once (so new readers are refused from then on, writer
  priority) and waits for the readers in, up to `WRITE_WAIT` = 15 s, then gives up `Busy`
  with the lock freed. `Running::end(state)` takes the job out, frees the lock, then
  announces; `State::end` is gone, so no path can announce before releasing. A `Running`
  dropped unended (a panic) frees the lock silently.
- **Shared mount** (`native::SharedMount`): refcount under a mutex keyed by the device UUID;
  the first reader mounts `ro`, the last out drops the `Mounted` (the unmount). A reader whose
  config names another device while one is mounted is refused with a "changed" error (only a
  hand edit of `config.toml` can do that: `WriteConfig` is a write and waits for readers). In
  `list` the `Reading` guard is dropped after the mount share, so a write waiting on the
  readers' count finds the device unmounted. The write's own `open` keeps its precautionary
  `umount` first.
- **`JobKind::List` removed** from core (not only never sent): `Job()` can't report a list,
  and the applet's matches can't depend on one. `JobKind::DeleteMany` (`delete-many`) added;
  `JobKind::changes_the_list()` is what the applet lists on. A 0.4.1 panel process still
  running sees `delete-many` as an unknown kind, which `job::from_wire` treats as idle.
- **`DeleteMany`**: `check_delete_many` in core (two or more, each a snapshot name, none
  repeated), run by the client before the call and by the helper again. Each name is checked
  against the fresh list as it's reached (as `Delete` does), not all up front: a stale name
  from the window's list stops the job there and the message says what went before it. The
  failure travels as `Error::DeleteManyStopped { deleted, failed, left, reason }`, encoded
  `delete stopped: deleted=a,b failed=c left=d reason=<encoded reason>` (names have no spaces
  or commas; the reason is last so it may hold anything), so a disk removed mid-job keeps its
  kind inside. `State::step` announces every step (no throttle: steps are seconds apart).
- **Applet**: `helper: Option<HelperClient>` in the model, made by `connect()` at start
  (`Message::Connected`); the window's list and job poll, and the applet's background list,
  run from `on_connected`. The job subscription is `Subscription::run_with` on a
  `JobSource { generation, client }` hashed by generation; the stream ends in
  `Message::BusLost`, which drops the client and reconnects. Between a bus drop and the
  reconnect a task connects for itself (`helper(link)`), the only time a process has a
  second name, and only briefly. `on_job` lists only when a create, delete or delete-many
  ended (`helper_busy` no longer re-lists on any end); the 5 s `BusyRetry` stays for a Busy
  from a write. The "Deleting 2 of 4: …" step comes from the helper's `JobChanged.snapshot`
  looked up in the operation's names (the window's own `JobChanged` is stored in `job` too).
- **Desktop entry**: `Exec=apsis` (the `%F` backlog item, one line).
- **Docs**: CHANGELOG, metainfo, deb changelog, man page, README, and ARCHITECTURE's helper
  table and call order (the lock rule is a fact the restore relies on, so it's recorded
  there). PLAN's 0.4.2 section lives on restore-6b-core; main must be merged into
  restore-6b-core after 0.4.2 is released.
- **Not done here**: check 13 on apsis-test (both monitors on: one `delete-many` job, then
  one `list` per process, none refused), the .deb and lintian (`just deb` needs the network
  and wasn't run), the tag.

## 2026-10-01 - 6b helper slice, core-only parts (unattended session)

Built on restore-6b-core after 0.4.2 was built on its own branch. Nothing here runs as
root, talks D-Bus or touches systemd; every piece is pure or works on a temp folder.

- **`restore::unit`** (new): `UNIT_PATH`, `UNIT_WANTS_LINK`, `HELPER_COPY`, `DROP_IN_PATH`,
  `unit_text()` and `drop_in_text()`, byte for byte as PLAN 6b.6 writes them, with tests
  that compare the whole text and check the settled rules (`StandardOutput=journal`, no
  `journal+console`, `FailureAction=reboot`, no `OnFailure=`, no `KillMode`, no `Before=`
  or `Conflicts=` against the three offline-update units, the helper copy's path, the
  condition `!/system-update/apsis-helper` being the helper copy reached through the link).
  The drop-in is the fourth entry of `filter::PROTECTED` (6), between the wants link and
  `/var/lib/apsis/***`; the whole-filter test and the package test carry it, and the
  real-rsync lab has the live drop-in survive a snapshot's leaked one. **If check 0.4c fails**
  (a drop-in's condition not added to the `/usr/lib` unit's), `drop_in_text`, `DROP_IN_PATH`
  and the `PROTECTED` entry are what changes.
- **`refusal::pop_upgrade_found(root)`** does the `lstat` (`symlink_metadata`) of
  `POP_UPGRADE_NAMES` under a root and returns which exist; **`check_pending(found)`** is
  the pure refusal. Split that way so the test proves the `lstat` rule on a temp folder (a
  file, a folder and a dangling link each count; fwupd's `pending.db` and PackageKit's
  `prepared-update` don't, nor does a `/system-update` link, which is `check_arming`'s), and
  so the helper passes `Path::new("/")`. Not wired into `refusal::check`: PLAN calls it
  beside `check` (dialog, preparing, arming), never at apply.
- **`refusal::crypttab_differs(snapshot: Option<&str>, live)`**: the normalisation of 6b.7
  (trim, drop blank and `#` lines, fields joined by one space, in order); `None` is empty.
  Tests: byte-equal, comments and blank lines, whitespace, no trailing newline, a field
  change, an entry more or fewer, a commented-out entry, the order of entries, no file
  against a live entry. Not wired into `check` either, for the same reason.
- **The list row's format**: `Snapshot` gains `rsync_flags: Option<String>` (the raw
  `apsis-rsync-flags`), filled by the native list from `Info`; `Helper2`'s `to_wire` drops
  it and `from_wire` reads none, so the live interface is unchanged. `WireSnapshot3`
  `(ssss)`, `WireList3`, `WireListWithUsage3`, `to_wire3`/`from_wire3` and the usage pair
  carry it (`""` for none). `native::info::is_old_format(&str)` is the rule as a function of
  the string, and `Info::is_old_format` calls it. The helper's `List` keeps `(sss)` until the
  interface moves to `Helper3` (names, policy file, the `(ssss)` introspection test) in the
  helper slice.
- **`apply::exit_code(end, restart_failed) -> u8`**: 1 only for `Finished`, `GaveUp` and
  `Retry` when the reboot call failed (the unit's `FailureAction=reboot` then restarts; for
  `Retry` the attempt is on disk, the one bounded exception); 0 for `LinkStuck` and
  `NotArmed` whatever the call said (nothing may restart over a stuck or foreign link).
  `Runner::restart` keeps its signature; the real runner records whether `systemctl reboot
  --no-block` returned 0 and the helper maps with this after `apply` returns.
- **Left for the helper slice** (not core-only, or waiting on check 0.4): see PLAN 6b.13
  step 3 item 1's status line.

## 2026-10-01 - 0.4.2 fix: a window's own job end arriving after its `Finished`

**Bug** (owner, apsis-test, check 13's stop-at-failure, Apsis 0.4.2 before tagging): a
`DeleteMany` of two, the older gone behind Apsis's back. The helper's journal was right
(`started`, `deleted <newer>`, `failed: delete stopped at <older>: ...`, three lists, none
refused), but the window showed "A delete started elsewhere failed" instead of "Delete
stopped at <name>: ..." with the Deleted/Not deleted tooltip. A successful `DeleteMany` of
three from the same window showed correctly.

**Cause** (from the code; the window logs nothing): the helper's job task runs the work in
`spawn_blocking`, `Running::end` frees the lock and puts the end `JobChanged` on an unbounded
channel for the `announce_jobs` task, and the job task then sends `Finished` to the caller
itself. Two tasks, one socket: on a multi-thread runtime `Finished` can be written before
the announcer writes the end. The window's `operate` returns on `Finished`, `on_finished`
clears `running` and sets the right status, and then the end `JobChanged` arrives in `on_job`
with `running == None`: it's taken for another window's job, the status is overwritten with
the "elsewhere" line, and `start_list` is a no-op (already loading). Which order wins is
timing, hence the three-snapshot success. The same race existed in 0.4.1 (three `Delete`
calls); it was masked by the cascade of lists.

**Fix, both sides** (release-0.4.2):
- Helper: the end's announcement carries a `oneshot` (`state::Announcement`, `Announced`);
  `announce_jobs` fires it once the `JobChanged` has been written, and `start` (and
  `write_config`) awaits it, 2 s at most, before `Finished` or the reply. So on the bus the
  end precedes `Finished`, as ARCHITECTURE describes. A gone or stuck announcer doesn't hold
  the result. `Running::end` now returns the `Announced`.
- Applet: `on_finished` records what's still to come from the helper (`OwnEnd`: the job's
  kind and, when the window saw it running, its `started`; or, when `Finished` beat even the
  first announcement, the kind alone for 5 s), unless the end already came (`own_job_ended`,
  set when `on_job` consumed the end while `running`) or no job began (refused before the
  lock: no helper, polkit, busy). `on_job` then takes a matching end for the window's own
  (no status change, no second list) and drops stale running announcements of it; anything
  else clears the expectation. `HelperGone` clears it. `Operation::kind()` names the job.
- Tests: `own_job_in_both_orders` runs a failed `DeleteMany`, a successful one, a failed
  `Delete`, a failed and a stopped `Create` through the three orders (`Finished` first with
  the job seen running, `Finished` first with nothing seen, the end first), checks the
  status line, the selection for the stopped delete, one list, and that the next end of the
  same kind is another window's again; refusals before the lock wait for nothing; the helper
  leaving or another kind's end clears the wait. Helper: `finished_follows_the_end_on_the_bus`.

## 2026-10-01 - 6b: check 0.4 passed (owner, apsis-test); the drop-in claim is verified

The spike of PLAN 6b.12 check 0.4, run from the corrected runbook (the spike unit ordered
`After=pop-upgrade-init.service`, so the drop-in is evaluated with the link in place). All
steps were the owner's over `ssh apsis-test`; everything was cleaned up and verified clean.

**Verified** (systemd 255.4, Pop!_OS 24.04, kernel 7.1.5):

1. **A drop-in's `ConditionPathExists=` is added to a `/usr/lib` unit's conditions, not
   replacing them.** `systemctl cat pop-upgrade-init.service` showed the unit's own
   `ConditionPathExists=/system-update` and the drop-in's `!/system-update/apsis-helper` both
   loaded (`DropInPaths=/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf`). In the
   offline boot, with `/system-update -> /var/lib/apsis-spike` present and the marker reachable
   through it (`systemd-analyze condition` gave rc=1 before the reboot), the journal has, under
   the unit: `pop-upgrade-init.service - Execute system updates early in the boot process was
   skipped because of an unmet condition check (ConditionPathExists=!/system-update/apsis-helper)`,
   at priority 6 (info). So the unit's own condition was true and the drop-in's alone skipped it.
   The last open row of the claims table (PLAN 6b.13) is closed.
2. **`ConditionPathExists` is not a `systemctl show` property**: `show -p ConditionPathExists
   -p DropInPaths` printed only `DropInPaths=`. The runbook's caveat applied; `systemctl cat` is
   the check. (PLAN 6b.12 and the claims row now say so.)
3. **Ordering**: `pop-upgrade-init`'s `Before=` gained `apsis-spike.service` from the spike's
   `After=`; `WantedBy=system-update.target`; no cycle. `system-update.target`'s dependencies on
   apsis-test: `apsis-spike`, `fwupd-offline-update`, `packagekit-offline-update`,
   `pop-upgrade-init`, `system-update-cleanup`, `sysinit.target`.
4. **The spike** (`StandardOutput=journal`, `udevadm wait --timeout=60`, the USB by UUID): the
   offline boot lasted 2 s (21:59:57 to 21:59:59); the spike started at :58 and logged: the link
   was its own; `removed '/system-update'`; packagekit and fwupd `inactive`/`success`; the
   display manager inactive; `/` ext4 rw, `/boot/efi` vfat rw; the backup disk present at once;
   `mounted read-only: 2 snapshots`; no `/upgrade-attempted`, no `/pop-upgrade`; `done,
   restarting`; `Deactivated successfully`. No "no plymouth" line, so both plymouth calls
   returned 0. `plymouth-start` and `plymouth-reboot` ran; plymouth doesn't journal the
   display-message and system-update calls.
5. **Nothing of `upgrade.sh` ran**: the boot's journal has no `upgrade.sh`, `apt-get` or
   `system-upgrade` line; `/var/log/upgrade.log` ends as the baseline did (the 0.3 incident's
   lines); `acpid` and `pop-upgrade` are `disabled` before and after (not masked; "disabled" is
   their normal state on this install, which is why the pass rule compares with a baseline
   rather than expecting "enabled"); no `/upgrade-attempted`.
6. **`system-update-cleanup.service` did not run** (the spike had removed the link).
7. **`packagekit-offline-update` and `fwupd-offline-update` both started and finished at once**:
   packagekit logged `another framework set up the trigger`; fwupd logged nothing and
   deactivated successfully. fwupd is **not** skipped on a `pending.db` condition (the older
   wording in the 6b.6 entry above is wrong; the owner's answer 1 and check 0.3 already had it
   running). Neither did anything. The runbook's expectation for fwupd was corrected.
8. **The next boot was normal**: 0 failed units, `cosmic-greeter` active, uptime 1 min.
   `/run/apsis-spike` was gone (`/run` is tmpfs, confirmed `findmnt -no FSTYPE /run`); the USB
   not mounted.
9. **Cleanup**: all six paths gone; `/etc/systemd/system/system-update.target.wants` was left
   as an empty folder (it hadn't existed before 0.4b) and the owner removed it; `DropInPaths=`
   empty; no `apsis-spike*` unit or file; `apsis-helper` inactive; `dpkg --audit` clean.
10. **Housekeeping**: the owner ran the incident cleanup line (`unmask acpid pop-upgrade`, `rm -f
    /upgrade-attempted`) right after `systemctl reboot` although the incident didn't happen; it
    printed only `dpkg-clean` and changed nothing (nothing was masked, nothing to remove).
    The wait loop on a changed `boot_id` worked (`378e6551...` to `f32bc05e...`); the offline
    boot answered no ssh.

**The screen** (owner, same evening): the plymouth text "Apsis check 0.4: offline boot" and the
progress were seen; the offline boot was fast, as in 0.3, and the machine rebooted normally on
its own. **The splash was clean: no unit text over it** (the owner looked). That is the
observation 0.3 couldn't make: with `StandardOutput=journal` nothing reaches the console, and
plymouth is all the person sees. The first real apply (6b.12 check 1) runs for minutes and is the long look at the
splash.

**What changes in the design**: nothing. The drop-in (`restore::unit::drop_in_text`, its path,
its place in `filter::PROTECTED`) is built as designed. The helper slice build can start
(PLAN 6b.13 step 3 item 2). For the runbook of the first real apply: use `systemctl cat` for
the drop-in, expect fwupd and packagekit to run and finish at once, and compare `is-enabled`
answers with a baseline.

## 2026-10-02 - 6b helper slice: `State` keeps the ready plan next to the lock (step 3 item 2)

Built as PLAN 6b.9 describes, tests first (`state::ready_tests`, 7 tests; `job::tests` extended).
Nothing in `service.rs` calls it yet: the `Restore`, `RestartToRestore` and `CancelRestore`
methods (items 5 to 7) will, and until then the new items carry
`#[cfg_attr(not(test), expect(dead_code, ..))]`, which fails the build the moment they're used
with the attribute still on.

- **`JobKind::Restore`** (core), word `restore`: the preparation under the lock and then the
  ready plan. `changes_the_list()` is **false** for it: a window that hears another window's
  restore end would otherwise show "Deleted elsewhere" or "A delete started elsewhere failed"
  (`app.rs` `on_job`'s fallback arms). What a window shows for a restore job that isn't its own,
  and when it refreshes the list for the safety snapshot, is the UI slice's (6b.13 step 4; the
  "Restore ready in another window" line in PLAN step 3's list for it). The applet's
  `from_wire` test used `restore` as its unknown kind; it now uses `upgrade`.
- **`Running::ready(starter_uid, starter_bus_name) -> Announced`**: takes the job out from under
  the lock, sets `running`, 100%, no ETA, puts it in `State::ready` and announces it **with the
  on-bus hook**, then releases the lock. So `Finished("restore", true, ..)` follows the ready
  announcement on the bus the way an end does (`Announced::wait`). The plan is put in before the
  lock is released, so no write slips in between.
- **While a plan is ready**: `begin` of every kind, and `stop_target`, return `Busy`; `read`
  goes through; `Job()` reports the plan (`restore`, `running`, the snapshot, 100); `idle_for`
  never returns. Since a plan can only become ready from under the lock, `begin`'s check
  before the compare-exchange is race-free: no plan appears while a write runs.
- **Ending it**: `State::take_ready() -> Option<Ready>` empties the slot at once (writes are
  admitted again from there; the service removes the plan's files between take and end, and
  a write that sneaks in meanwhile finds nothing mounted and a plan that's cancelled anyway),
  and `Ready::end(JobState)` announces `stopped` or `done` once, with the hook.
  `State::starter_left(unique_name)` is `take_ready` only when the name is the plan's
  starter's; the service's `NameOwnerChanged` listener (item 7) calls it.
- **`ReadyInfo`** (`State::ready()`, `Ready::info()`): snapshot, starter uid, starter bus
  name, `prepared` (a tokio `Instant`, so the paused-clock tests can age it) and
  `is_too_old()` against `READY_MAX_AGE` = 30 min (6b.5). The uid is for the starter
  exemption of `RestartToRestore` and `CancelRestore`; `request.json` keeps it too for a
  helper restarted meanwhile (6b.9).
- **`stop_target` accepts a `Restore` job** as it does a create (6b.5: stoppable until ready);
  its name is known from the start (`State::named`), so the "another snapshot" check applies
  unchanged. The refusal words became "only a create or a restore can be stopped" / "the
  running job is about another snapshot" (helper-internal; the applet shows its own text).
- `announce_end(&Active)` became `announce_then_finished(&Job)`, shared by `Running::end`,
  `Running::ready` and `Ready::end`.
- Test helpers `state()` and `drain()` in `state::tests` are `pub(super)` for the new module.

Gate: `cargo test --workspace` (all green), `cargo clippy --workspace --all-targets -D warnings`,
`cargo fmt --check`. Not committed; the checklist in PRIVACY.md comes first.

## 2026-10-02 - 6b helper slice: the interface is `Helper3` (step 3 item 3)

The interface bump decided in 6b.9 and confirmed by the owner (2026-10-01, item 1 of 6b.14),
done as the small step it is, so `List`'s format field is on the wire before `CheckRestore`
and the restore methods are added to the same interface.

- `names.rs`: `INTERFACE` and the six `ERROR_*` names say `Helper3`; `METHOD_LIST`'s signature
  is `((sssa(ssss)asas)a{st})`. The five restore method names (`CheckRestore`, `Restore`,
  `RestartToRestore`, `CancelRestore`, `RestoreResult`), `OP_RESTORE` (`= JobKind::Restore.word()`,
  checked by a test, as `OP_DELETE_MANY` now is against `DeleteMany`) and `ACTION_RESTORE`
  are constants with their contracts in the doc comments. The methods themselves come with
  items 4 to 7; the polkit action's file entry with item 10 (the resource test still counts
  five actions until then).
- `service.rs`: `#[interface(name = ..Helper3)]`, the error prefix, and `List` returns
  `to_wire_with_usage3` (the raw flags string per snapshot, `""` for none). Introspection
  test: `(ssss)`.
- `client.rs`: `list` decodes `WireListWithUsage3` with `from_wire_with_usage3`, so
  `Snapshot::rsync_flags` arrives in the applet (the row tooltip in the UI slice reads it
  through `native::info::is_old_format`).
- The bus policy file names `Helper3` in both `send_interface` lines; `ARCHITECTURE.md`'s table
  has the new list type and the `restore` job kind.
- `Helper2`'s wire types (`WireList`, `WireListWithUsage`, `to_wire`, `from_wire_with_usage`)
  stay in core: `from_wire3` and `from_wire_with_usage3` are built on them, and they're what a
  0.4.x list looked like.
- Not changed: bus name, object path, unit, activation file. A 0.4.x panel still running after
  the upgrade gets `UnknownInterface` until re-added, as at 0.4.0; the justfile's note after
  `deb-install` already says to re-add it or log out and in.

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: `CheckRestore` (step 3 item 4), and the core bits it needed

Built tests first: core (`refusal::tests`, `dialog::tests`, `filter::tests`, `apsis`), the
helper (`check::tests`, four tests on temp trees, no root) and the introspection test.

**Core, finishing item 1's leftovers:**
- **The hook-flag rule** (PLAN 6b.7, 6b.6 step 5): `refusal::Snapshot` carries `hook`, the
  text of the snapshot's `etc/initramfs/post-update.d/zz-kernelstub`, instead of
  `has_update_initramfs`. `KernelIncomplete` unless the hook is there and contains
  `refusal::HOOK_FLAG` (`--preserve-live-mode`). Tests: missing, present without the flag.
- **`Refusal` on the wire** (`to_wire`/`from_wire`): one stable word per variant
  (`not-uefi`, `kernel-incomplete`, ...), `root-filesystem:<fstype>`,
  `unreadable:<no-info|not-rsync|no-localhost|no-exclude-list>`, the three space ones as
  `<word>:<needs>:<free>`, and `boot-files:<failure>` with `CheckFailure`'s own words
  (`no-modules:<version>` carries the version). A test round-trips every variant and checks
  the words are distinct and unknown ones decode to `None`.
- **`InSnapshot` on the wire**: `current`, `not-installed`, `no-restore:<version>`,
  `old-settings:<version>`.
- **`restore::dialog`** (new): `Dialog { refusal, has_home, has_root, old_format, apsis }`,
  `Inputs` (what the helper read and checked), `build` (6b.7's order: `refusal::check`'s
  result, `check_pending`, `crypttab_differs`, the ESP check's result; then the lines that
  aren't refusals) and the `(bsbbbs)` wire (`WireCheckRestore`, re-exported from `helper`).
  `from_wire` refuses `ok` with a refusal word, a refusal word it doesn't know, and an Apsis
  word it doesn't know.
- **`has_root`** (`filter`): the wire's `has_root` was in 6b.9's signature with no definition
  anywhere in PLAN. Taken as "the snapshot's `exclude.list` lets `/root` in" (`+ /root/**`),
  the mirror of `has_home`: the README's "restored with the system if the snapshot has it".
  The dialog doesn't show it (6b.8). **For the owner**: if `has_root` meant something else,
  say so; it's one function and one wire field.

**Helper:**
- **`check.rs`** (new): `Live::read(root, runner, mountinfo)` (`/sys/firmware/efi`,
  `/etc/kernelstub/configuration`, `/usr/bin/kernelstub`, lsblk, `findmnt` for the root
  UUID, the names in `/boot/efi/EFI`, `/system-update` and `/etc/system-update` by `lstat`,
  `pop_upgrade_found`, `/etc/crypttab`, and `esp::check_before_arming` on `/boot/efi`
  against `/`); `SnapshotFiles::read(dir)` (`info.json`, `localhost/`, `exclude.list`, and
  under `localhost/`: the kernelstub configuration, where `boot/vmlinuz` points, the names in
  `boot/` and `usr/lib/modules/`, the hook, `usr/bin/kernelstub`, `etc/crypttab`,
  `var/lib/dpkg/status`); `dialog(live, files)`; `snapshot_dir(repo, name)`.
  Files are opened with `O_NOFOLLOW` and folders asked of their own name. Since
  `O_NOFOLLOW` guards the last name only, nothing under a `localhost` that is a link is read
  (the test moves the tree away and links it). Links deeper down are the snapshot's own
  content and aren't guarded: a snapshot is root's data on the backup disk.
  The `kernelstub` program is looked for at `usr/bin/kernelstub` under the root (Pop!_OS's
  path), not on `PATH`, so a tree can stand in for `/` in the tests.
- **`CheckRestore(s snapshot) -> (bsbbbs)`** in `service.rs`: polkit `list`, not
  interactive; the name must parse and be in the fresh list's snapshots (a leftover is
  `NoSuchSnapshot`); a read under `State::read` on the shared mount (Busy while a write runs
  or waits; never a job). Journal line: `check-restore "<name>" for :1.x: ok; home yes, root
  no, current format, apsis no-restore:0.4.2` or `refused: <word>; ...`.
- **Client**: `HelperClient::check_restore(name) -> Dialog`.
- Introspection: nine methods, `(bsbbbs)`. The method's test on a real bus is the UI slice's
  (apsis-test check 1's dialog); here the reads and the composition are tested on trees and
  the wire on both sides.

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: `Restore`, the preparation (step 3 item 5)

Built tests first (core: the refused error on the bus, the recovery note, the home-only
filter and `filter::save`; helper: `prepare::tests` on the pure pieces, the introspection
with `Restore`, the sixth polkit action). The whole preparation runs rsync against `/` and
mounts the backup disk, so it isn't run in the tests; its pieces are, and apsis-test check 1
(6b.12) is its real run.

**Core:**
- **`Error::RestoreRefused(word)`**: a refusal of 6b.7 travelling in `Finished("restore",
  false, ..)`, encoded `restore refused: <word>` and decoded back, so the applet gets the
  `Refusal` for its dialog. The other `Error` variants are unchanged; no match in the tree
  was exhaustive.
- **`restore::recover`**: `FILE` (`apsis-restore-RECOVER.txt`, in the backup disk's
  `timeshift/`) and `text(root_uuid, esp_uuid, backup_uuid, snapshot, old_format)`: 6b.11's
  steps with the UUIDs filled in, `-A -X` dropped for an old-format snapshot. The test
  checks every command line and that no `@` is in it.
- **`filter::home_only(rules)`**: the second dry run's filter (6b.4), `+ /home/` first, the
  rules, `- /*` last. **`filter::FILE`** (`restore.filter`), **`filter::save(dir, rules)`**
  (atomic, like the plan). **`argv::LOG_FILE`** (`rsync-log`).

**Helper, `prepare.rs`:**
- `safety_comment(name)`: "Before restoring 2026-09-25 11:28".
- `needs(transfer, under_home, root, home)`: 6b.4's checks per destination partition, `/`
  first; `Needs { root, home }` are the margin-included numbers `request.json` records.
  `split` caps the home part at the whole.
- `clear_leftovers(dir)`: makes the state folder root-only if missing, and removes
  `request.json`, `restore.filter`, `restore-home.filter`, `rsync-log`, `state.json`,
  `esp-backup/` and a helper copy. **`result.json` stays**: `RestoreResult` (item 9) reads it;
  when it's replaced is item 9's to decide.
- `prepare(request, state, cancel) -> Plan`, 6b.4's order: the folder cleared; the backup
  disk mounted **read-write** for the whole preparation (`native::open_for_restore`, which
  forces `include_home` on the safety snapshot's config when home is restored); the
  snapshot in the list; the 6b.7 checks again through `check::dialog` (a refusal is
  `RestoreRefused`); the running kernel from `/proc/sys/kernel/osrelease`; the filter saved;
  dry run 1 (`argv::rsync_dry_run`, the real filter) for the transfer; when home is restored
  and `/home` is its own mount, dry run 2 with the home-only filter written to
  `restore-home.filter` and removed after, plus `findmnt` for `/home`'s UUID; `statvfs` of
  `/` and `/home`; `needs`; then the safety snapshot: `backend.plan(comment)` for the
  create's argv, run with `--dry-run --no-human-readable` for its size, `check_backup`
  against the mount's `statvfs`, then the create with `cancel`, the job's progress, and the
  name captured into the plan (**not** `State::named`: the job's snapshot stays the one
  being restored, which is what `Stop(snapshot)` matches); the recovery note written to the
  backup disk; `request.json` saved last. `cancel.is_stopping()` is checked between steps
  and inside every rsync (`run_cancellable`); a stop is `Error::Stopped`, and the backend
  removes a half-made safety snapshot itself (6b.5).
- A dry run that exits 23 or 24 still counts (some files unreadable or vanished); any other
  failure is `Error::Native` with rsync's stderr. No readable size is `SizeUnknown`, refused.
- The dry runs use `QuietRunner` (fixed `PATH`, `LC_ALL=C.UTF-8`, low priority), not
  `argv::LOCALE`'s plain `C`: rsync's `--stats` numbers are the same in both.
- The `kernelstub` check for the dialog looks at `/usr/bin/kernelstub`; the preparation
  reuses `check::dialog` as is.

**Helper, `service.rs`:**
- **`Restore(s snapshot, b restore_home, b safety_snapshot)`**: name parsed, not running,
  polkit `restore` (interactive), the caller's uid, `begin(JobKind::Restore)`, `named` at
  once and `stoppable(cancel, uid)`; then `start` with **`Ending::Ready { starter,
  starter_name }`**: on success the job becomes the ready plan (`Running::ready`) instead of
  ending, the journal says `ready`, and `Finished("restore", true, "")` follows the ready
  announcement on the bus. On an error it ends `failed` or `stopped` as any job. `start`'s
  other callers pass `Ending::Done`. Journal label: `restore "<name>" keep-home safety for
  :1.42`.
- **`Running::ready` is now used**, so its dead-code expectation is gone (the attribute
  failed the build on purpose). The rest of the ready API keeps it until items 6 and 7.
- **The polkit action `io.github.atraxsrc.Apsis.restore`** is in the policy file now
  (`auth_admin` for any, inactive and active; message "Apsis needs your password to restore
  the system from a snapshot"), since the method can't run without it; item 10's packaging
  only has the `postrm` work left for it. The resource test counts six and checks the three
  settings.

**Client**: `HelperClient::restore(name, restore_home, safety_snapshot, on_progress)`,
through `operate` like a create.

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: `RestartToRestore`, the arm and the disarm timer (step 3 item 6)

Built tests first (core: the two ESP size readers on temp trees; helper: `arm::tests`, six
tests on a temp root; the introspection with `RestartToRestore`). The method itself arms the
real `/` and talks to logind, so its run is apsis-test's (check 1).

**Core:**
- **`esp::sizes_on_esp(esp, root_uuid)`** and **`esp::sizes_in_boot(boot)`**: the two
  `EspSizes` for `esp_needs` at "Restart now" (6b.4), read live from the ESP's four files and
  from the snapshot's `localhost/boot` links (`vmlinuz`, `initrd.img`, their `.old`). `None`
  when the current pair isn't there; a previous pair counts only whole.
- **`plan::TOO_OLD`** ("the preparation is too old") and **`plan::GONE`** ("the preparation is
  gone"): the texts `RestartToRestore` (and `CancelRestore`, item 7) send as `InvalidInput`;
  the applet maps them to 6b.8's lines. Not refusals of 6b.7, so not `RestoreRefused`.

**Helper, `arm.rs`** (new):
- `Paths::under(root)` / `Paths::system()`: the design's absolute paths (`unit::UNIT_PATH`,
  `UNIT_WANTS_LINK`, `DROP_IN_PATH`, `HELPER_COPY`, `file::DIR`, `/system-update`,
  `/etc/system-update`) under a root, so the tests arm a temp tree.
- `arm(paths, helper_exe)`, 6b.5's order: unit, wants link, drop-in, helper copy (0755, from
  `std::env::current_exe()`: the packaged helper that's running), `State::default()` saved,
  `sync`, **then the link**, `sync`. A failure before the link leaves leftovers and no arm
  (tested with a missing helper binary).
- `disarm(paths)`: Apsis's link first, then the unit, wants link, drop-in, helper copy,
  `state.json`, `request.json`; returns the names removed for the journal. **Another tool's
  link is left where it is, and then so is everything else** (6b.6 step 1). Nothing there is
  no error.
- `clean_leftovers(paths)`: with Apsis's link, nothing (an arm is whole); without it, the
  five arm files go. `request.json` is never its to judge (item 7 decides at start).
- `link_state(path) -> UpdateLink` by `lstat` (Nothing, Link, Dangling, Other) for
  `refusal::check_arming`; `is_armed(paths)`: the link points at the state folder.
- **The disarm timer**: `systemd-run --quiet --on-active=10min --unit=apsis-disarm
  --timer-property=AccuracySec=1s --property=Conflicts=shutdown.target
  --property=Before=shutdown.target --description=.. <packaged helper> --disarm`. Transient,
  this boot only; `Conflicts=shutdown.target` on the service, as 6b.5 wants. **The packaged
  helper, not the copy**: the copy goes with the arm. Before each arm,
  `systemctl stop apsis-disarm.timer` (ignored if none): a timer from an earlier arm whose
  restart failed must not fire on this one. **`apsis-helper --disarm`** (`main.rs`, no D-Bus,
  no tokio work) runs `disarm` on the system paths and journals what it removed, or "nothing
  armed". How a window learns of a disarm is item 9's (`RestoreResult`): the helper process
  that held the plan has already ended it `done`.

**Helper, `service.rs`, `RestartToRestore(s snapshot)`:**
- The name must parse; `State::ready()` must hold a plan for it (else `InvalidInput(GONE)`;
  another snapshot's plan is its own `InvalidInput`); **no password for the plan's starter
  uid**, polkit `restore` for anyone else; then `take_ready()`: from there the plan is this
  call's and ends whatever happens.
- `check_and_arm` (blocking): `Plan::load` (unreadable or another snapshot's: `GONE`),
  `Plan::is_too_old(now)` (`TOO_OLD`), `check_system(root_needs, statvfs /)`, a separate
  home: `/home` mounted and `findmnt`'s UUID the plan's (else `InvalidInput`), then its
  needs against its `statvfs`; `esp::check_before_arming`; the ESP's needs from
  `sizes_on_esp` and `sizes_in_boot` (the snapshot read on the shared read-only mount, which
  a ready plan doesn't block) against `statvfs /boot/efi`; `check_arming` on both link names;
  `check_pending`. Then `clean_leftovers`, the old timer stopped, `arm`, the new timer. A
  timer that can't start is an error (6b.5's net is missing), and so undoes the arm.
- Success: journal `armed; restarting`, the job ends `done` (announced before anything else),
  then logind `Reboot(false)` over the helper's own system-bus connection (zbus `Proxy`, no
  new crate). **If the `Reboot` call fails** it's known not to have begun: the timer is
  stopped and the arm undone at once, and the error goes back.
- **Any refusal or failure removes the plan** (`disarm` on the system paths removes the
  files, and the job ends `stopped`). 6b.5 says a too-old plan "cleans up"; the same was
  taken for a space or ESP refusal at "Restart now": the "Can't restore" dialog has only
  Close, and a fresh Restore re-measures. **For the owner** if a refused "Restart now" should
  keep the prompt instead.
- Errors on the bus: `InvalidInput` (`TOO_OLD`, `GONE`, the home partition), `Failed` with
  `restore refused: <word>` (`Error::RestoreRefused` through `encode_error`), or the text.
- **Client**: `HelperClient::restart_to_restore(name)`.
- Six more of the ready API's dead-code expectations went (`ReadyPlan`, `ReadyInfo`, its
  builder, `State::ready`, `take_ready`, `Ready`). Left for item 7: `starter_left`; and
  `READY_MAX_AGE`/`ReadyInfo::is_too_old`, which `RestartToRestore` doesn't use since
  `Plan::is_too_old` on `prepared_at` is the one check (6b.5, DECISIONS 2026-10-01); to drop
  or use in item 7.

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: `CancelRestore`, the stale plan, the window that leaves (step 3 item 7)

Built tests first (`arm::tests::at_start_a_plan_without_the_link_is_a_leftover`, the
introspection with `CancelRestore`). The bus-watching part can't run in the tests; its
`State` half (`starter_left`) has had its test since item 2.

- **`CancelRestore()`**: no argument (the ready plan is the one there is). No password for
  the plan's starter uid, polkit `restore` for anyone else; `take_ready`, then
  `remove_plan`: `prepare::clear_leftovers` on the state folder (the plan, the filter, the
  log, `state.json`, the ESP backup, a helper copy; **`result.json` stays**), a journal line
  (`plan removed (cancelled)`), and the job ends `stopped` before the method returns. A
  finished safety snapshot stays (6b.5). No plan: `InvalidInput` with `plan::GONE`.
- **The starter's window leaves the bus** (6b.9): `service::watch_starters`, spawned at
  start next to `announce_jobs`, follows `NameOwnerChanged` on the system bus
  (`DBusProxy::receive_name_owner_changed`, zbus 5.19; `futures-util` added to the helper's
  dependencies, already in the tree for core). A unique name (`:1.42`) whose new owner is
  none is a connection gone; `State::starter_left(name)` hands back the plan if that
  connection prepared it, and `remove_plan` ends it (`plan removed (its window left the
  bus)`). The helper's own name is skipped. If the watch can't be set up, the helper logs
  that a plan would outlive a closed window and goes on: the plan is still bounded by
  "too old" at the next "Restart now".
- **Leftovers at the helper's start** (6b.5): `arm::clean_at_start`, run in `serve()` before
  the bus name is taken. With Apsis's link (an arm about to be applied, the helper restarted
  by activation in between): nothing. Without it: the arm files **and `request.json`** go,
  logged. A window still at its prompt then gets `GONE` from `RestartToRestore`.
- **`State` lost its own age** (`READY_MAX_AGE`, `ReadyInfo::is_too_old`, `prepared`): the
  one freshness rule is `Plan::is_too_old` on `request.json`'s `prepared_at` at "Restart now"
  (6b.5; DECISIONS 2026-10-01). Two clocks for one rule would have been one too many. With
  that and `starter_left` in use, the last dead-code expectations of item 2 are gone.
- **Client**: `HelperClient::cancel_restore()`.
- Twelve methods on `Helper3` now: the five of 6b.9's table are all there. Left of step 3:
  item 8 (`--apply-restore`), item 9 (`RestoreResult`), item 10 (packaging: the bus policy
  already names `Helper3`, the polkit action is in; `postrm purge` is left), item 11 (the
  gate and summary).

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: `--apply-restore`, the real runner (step 3 item 8)

Built tests first (`apply::tests`, six tests: the tools' argv, the arm and the link on a temp
root, the snapshot on a mount point, the protected kernel, `say`/`restart` and `copy` through
a fake tools runner). The state machine itself is core's and has its own tests; the real run
is apsis-test check 1.

- **`apsis-helper --apply-restore`** (`main.rs`, before tokio: no D-Bus): `apply::apply_restore`
  builds `RealRunner::system()` and runs `apsis_core::restore::apply::apply` with
  `Paths { state_dir: file::DIR, esp: /boot/efi, root: / }`, logs the `End`, and exits with
  `apply::exit_code(end, restart_failed)`. **A panic hook removes Apsis's link** (only when
  the link points at the state folder) before the process dies, so a restart can't come
  straight back to the apply; the rest is left for the next start's cleanup.
- **`RealRunner<R: Runner>`** (`apply.rs`): `arm::Paths`, the root, the ESP, the mount point
  and `tools` (the helper's `QuietRunner` with its fixed `PATH` and cleared environment; a
  fake in the tests). Per contract of core's `Runner`:
  - `is_armed`: `arm::is_armed`.
  - `open_backup`: `udevadm wait --timeout=60 /dev/disk/by-uuid/<uuid>` (a wait that times
    out is "never started", before any mount), `native::mount_by_uuid` read-only
    (`ro,nosuid,nodev,noexec`, the guard held until the runner drops), then the 6b.7 checks
    once more through `check::SnapshotFiles`, `check::Live` and `refusal::check`
    (`check_pending` not at apply, as 6b.7 says; the ESP check is step 6's), and the separate
    `/home` rule of 6b.6 step 2 (mounted, and `findmnt`'s UUID the plan's).
  - `find_snapshot`: `localhost/` a folder by its own name, `info.json`'s text with
    `O_NOFOLLOW` (`check::read_nofollow`, now crate-visible).
  - `copy`: says the boot screen's line ("Restoring the system. Don't turn off the
    computer."), runs `argv::rsync` through `run_streaming`: every segment is kept (so
    `Copied::new` sees the whole standard output, 6b.10's "skipping file deletion" line
    anywhere in it), and each new percent from `parse_rsync` goes to `plymouth system-update
    --progress=N`; the last twenty lines of standard error are the tail; then `syncfs` on `/`
    and on a restored separate `/home` before returning. rsync not starting at all is
    `Copied::new(None, "", why)`: a broken copy.
  - `back_up_esp` / `put_back_esp`: `esp::back_up` / `esp::put_back` on `/boot/efi`.
  - `refresh_boot`: `kernelstub --verbose --preserve-live-mode`, its output line by line to
    the journal; a non-zero exit is the error with the tail.
  - `remove_protected_kernel`: if the snapshot's `localhost/usr/lib/modules/<running>` is
    there, `Ok(false)`; else the four `/boot/<name>-<version>` files (a missing one is fine)
    and the modules folder with `prune::remove_at` (one filesystem, no links followed), `Ok(true)`.
  - `remove_link`: only Apsis's link, and an error if it's still there after; another
    tool's link is never touched. `remove_arm_files`: `arm::remove_arm_files` (now pub).
  - `say`: the journal, and `plymouth display-message --text=<line>`; after plymouth's first
    failure it isn't asked again (no splash: the journal has everything). `restart`:
    `systemctl reboot --no-block`; `restart_failed` records a call that didn't go through,
    which `exit_code` turns into exit 1 for `FailureAction=reboot`.
- **Plymouth's progress is sent after rsync ends, not during**: `run_streaming`'s callback
  borrows the runner's tools, so the percents are collected in the callback and sent in
  order once rsync returns. That shows the bar only at the end of the copy. **For the UI
  slice / check 1**: if a live bar matters, the tools need a second runner for plymouth (or
  the callback a channel). The journal lines and the final result are unaffected.
- `native::mount_by_uuid` (pub) is `mount` without a `Device`: the apply has only the plan's
  UUID. `mount` now calls it.

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: `RestoreResult` (step 3 item 9)

Built tests first (core's `state::tests::the_restore_result_survives_the_bus` over every
outcome, with and without a snapshot and a time; the introspection with the thirteenth method).

- **`restore::state::RestoreResult`** (core): `state: ResultState` (`None`, `Ready`,
  `Ended(Outcome)`), `snapshot`, `message`, `when`; `none()`, `ready(snapshot)`,
  `of(&Report)` (the safety snapshot and the home choice stay in the file), `to_wire` /
  `from_wire` for `(sssx)` with `""` and `0` for a `null` snapshot or time (6b.10). An unknown
  state word is a bad reply. `WireRestoreResult` is re-exported from `helper`.
- **`RestoreResult()`** on `Helper3`: polkit `list`, not interactive; `ready` with the
  snapshot while `State::ready()` holds a plan, else `Report::load` of `result.json` (a
  missing file is nothing; an unreadable one is logged and answered as nothing, since the
  apply's own minimal report is what guards against that), else nothing. A read of one file:
  never a job, never refused, no mount.
- **When `result.json` is replaced**: only by the next apply (core writes it) and by `postrm
  purge` (item 10). The preparation, a cancel and a disarm all keep it, so the window can
  show the last restore's outcome after login whatever happened since. Settled here, as item
  5 left it.
- **Client**: `HelperClient::restore_result()`.

Gate: workspace tests, clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b helper slice: packaging (step 3 item 10), and the slice's gate (item 11)

**Packaging.** Tests first (`resource_tests::purge_removes_the_restores_leftovers_and_only_apsis_link`).
- The polkit `restore` action came with item 5, the `Helper3` bus policy with item 3.
- **`postrm purge`** now also: stops `apsis-disarm.timer` if one runs; removes
  `/system-update` **only when `readlink` gives Apsis's state folder** (another tool's link
  is left where it is, as everywhere else); removes a leftover `apsis-restore.service` and
  its wants link, the pop-upgrade-init drop-in and its folder with `rmdir` (so only when
  empty); then `/var/lib/apsis` whole (the plan, the last result, an ESP backup, a helper
  copy). `remove` keeps all of that: an armed restore survives a package remove and runs
  from the helper copy, as designed. Snapshots are never touched; the test holds that the
  script doesn't name `timeshift`. `sh -n` passes.
- Nothing new is installed; `just deb-install`'s re-login note stays.

**The gate (item 11)**: `cargo test --workspace` (core 364, helper 69, applet 77 and the
rest, all green), `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo fmt
--check` clean. Each item was committed on `restore-6b-core` as it passed, by the owner's
"yes" (the plan's "no commit" was for the unattended variant).

**The helper slice, as built (2026-10-02, nine commits):**
1. `State` keeps the ready plan next to the lock; `JobKind::Restore`.
2. The interface is `Helper3`; `List` carries each snapshot's format.
3. `CheckRestore`; the hook-flag rule, refusals on the wire, the dialog.
4. `Restore` prepares the plan to the ready prompt; the polkit `restore` action.
5. `RestartToRestore` re-checks, arms, starts the disarm timer, reboots.
6. `CancelRestore`; the plan goes with its window and at start without the link.
7. `--apply-restore`, the real runner for the apply.
8. `RestoreResult`.
9. Packaging's `postrm purge`.

**Open for the owner, gathered from the entries above:**
- `has_root` (item 4): taken as "the snapshot's `exclude.list` lets `/root` in".
- A refused "Restart now" removes the plan (item 6), not only a too-old one.
- Plymouth's progress bar is sent after the copy, not during (item 8); a live bar needs a
  second runner for plymouth.
- The dry runs run under `LC_ALL=C.UTF-8`, not `C` (item 5); the numbers are the same.

**Next**: the UI slice (PLAN 6b.13 step 4) can start; and apsis-test check 1 (6b.12), the
first real apply, needs its runbook, for which the check 0.4 runbook is the template
(DECISIONS 2026-10-01, "check 0.4 passed"). The helper's journal lines to expect are named
in the entries above.

## 2026-10-02 - 6b helper slice: the owner's answers to the four open items

Each accepted, with an addition, and built the same day (tests first).

1. **`has_root`**: "the snapshot's `exclude.list` lets `/root` in" is accepted, **and the
   snapshot's `/root` must have content.** If the list allows `/root` but the folder is empty
   or missing, `has_root` is false, so a restore with `--delete` can never wipe `/root` against
   an empty source. **Where the check lives**: the helper reads it in
   `check::SnapshotFiles::read` (`root_has_content`: `localhost/root` is a folder by its own
   name with at least one entry), hands it to core as `dialog::Inputs::root_has_content`, and
   core's `dialog::build` makes `has_root = filter::has_root(excludes) && root_has_content`.
   So it's in the dialog (`CheckRestore`) and again at plan time (`prepare` runs the same
   `check::dialog`). **What makes it bite**: `filter::Request` gained `restore_root`
   (`prepare` passes `dialog.has_root`), and `filter::rules` adds `- /root/***` when it's
   false, before the snapshot's own lines (whose `+ /root/**` would otherwise let `--delete`
   empty it). Tests: `dialog::tests::root_counts_only_when_the_snapshot_has_something_there`,
   `filter::tests::root_is_kept_when_the_snapshot_has_nothing_there`, and `check`'s snapshot
   test (a file in `root/`, then an empty folder, then none).
2. **A refused "Restart now" removes the plan**, for every refusal and failure, not only "too
   old". Reason: no waiting state (PLAN 6b.5, owner 2026-09-30); anything changed between the
   safety snapshot and a later restart is in no snapshot, so a kept prompt could silently wipe
   it. **The "Can't restore" dialog must say the plan was dropped and that a fresh Restore
   re-measures**: added to 6b.8's string table for the UI slice (`restart-refused-note`: "The
   preparation was dropped. Restore again to measure afresh."); it wasn't in the list before.
3. **Plymouth's bar after the copy**: accepted for 0.5.0, **with one `plymouth
   display-message` at the start of the apply** ("Restoring the system. Don't turn off the
   computer.", the boot screen's wording). Built: `apply::run` says it first, before core's
   state machine; `copy` no longer says it. One call, no second runner. **A live bar during
   the copy is a 0.5.x backlog item** (PLAN "0.5.x - After the restore").
4. **The dry runs under `LC_ALL=C.UTF-8`**: accepted, provided the real restore runs under the
   same locale. **It does**: `RealRunner::system()` runs rsync through `QuietRunner::new(SAFE_PATH)`,
   the same runner type with the same cleared environment and `LC_ALL=C.UTF-8`
   (`apsis_core::native::runner`), as `prepare`'s dry runs do; so the `--stats` numbers of the
   dry run and the real run come from the same locale. **Unverified, for the owner**: that
   `C.UTF-8` exists on the HP and in the offline boot. Read-only check:
   `ssh apsis-test 'LC_ALL=C.UTF-8 locale charmap'` (expect `UTF-8`). Added to the claims
   table.

## 2026-10-02 - 6b UI slice, step A: the restore flow in the applet (PLAN 6b.13 step 4)

Built tests first (`app::tests::restore`, 13 tests on the model: the 6b.12 applet list).
The preview branch's layouts were rebuilt in the real code; the branch stays unmerged.

**Model and flow (`app.rs`):**
- `Operation::Restore { snapshot, restore_home, safety_snapshot }` (kind `restore`), run
  through `HelperClient::restore` with the safety snapshot's progress.
- `Dialog::Restore { snapshot, check, restore_home, safety_snapshot }` (defaults: home kept,
  safety on; the radios only when `check.has_home`, and `RestoreHome(1)` is ignored without
  it), `Dialog::Refused { snapshot, refusal, dropped }`, `Dialog::StopRestore`,
  `Dialog::Ready`. `Dialog::lost_for_good()` (restore home and no safety), `extra_lines()`
  (no home, old format, which Apsis; in that order), `refusal_lines()` (the pair per
  refusal from 6b.8's table, and the dropped line).
- **Restore is a click only**: `can_restore()` is `can_create()` with exactly one snapshot
  selected (not a leftover). No `Shortcut`; `shortcut_for(Enter)` is `None` (tested). The
  button asks `CheckRestore` first (`checking` holds the toolbar meanwhile), then opens the
  dialog or the "Can't restore" one; a helper error goes to the status line.
- **Ready**: `Finished(Restore, Ok)` sets `ready`, opens `Dialog::Ready`, says `Preparing
  restore · ready` and lists (the safety snapshot is new; reads aren't blocked). While
  `ready`, `can_create()` is false. **Esc, Cancel and closing the prompt are Cancel restore**
  (`cancel_restore` → `CancelAnswered`: "Restore cancelled", list). **Restart now** →
  `RestartAnswered`: "Restarting…"; a refusal opens `Dialog::Refused { dropped: true }`
  (owner, 2026-10-02); `plan::TOO_OLD` / `plan::GONE` (as `CliError::Other` text) map to
  their lines (`plan_error_text`). Closing the window needs nothing: the helper removes the
  plan when the connection leaves the bus.
- **Preparing**: `restore_progress()`: `checking the snapshot…` before any progress, the
  safety snapshot's percent and time with it, `ready` at the prompt, and `Restore ready in
  another window` for another window's job at 100%. `stoppable()` includes a running
  `restore` job until ready; Stop opens `Dialog::StopRestore` (its own words).
- **Another window's restore**: `active_job()` now excludes only `configure`, so a restore
  (and a ready plan) elsewhere holds Create, Restore and Delete off (6b.9). Its end lists
  (the safety snapshot) and sets no status: that window says how it went.
- **`CliError::RestoreRefused(Refusal)`** from `Error::RestoreRefused(word)`; an unknown
  word stays `Other`. `error_summary` gives the refusal's first line.
- **After login**: the window asks `RestoreResult` when it connects
  (`read_restore_result`); an ended result is kept in `restore_result` and shown by
  `result_text()` when no status line is up: done (also `problems`), boot-kept (also
  boot-broken: the warning role on the phrase), failed (the error role on `incomplete`,
  what to do from the message: "backup disk" → reconnect, "space" → free space, else see
  README; `Restore again` when the snapshot is still listed), not started. The tooltips are
  6b.8's. `RestoreAgain` selects the snapshot and runs the normal flow with the defaults.
- `WindowResized(height)` from a window event listener, for the dialog's scrolling body.

**Views (`view.rs`):** the toolbar's Restore between Create and Delete
(`document-revert-symbolic`, no tooltip); the row tooltip "Older format: made without ACLs
and extended attributes" from the list's `rsync_flags` through `is_old_format` (on the row's
content: a `ListButton` can't be wrapped in a tooltip); the job line uses
`restore_progress()` and offers Stop for a restore until ready; the result line shows the
last restore when no status is up, with `phrase_line` for the role on the phrase only; the
four dialogs, the restore one with the preview's scrolling body (`DIALOG_CHROME` = 216 px,
the window's height tracked).

**Strings**: 6b.8's table, in `i18n/en/apsis.ftl` under "Restore (0.5.0)", with three
refusal pairs the table had no words for yet (boot space, size unknown, boot files) and the
"Restore ready in another window" line.

**Left for step B**: the layout test (`APSIS_LAYOUT_TEST=1`, both window sizes, screenshots
with `APSIS_SCREENSHOTS`), `docs/UI.md`, the man page and README lines of step 5.

Gate: workspace tests (applet 90), clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b UI slice, steps B and C: the layout test and UI.md

- **`app::tests::restore::every_restore_state_fits_both_window_sizes`** (from the preview's
  `preview_states_fit_both_window_sizes`): sixteen states (the dialog's seven variants, the
  refusal and the dropped one, preparing, the stop dialog, ready, the four results) at
  720 x 520 and 640 x 440. Each page fits the window under a 48 px header, each dialog with
  its button row is at most the window's height minus 16 px and its width minus 16 px. Runs
  with `APSIS_LAYOUT_TEST=1` (real text, the installed fonts), and with `APSIS_SCREENSHOTS`
  writes `restore-<state>-<w>x<h>.rgba` of the page with the dialog centred over it.
  **`.rgba`, not `.png`**: the main branch's `screenshots` test writes raw RGBA with the size
  in front, and the `image` crate is only a transitive dependency here; the preview's PNG
  writing would have added it as a dev-dependency. Same convention, no Cargo change.
- **Looked at** (dark theme, converted to PNG outside the tree): the dialog at 720 x 520 is
  the 6b.8 mockup; at 640 x 440 the busiest variant (no home, old format, Apsis 0.3) scrolls
  with its scrollbar and keeps Cancel and Restore visible; the ready prompt sits over
  `Preparing restore · ready`; the failed result has `incomplete` in the error colour and
  Restore again at the right. One fix from the look: `phrase_line`'s row is centred
  vertically (the coloured phrase sat above the baseline).
- **UI.md**: the toolbar, the status area (Stop for a restore; the result after login), the
  old-format row tooltip, the four dialogs in the table, a "Restore (0.5.0)" section under
  Jobs, the keyboard rule (no key for Restore; Esc on the prompt is Cancel restore), the
  tests. The README, man page and CHANGELOG lines are step 5's (with the release).

**The UI slice is complete** but for step 5's docs. Gate: workspace tests (applet 91, with
the layout test on), clippy `-D warnings` on all targets, fmt check: all clean.

## 2026-10-02 - 6b step 5: the docs (README, man page, ARCHITECTURE, CHANGELOG)

- **README**: the status note (0.5.0 in development; restore experimental, Pop!_OS with
  systemd-boot, at the next start); a "What it does" bullet; "How restore differs from
  Timeshift's" (never "restores like Timeshift"); step 4 of "Use" (the flow in the user's
  words); the password note (a restore asks every time); **"Known limitations of restore"**
  (the platform, another installation, same size and time, hard links and Flatpak, the live
  `fstab` and `crypttab`, kernelstub only and the snapshot's initrds, `/root` with content,
  home restored "too"); **"If a restore goes wrong"** (6b.11's steps, the recovery note, and
  the "incomplete" / "didn't start" lines, with `journalctl -b -1 -u apsis-restore`); the
  Esc row and "Restore has no shortcut"; five Troubleshooting rows; the layout test line.
- **Man page** (`docs/apsis.1`, `man -l` parses it): the description, a **RESTORE** section
  (the checks, the choices, the preparation, the prompt, the arm, the offline run, what isn't
  restored), Esc and "no shortcut", FILES (the recovery note, `/var/lib/apsis/restore/` and
  its files, the arm's four paths with the recovery hint, the helper's two offline modes),
  SECURITY (a restore is a write; the ready plan refuses writes; the `restore` action). The
  `.TH` line still says 0.4.2 and 2026-10-01: **the owner's at release**, with the version.
- **ARCHITECTURE**: the `restore` action and the offline part in the privilege model; the
  six actions; "The restore's files" under the installed files (the state folder's files,
  the arm's files and the commit point, the recovery note, the transient timer).
- **CHANGELOG.md**: an `[Unreleased] - 0.5.0` section (Added, Changed). **Not touched**:
  `resources/deb/changelog`, whose entries carry an identity trailer; the 0.5.0 entry is the
  owner's at release, with `Cargo.toml`'s version bump.
- Also the owner's at release: the metainfo's release entry and screenshots.

**PLAN 6b.13 steps 1 to 5 are built and documented.** What's left is the owner's: the
locale check and the `has_root` reading (DECISIONS 2026-10-02, the four answers), the fresh
baseline on 0.5.0, check 1's runbook (next), checks 1 to 13, the version and the release.

## 2026-10-02 - 6b: check 1's runbook and `tools/restore-check.sh`

- **`notes/check-1-runbook.md`** (untracked, like check 0.4's: it names the test machine's
  account and disk by placeholders, and the owner fills them in): the first real apply, keep
  home with the safety snapshot. 1a installs the .deb on apsis-test; 1b runs the owner's two
  read-only checks (the `C.UTF-8` charmap, and `pkaction` for the `restore` action: check 10);
  1c takes the fresh baseline on 0.5.0 and saves the "before" numbers (`/home`'s mode,
  cryptswap, ping's capability, the Flatpak list and size); 1d makes the changes the restore
  must undo (the marker, cowsay) and the one it must keep (a file in `~`); 1e drives the
  dialog and the preparation from apsis-test's screen while the helper's journal is followed,
  and checks the plan files and the recovery note with the prompt up (and that nothing is
  armed yet); 1f is Restart now with the boot id recorded and the screen watched; 1g the
  window after login; 1h the offline boot's journal (the apply's steps, `pop-upgrade-init`
  skipped), `tools/restore-check.sh`, the 1c comparison and `result.json`; 1i the cleanup.
  "Passes when" and what to record follow the 0.4 runbook's shape.
- **`tools/restore-check.sh`** (tracked; the small read-only script PLAN 6b.12's harness
  names): run as root after a restore. It checks the arm is gone (the link, both names, the
  unit, its wants link, the drop-in and its folder, the helper copy, `state.json`, the disarm
  timer), prints `result.json`, compares the ESP's `vmlinuz.efi` and `initrd.img` byte for
  byte with the files `/boot`'s links point to, checks the modules for the booted and the
  running kernel and the loader entries, `dpkg --audit`, failed units, `acpid` and
  `pop-upgrade`'s `is-enabled`, Pop's upgrade leftovers, and prints the previous boot's
  journal for `apsis-restore.service` and `pop-upgrade-init.service`. `FAIL` lines are what to
  look at; it changes nothing. POSIX sh, `sh -n` clean.

## 2026-10-02 - check 1, steps 1a to 1c (owner, apsis-test); check 10 passed

- **1a**: the 0.5.0 build installed; the policy file has the `restore` action (1), the bus
  policy names `Helper3` (2).
- **1b, the locale**: `LC_ALL=C.UTF-8 locale charmap` printed `UTF-8` on apsis-test. The claim
  in PLAN 6b.13's table is verified for the installed system; the offline boot runs the same
  system with the same locale files.
- **1b, check 10**: `pkaction --verbose --action-id io.github.atraxsrc.Apsis.restore` shows
  the description, the message, vendor Apsis, the icon, and `implicit any`, `inactive` and
  `active` all `auth_admin`. **Check 10 passed.**
- **1c, the baseline on 0.5.0**: `2026-10-02_10-50-46`, comment "baseline 0.5.0", created in
  2m 22s with `--link-dest` of the 2026-10-01 snapshot (the helper's `create ... done`; the
  list then had 3 snapshots, about 12.0 GB free of 30.1). The journal shows the 0.5.0
  helper's create path unchanged from 0.4.2.
- **The runbook's placeholder**: the mount command in 1c was run with `<backup-uuid>` as
  written, and the shell took it for a redirect ("backup-uuid: No such file"); nothing on the
  machine was touched (the mount never happened, `umount` said not mounted). The runbook now
  reads the UUID from `/etc/apsis/config.toml` (`backup_device_uuid`) into `$uuid` once, and
  every command uses it. The 1c mount step is still to run.
- **1c, the mount step** (with `$uuid`): three snapshots on the USB (09-30, 10-01, the
  baseline); the baseline's `info.json` has `"apsis-rsync-flags": "-aAX --numeric-ids"` (the
  new format) and `"app-version": "apsis 0.4.2"`: **the version isn't bumped yet**, so the
  snapshot's dpkg database says 0.4.2 and the restore dialog will show the "This snapshot has
  Apsis 0.4.2" line for it, by its rule (6b.2). Expected for check 1 (the binary is the
  0.5.0 build either way); the bump comes before check 9. The runbook's 1e says so.
- **1c, the "before" numbers, and 1d** (owner, 2026-10-02 11:05): `/home` is 755 root:root;
  `swapon` shows cryptswap (`dm-0`, 4G) and a zram swap; `getcap /usr/bin/ping` is
  `cap_net_raw=ep`; seven user Flatpak entries (an app, runtimes and extensions), 1.9G (decision
  6's figure); `acpid` and `pop-upgrade` `disabled`; kernel 7.1.5-76070105-generic; no cowsay,
  no marker. 1d: the marker, cowsay `ii`, the kept file in the home folder. Saved to the
  owner's `check-1c-before.txt`. Next: 1e.

## 2026-10-02 - check 1, step 1e (owner): the dry run refused as `size-unknown`; the fix

**What happened** (apsis-test, 11:07 to 11:09): `check-restore ... ok; home yes, root no,
current format, apsis no-restore:0.4.2` (the dialog as expected, with the Apsis 0.4.2 line);
`restore "<baseline>" keep-home safety ... started`; the restore's dry run ran
(`rsync -a -A -X --numeric-ids --delete --force --sparse --stats --dry-run
--no-human-readable --exclude-from=... localhost/ /`, 39 s); then `failed: can't restore this
snapshot: size-unknown`. The window showed "Can't restore" with the size line. No safety
snapshot was made, nothing was armed, the state folder holds only `restore.filter`.

**Cause** (in the code, reproduced with rsync 3.2.7 locally: the stats line parses with and
without `--no-human-readable`): the helper's `QuietRunner` reads rsync's standard output only
to hand it to the callback, and its `run`, `run_streaming` and `run_cancellable` all return
`RunOutput::stdout` **empty** (by design, since 0.4.0: a create keeps nothing of rsync's
output). `prepare::dry_run` and the real runner's `copy` read `output.stdout`: always nothing,
so every dry run was `SizeUnknown`, and the apply's copy would have handed core an empty
output (6b.10's "skipping file deletion" line never seen). My tests used a fake runner that
returned stdout in `RunOutput`, which hid it: the fakes didn't behave like the real runner.

**Fix** (tests first; the fakes now stream like `QuietRunner` and keep no stdout):
- `prepare::dry_run` takes any `Runner` and collects the stream in the callback
  (`the_dry_run_reads_the_size_from_the_stream`: the size, and `SizeUnknown` without the line).
- `RealRunner::copy` collects the whole stream for `Copied::new` (the existing copy test now
  runs against a streaming fake).
- `RealRunner::tool` runs through `run_streaming` and collects the output: kernelstub's lines
  reach the journal (`the_boot_refresh_reads_kernelstub_and_reports_a_failure`), and the
  `findmnt` for a separate `/home` in `open_backup` gets its answer (it would have refused
  every separate home as "isn't the partition the plan was made with").
- `mount_uuid` in `prepare.rs` and `service.rs` uses `DirectRunner`, which captures output,
  and was never affected. `native.rs`'s lsblk and findmnt likewise.

**Also seen**: no password prompt after Restore, although the `restore` action is
`auth_admin` everywhere (`pkaction`, 1b) and the helper calls polkit with
`AllowUserInteraction`. The likely reason is the test-only polkit rule on apsis-test (it lets
the SSH session's account call the helper) if it covers every `io.github.atraxsrc.Apsis.*`
action; to confirm from its scope, not from its content here. If the rule covers only `list`,
this is a bug to find.

**Next**: rebuild the .deb, reinstall on apsis-test, re-run 1e from the Restore click. The
failed preparation left nothing to undo.

**Resolved (owner, the same day)**: the test-only rule in `/etc/polkit-1/rules.d/` matches the
action prefix `io.github.atraxsrc.Apsis.`, so it covers the `restore` action too. The missing
prompt is the rule's doing, not a bug; the folder is root-only, which is why the first look at
it found nothing. The runbook's 1e now says so, and check 10 (the prompt on a machine without
the rule) stays with the owner's main machine.

## 2026-10-02 - check 1, step 1e again (owner): the safety snapshot's dry run had no exclude list

With the stream fix in, the first dry run measured the restore in 31 s. The next step failed:

    rsync: [client] failed to open exclude file .../apsis-staging/<name>/exclude.list

**Cause**: the safety snapshot's dry run reused the create plan's argv unchanged. That argv
reads `--exclude-from` and writes `--log-file` inside the staging folder, which the create
makes only when it runs (`execute` -> `build`). The old `size-unknown` bug stopped every run
before this point, so it never showed.

**Fix**: `apsis_core::native::dry_run_argv(argv, exclude_file)` turns a create's argv into a
dry run that works before the staging folder exists: the exclude list is read from the given
file, there is no `--log-file`, and `--dry-run --no-human-readable` come last. The helper
writes the plan's exclude text to `/var/lib/apsis/restore/safety.exclude` for the run and
removes it after, the way the home filter is handled; `clear_leftovers` removes it too.
Checked locally with rsync 3.2.7: a dry run against a destination whose parent doesn't exist
prints "created directory", creates nothing, and its "Total transferred file size" counts only
what `--link-dest` can't link. That is the safety snapshot's cost on the backup disk.

The test-only polkit rule on apsis-test is also recorded above as the reason for the missing
password prompt. Next: rebuild, reinstall, re-run 1e from the Restore click. Nothing to undo.

## 2026-10-02 - check 1, steps 1f to 1h (owner): the offline boot refused its own arm

The first full run on apsis-test. 1e passed as designed: `started`, the restore's dry run
(31 s), the safety snapshot's dry run, the create (77 s, `created .../2026-10-02_11-29-34`),
`ready` 2.5 min after the click; the Ready dialog, Restart now, `armed; restarting`. The
offline boot (2 s) ran `apsis-restore.service`: the plymouth line first, `pop-upgrade-init`
skipped on `ConditionPathExists=!/system-update/apsis-helper` (check 12's first part passed),
no `upgrade.sh` or `apt-get` in that boot. Then, one second in:

    the restore ended: not-started: the restore was refused: pending-update
    apply-restore ended: Finished(NotStarted)

The machine restarted on its own into a normal boot; the window said "The restore didn't
start · nothing was changed"; `result.json` said `not-started` with the baseline, the safety
snapshot and `home: keep`. Nothing on the disk was touched. The splash wasn't observed
(the owner's back was to the screen); check 2 will.

**Cause**: `check::Live::read` sets `pending_update` when `/system-update` or
`/etc/system-update` exists. In the offline boot `/system-update` is Apsis's own link, so the
apply's step 2 (the 6b.7 checks once more, `refusal::check`) refused itself. The dialog, the
preparation and the arming check were right to look at the name; the apply wasn't.
`refusal::check_pending`'s own note says it must not run at apply for this reason, but the
pending flag reached `refusal::check` by another path.

**Fix**: `Live` also records `etc_system_update` alone, and `Live::as_system_at_apply()` is
the view the apply checks: `/system-update` is Apsis's (step 1 verified the link), so only
`/etc/system-update` counts as another update's there. The test for `Live::read` covers a
dangling `/system-update` (pending for the dialog, not for the apply) and `/etc/system-update`
(pending for both).

**Two more things from `tools/restore-check.sh` and the helper's journal**:
- `FAIL drop-in folder left: /etc/systemd/system/pop-upgrade-init.service.d`:
  `arm::remove_arm_files` removed the drop-in but not its folder, which is Apsis's too (Pop's
  unit ships none). It now removes the folder when empty and leaves it when another drop-in
  is in it. `postrm` already did this on purge.
- `leftovers removed at start: request.json` after the restart: the apply's cleanup removed
  the arm but left the plan, and the helper's first start did the apply's job. The core's
  step 8 cleanup now removes `request.json` after `result.json` is saved and the link and arm
  files are gone (the plan's names are in the result). Nothing changes for a link whose
  removal failed: the cleanup stops before the plan, so a later boot that finds the saved
  end still reads it; the core test for that boot puts the plan back with the link.

Not changed: `tools/restore-check.sh` (its FAIL was right) and the runbook. Next: rebuild,
reinstall, and check 1 again from 1d (the marker, cowsay and the kept file are as 1d left
them, so 1d is a look, not a redo), then 1e with a fresh preparation, 1f with the screen
watched, 1g, 1h. The safety snapshot from this run stays on the USB; the next preparation
makes another.

## 2026-10-02 - check 1, run 2, step 1f (owner): the boot screen showed the journal's lines

Seen on the screen: "Restoring the system. Don't turn off the computer." for a moment, then
"copying, attempt 1 of 3" over the Pop logo; no progress bar; the restart in under a minute.

**Cause**: `RealRunner::say` wrote each line to the journal and to `plymouth display-message`,
so every step's line replaced the one before. That was the 6b.11 build; the owner's answer 3
(above, one message at the start) moved the first line into `apply::run` but left `say` as it
was.

**Fix**: `say` is the journal only. `run` writes the start line to the journal and sends it to
plymouth once, through `splash`. `progress` and `splash` share one `plymouth` call: after the
first failure plymouth isn't asked again, and the journal now says so with plymouth's stderr
("plymouth isn't answering (...)"), so a missing bar can be told from a bar too quick to see.
The test `say_goes_to_the_journal_only_and_restart_through_the_tools` replaces the one that
expected every line on the screen.

**Open from this run, pending 1h**: whether the copy's length (the restart came in under a
minute) and the missing bar are right for a same-day baseline, from the unit's journal: the
time between `copying, attempt 1 of 3` and rsync's summary, and whether the bar's call failed
(no line about it in this build; the next build logs it).

## 2026-10-02 - check 1, run 2, steps 1g and 1h (owner): the restore worked, on the wrong row

**The restore went through end to end**: `copying, attempt 1 of 3` at +1 s, `refreshing the
boot files` 54 s later, kernelstub's lines (the kernel and initrd copied to the ESP, the
previous pair backed up, both entries written), `the restore ended: done`, `Finished(Done)`,
the restart 56 s after the unit started. `pop-upgrade-init` skipped on the condition; no
`upgrade.sh` or `apt-get`. `tools/restore-check.sh`: every line ok (the arm gone, the drop-in
folder gone, no disarm timer, `result.json` done, no ESP backup left, the ESP byte for byte
`/boot`'s, modules for the running kernel, both loader entries, dpkg clean, 0 failed units,
no Pop leftovers). `/home`'s mode, cryptswap, ping's capability, the Flatpak list and size,
the two units and the kernel as in 1c. The state folder: `result.json`, `restore.filter`,
`rsync-log`, nothing else, and no "leftovers removed at start" line. The window: `System
restored to ...`.

**But the marker and cowsay were still there**, because `result.json` and the helper's
journal name the snapshot restored as `2026-10-02_11-29-34`: run 1's safety snapshot ("Before
restoring 2026-10-02 10:50"), taken at 11:29, after 1d's changes at 11:05. The baseline
`2026-10-02_10-50-46` wasn't the row selected. The restore did exactly what was asked; the
test's expectation about the files holds only for the baseline. **Check 1 needs run 3 with
the baseline selected**: the Ready dialog names the date of the snapshot being restored
(`ready-body`), which is the place to read before Restart now.

**A thing for the owner to decide** from this: the safety snapshot's comment names the
baseline's date ("Before restoring 2026-10-02 10:50"), and a row carrying that date is easy
to take for the baseline. Options: the comment reads "Safety snapshot, before restoring
<date>"; the Restore dialog shows the snapshot's comment under its date; both; or neither.
Not changed.

**Two small changes from the journal**, gate green:
- The journal said nothing between `copying` and `refreshing the boot files`; rsync's exit
  wasn't in it (the runbook expected a summary). Core now says `the copy ended: rsync exited
  <code>` (or `didn't exit`) right after the copy. Tested in the done path.
- `apsis-restore.service: Main process exited, code=killed, status=15/TERM`, `Failed with
  result 'signal'`. `systemctl reboot --no-block` starts the shutdown at once, and systemd
  stopped the unit while the helper was still unmounting the backup disk (`Mounted`'s drop,
  after `run` returned). `restart` now drops the mount before the reboot call, so the process
  has only its last journal line and the exit left. The window can't be closed entirely: the
  restart is asked from inside the unit, by design (6b.6). A `FailureAction=reboot` that fires
  into a shutdown already under way is harmless, and was. No unit test reaches the real
  mount; run 3's unit end ("Deactivated successfully" or still 'signal') is the check.

Not a defect: no "ESP backup", "check passed" or "kernel kept" lines; core says those only
when something is off, and the runbook's expectation list was written from the plan, not the
code. The runbook's 1h is corrected to what the journal says on a clean run.

Snapshots on the USB now: the two from 0.4, the baseline, and two safety snapshots
(`11-29-34`, `11-55-27`). Run 3 makes a third; they can go after check 3.

## 2026-10-02 - the safety snapshot's row (owner): both options

After run 2 restored a safety snapshot by mistake (above), the owner chose both: the comment
reads "Safety snapshot, before restoring <date>" (`prepare::safety_comment`), and the Ready
dialog shows the snapshot's date and comment in its muted line, as the Restore dialog already
did, so the row is clear at the last click. README, the man page and UI.md say the new
wording. Older safety snapshots keep the comment they were made with.

## 2026-10-02 - check 1 passed (owner, apsis-test, run 3)

The baseline `2026-10-02_10-50-46` restored, keep home, safety snapshot on. What the "Passes
when" list asked for:

- **The dialog**: `home yes, root no, current format, apsis no-restore:0.4.2`; the home radios
  shown (the snapshot has `/home`), the Apsis 0.4.2 line (expected until the version bump).
- **The helper's lines**: `restore ... keep-home safety ...: started` 12:55:54; the restore's
  dry run 40 s; the safety snapshot's dry run 34 s; the create 2 min 45 s (linked to the
  newest snapshot, the run 2 safety snapshot); `ready` 13:00:14, 4 min 20 s after the click;
  `restart-to-restore ...: armed; restarting` 13:00:18. Nothing was armed before Restart now.
- **The offline boot** (`-1`, 59 s of journal): the unit started 1 s into the boot; the start
  line; `copying, attempt 1 of 3` at +1 s; `the copy ended: rsync exited 0` 53 s later;
  `refreshing the boot files` 3 s after that (the ESP backup, unlogged on success);
  kernelstub's lines and the helper's summary of them; `the restore ended: done`;
  `Finished(Done)`; **`Deactivated successfully`** (run 2's TERM is gone, so the unmount
  before the reboot call did it); the restart 58 s after the unit started.
- **`pop-upgrade-init`** skipped on `ConditionPathExists=!/system-update/apsis-helper`; no
  `upgrade.sh`, `apt-get` or cleanup in that boot. The warnings in that boot are the HP's
  ACPI ones, present in every boot.
- **The screen** (owner looked): the Pop logo with "Restoring the system. Don't turn off the
  computer." alone, for the whole offline boot (one to two minutes as seen), then a normal
  restart to the greeter. No progress bar was noticed: the one call comes after the copy,
  3 s before the restart, and the new "plymouth isn't answering" line didn't appear, so
  plymouth took it. A live bar is 0.5.x's.
- **`tools/restore-check.sh`**: every line ok; `result.json` `done`, the baseline, the safety
  snapshot `2026-10-02_12-57-08`, `home: keep`.
- **The test files**: the marker gone, cowsay gone (`no packages found`), the kept file there
  with its 11:05 time.
- **Before and after**: `/home` 755 root:root; cryptswap on (`/dev/dm-0`); `ping`
  `cap_net_raw=ep`; the same seven Flatpak rows and 1.9G; acpid and pop-upgrade disabled;
  kernel 7.1.5. Identical to 1c.
- **The state folder**: `result.json`, `restore.filter`, `rsync-log`; no leftovers line at
  the helper's start.
- **The window**: `System restored to <the baseline's date>`; the list has the baseline, three
  safety snapshots (`11-29-34`, `11-55-27`, `12-57-08`) and the two 0.4 snapshots.
- **Check 10's `pkaction`**: `auth_admin` everywhere (1b). The prompt itself is shown only
  without the test rule: on the owner's main machine, at release.

Not done from the list: launching one Flatpak app after the restore (the owner, before check
3). Six fixes came out of the three runs, all committed today. Next: check 2, the kernel
rollback, with its own runbook from check 1's.

## 2026-10-02 - check 2's setup (owner): kernelstub picks the newest kernel, so the apply now names it

**Getting a kernel the snapshot lacks**, on apsis-test (Pop!_OS 24.04, 7.1.5 running, 7.0.11
installed):
- Route A, a Pop kernel update: none offered, after `apt update`.
- Route B, remove 7.1.5 and snapshot on 7.0.11: apt refuses twice. `linux-system76` holds the
  image, `pop-server` holds `linux-system76`, `pop-desktop` holds `pop-server`. Not done to
  the test machine. Nothing was removed; a snapshot taken meanwhile while booted on 7.0.11
  (with 7.1.5 still installed and linked) was deleted as unusable.
- **Route C, taken**: an Ubuntu mainline build as kernel B (`v7.2.6`, image and modules .debs
  from kernel.ubuntu.com, into the test account's home), the baseline `2026-10-02_10-50-46`
  as the snapshot. The image's maintainer scripts hand `run-parts` two directories
  (`/etc/kernel/<step>.d /usr/share/kernel/<step>.d`), which 24.04's `run-parts` refuses
  ("missing operand"), so the .deb was repacked with the second directory dropped from its
  four scripts (`dpkg-deb -R`, `sed`, `dpkg-deb -b`) and installed. Two things the repack
  left: the kernel file owned by the test account (`chown root:root`), and no
  `linux-update-symlinks` run (done by hand, so the links name 7.2.6). Pop's packages and
  metapackages were never touched; the restore removes all of B.

**The finding**: with the links still at 7.1.5, kernelstub's hook put 7.2.6 on the ESP. Its
source (`application.py:167`): `latest_option, previous_option =
KernelOption.latest_option(boot_path)`, the newest `vmlinuz-*` in `/boot` by version; a given
`--kernel-path`/`--initrd-path` wins (lines 171-193), the `/boot/vmlinuz` link is the last
fallback (lines 179-181), and the given path isn't saved (the configuration holds only the
options and ESP settings, as the journal's dump shows). PLAN 6b.6 had "newest by version"
right; the 2026-10-02 runbook's "follows the links" reading was wrong, and PLAN's "they
agree on Pop!_OS" holds for a live system, never at apply in a rollback: after pass 1,
`/boot` has the snapshot's kernels and the protected running kernel (rule 10), the newest.
Left alone, kernelstub would put the running kernel on the ESP, the check would fail against
the linked snapshot kernel, and **every kernel rollback would end `boot-kept`**: safe, never
the rollback. Found before any restore ran.

**The fix**: `RealRunner::refresh_boot` names the kernel: `kernelstub --verbose
--preserve-live-mode --kernel-path /boot/<target of /boot/vmlinuz> --initrd-path /boot/<target
of /boot/initrd.img>`, the targets read from the restored tree's links (relative targets
under `/boot`, absolute ones as they are). Without both links the plain call runs and the
check decides. The "previous" pair stays kernelstub's choice (after the copy, the second
newest is the snapshot's own newest, so kernelstub skips or duplicates it; reported, never a
failure; the next kernel update rewrites it). Test
`the_boot_refresh_names_the_kernel_the_links_point_to`; PLAN 6b.6 step 5 and the README's
limitation line say it. Check 2 runs against this build.

## 2026-10-02 - check 2 passed (owner, apsis-test): the kernel rollback

Route C (above): the baseline `2026-10-02_10-50-46` (kernels 7.1.5 and 7.0.11, links to
7.1.5) restored from a system running a mainline 7.2.6, with the build that names the kernel.

- **Before**: 7.2.6 running, three modules folders, the links and the ESP at 7.2.6; the
  marker, cowsay and the kept file as in check 1.
- **The preparation**: `check-restore ... ok`; the restore's dry run 36 s; the safety
  snapshot's dry run 35 s; the create 5 min 15 s (a whole kernel's modules to copy, linked to
  the run 2 safety snapshot); `ready` 7 min after the click; `armed; restarting`.
- **The offline boot** (64 s of journal): the start line; `copying, attempt 1 of 3`; `the copy
  ended: rsync exited 0` 58 s later; `refreshing the boot files`; kernelstub: "Manually
  specified kernel path: /boot/vmlinuz-7.1.5...", the same initrd, "Copying Kernel into ESP",
  "Backing up old kernel: No old kernel found, skipping"; `removed kernel
  7.2.6-070206-generic: not in the snapshot`; `the restore ended: done: the previous kernel's
  boot files: the kernel on the ESP isn't the one /boot links to`; `Finished(Done)`;
  `Deactivated successfully`. `pop-upgrade-init` skipped on the condition; nothing of the
  upgrade ran.
- **After**: `uname -r` 7.1.5; modules 7.0.11 and 7.1.5 only; `/boot` the snapshot's files
  and links, dated as the snapshot's; no 7.2.6 package in dpkg. `tools/restore-check.sh`:
  every line ok, the ESP byte for byte `/boot`'s 7.1.5 pair. The marker and cowsay gone, the
  kept file there, the other numbers as before. The state folder: `result.json`,
  `restore.filter`, `rsync-log`.

**The previous-pair report, explained**: kernelstub's "previous" is the second-newest kernel
in `/boot`, and at refresh time that was 7.1.5 itself (7.2.6 still protected, removed only
in step 7), the same file as the named kernel, so `installer.py:65-66` skipped the backup
and the ESP kept its earlier previous pair: 7.1.5, written when 7.2.6 was installed. The
snapshot's `/boot/vmlinuz.old` names 7.0.11, hence the report. The ESP's second entry boots
7.1.5 as well; nothing is broken, and the next kernel update rewrites the pair. As PLAN 6b.6
says: reported in the journal and the result, never a failure. **The window shows `System
restored to <date>`** and drops the message for a `done`; the owner leaves it so (the
journal and `result.json` have it).

Without the fix of the same day, this run would have ended `boot-kept` on 7.2.6. Check 2 is
the run that proved the fix on the machine.

Left on apsis-test: the 1a build of Apsis once more (the baseline's), `~/kernel-b`, the kept
file. Still open from check 1: launching one Flatpak app after a restore. Next: check 3,
home restored too, against the same baseline, with its runbook from check 1's.

## 2026-10-02 - check 3 passed (owner, apsis-test): home restored too

The baseline `2026-10-02_10-50-46`, **Restore them too**, safety snapshot on. apsis-test has
no separate `/home`.

- **Before**: two files in the test account's home made after the baseline (13:46 and
  14:06), a test line appended to `.bashrc`, the marker and cowsay back; `/home` 755, the home
  folder 750; seven Flatpak rows in the user install, 1.9G, the home 2.2G.
- **The preparation**: `check-restore ... ok`; `restore ... restore-home safety ...: started`;
  the restore's dry run 48 s; the safety snapshot's dry run 33 s; its create 2 min 2 s (home
  linked to the previous safety snapshot, which had it already: the kept file's link count 2
  in the new one); `ready` 3 min 53 s after the click.
- **The offline boot** (73 s of journal): `copying, attempt 1 of 3`; `the copy ended: rsync
  exited 0` 67 s later (home entered); kernelstub naming 7.1.5, and this time backing up
  7.0.11 as the previous pair (7.2.6 gone since check 2), so check 2's previous-pair report
  is cleared by this restore, not by a kernel update; `the restore ended: done`;
  `Deactivated successfully`; `pop-upgrade-init` skipped on the condition.
- **After**: both home files "No such file", `.bashrc` ending in the baseline's `fi`, the
  marker and cowsay gone; the safety snapshot `2026-10-02_14-12-19` has the account's home
  folder with both files and the test line. `/home` 755 and the home folder 750 as before;
  the same seven Flatpak rows, 1.9G and 2.2G as before (unchanged files skipped, hard links
  kept); **Proton VPN, the one app in the user install, starts**. `tools/restore-check.sh`:
  every line ok. `result.json`: `done`, the baseline, the safety snapshot, `"home":
  "restore"`. The state folder: `result.json`, `restore.filter`, `rsync-log`.

**Noted**: apsis-test's snapshot settings include home (the earlier safety snapshots carried
it, as the link count shows), so `prepare`'s "add `/home` to the safety snapshot when home is
restored" branch wasn't exercised by this check; the core tests cover it. The exclude list's
`/home` lines weren't listed (the `$uuid` variable wasn't set in that shell); the home folder
in the snapshot is the proof that matters.

The window's line and tooltip: to be confirmed by the owner (expected `System restored to
<date>` and "Home folders were restored too."). Left on apsis-test: the 1a build of Apsis once
more; five safety snapshots on the USB (`11-29-34`, `11-55-27`, `12-57-08`, `13-49-04`,
`14-12-19`), which can go, one at a time, before or during check 4. Next: check 4 (Stop
during the safety snapshot; Cancel at the prompt; after a cancel a normal restart doesn't
restore; a filled system disk between ready and Restart now).

## 2026-10-03 - a reinstall during a delete left a half-removed snapshot (owner, apsis-test)

After check 3 the applet's status area showed the helper's warning `2026-10-02_14-12-19:
incomplete: no exclude.list` (check 3's safety snapshot), still there after a reboot on
2026-10-03. Found from the helper's journal, `dpkg.log` and the USB (read-only mount):

- 14:25:42 `delete-many ["2026-10-02_14-12-19", "2026-10-02_13-49-04", +2 more]: started`
  (helper pid 4338), `14-12-19` first.
- 14:26:22 the owner's `apt install --reinstall` of the 0.4.2-1 deb (check 3's 3i step,
  "reinstall before check 4"). The deb's `prerm` stops `apsis-helper.service` on `upgrade`;
  14:26:23 `Stopping apsis-helper.service`, `Deactivated successfully` (SIGTERM, the job
  41 s in, no `deleted` line, no `Finished`). The bus started a new helper at once; its
  list at 14:26:24 carries the warning.
- The folder: `info.json` and `rsync-log` (14:14) kept, `exclude.list` gone, `localhost/`
  5 entries and 7.1G left (`prune::remove_in` unlinks a folder's files in `readdir` order
  and recurses into its folders, so `exclude.list` went before the long walk). The owner
  then deleted `11-29-34`, `13-49-04` and `11-55-27` one at a time (about 70 s each, all
  `done`); `12-57-08` was never listed again and is presumably among the "+2 more".

Not a bug in the delete itself: nothing in the helper removes a finished snapshot's files,
and `exclude.list` is written and synced before the create's rsync starts. Two gaps, **noted
here and deferred until after checks 4 to 13 (owner, 2026-10-03)**:

1. **The package's `prerm` stops the helper mid-job.** Right for the binary swap, wrong
   while a job runs: a delete leaves a half-removed folder; a create leaves a staging folder
   (already handled: the next create removes it, and the list shows it as a leftover); a
   restore preparation would leave its staging folder too, and its plan files are cleared
   on the next start. Candidates: the `prerm` refuses or waits while the helper reports a
   running job; or the helper, on SIGTERM, finishes the file it's on and announces
   `stopped` the way a Stop does. The applet should also say something when the helper
   vanishes mid-job (the window got no `Finished`; both windows just listed).
2. **A half-deleted snapshot is invisible and stuck.** `NativeRsync::list` turns a folder
   with `info.json` and no `exclude.list` into a warning only (Timeshift's rule); the window
   shows no row, so there is nothing to click, and `delete_known` refuses a name that isn't a
   snapshot or a leftover. The only way out is a root `rm -rf` on the USB. Candidate: list
   such a folder like a leftover (a row Delete can remove; `delete_snapshot` already accepts
   it, since it only asks for `info.json`), with the warning as its label.

Cleanup for check 4 (root, the owner): mount the USB read-write and remove
`timeshift/snapshots/2026-10-02_14-12-19` by hand, which frees 7.1G; nothing in `snapshots-*`
links to it (the tag links are made after the rename and removed by the delete first).
Check 4's runbook gets this as a precondition. Also noted: on apsis-test, deleting one
safety snapshot (2 to 3 GB, mostly hard links) takes about 70 s.

**Correction (2026-10-03, check 4's 4a):** the tag links are removed *after* the folder
(`delete_snapshot`: `prune::remove_at`, then `remove_tag_links`), not before, so the stopped
delete left `snapshots-ondemand/2026-10-02_14-12-19` pointing at the removed folder. Harmless:
`list` scans `snapshots/` only, and the next create's `update_symlinks` rebuilds every tag folder;
removed by hand with the folder. The reinstall of the current build wasn't repeated: the
helper and applet on apsis-test hash the same as the local deb (built 2026-10-02 14:26).

## 2026-10-03 - check 4 passed on apsis-test: Stop, Cancel, a normal restart, a filled disk

PLAN 6b.12 check 4, runbook `notes/check-4-runbook.md`, all four against the baseline
`2026-10-02_10-50-46`, keep home, safety snapshot on; the 14-12-19 cleanup and the build check
first (the correction above). Nothing restored, nothing left armed at any step.

- **4b Stop**: clicked 16 s in, during the restore's own dry run (`checking disk space…`, the
  create was never reached). Journal: `stop "2026-10-02_10-50-46" for :1.x (uid 1000):
  stopping, SIGTERM to rsync's process group (SIGKILL after 10 s)`, then 6 s later `restore
  "2026-10-02_10-50-46" keep-home safety for :1.x: stopped`. The state folder: today's
  `restore.filter` and check 3's `result.json`; no `request.json`, no `/system-update`, no
  unit file; no `apsis-staging` on the USB; the same three snapshots.
- **4c Cancel**: `ready` 5 min 17 s after the click (the restore's dry run 31 s, the safety
  snapshot's dry run 18 s, its create 3 min 45 s). Journal: `restore "2026-10-02_10-50-46" for
  :1.x: plan removed (cancelled)`, `cancel-restore for :1.x: cancelled`. The state folder:
  `result.json` only (the plan and the filter gone); the safety snapshot `2026-10-03_10-08-43`
  stays, four rows. **The normal restart**: new boot id, `journalctl -b -1 -u
  apsis-restore.service` says `-- No entries --`, zero `system-update.target` lines, kernel
  7.1.5 unchanged, the marker and the kept file absent as check 3 left them.
- **4d The filled disk**: `ready` 3 min 11 s after the click (the create 1 min 18 s, linked
  to the morning's safety snapshot). `root_needs` 10 106 179 719 bytes (the 2% floor of a
  460G partition), 454 974 615 552 free; `fallocate` of about 424 GiB left 200M (`df`:
  `460G 436G 200M 100%`). Restart now: no restart. Journal: `plan removed: request.json`,
  `restart-to-restore "2026-10-02_10-50-46" for :1.x: failed: can't restore this snapshot:
  system-space:10106179719:209661952`; the job ended `stopped`. After `rm` of the fill:
  424G free, the state folder `restore.filter` and `result.json`, no `request.json`, no
  link, `apsis-helper.service` the only unit. The safety snapshot `2026-10-03_10-20-54`
  stays, five rows.
- 4e (the 31-minute prompt): skipped (owner); `Plan::is_too_old` has its unit tests.

**Journal wording vs the runbook** (behaviour as designed, lines differ): the Stop line says
`stopping, SIGTERM ...` rather than `ok`; a refused Restart now logs `plan removed:
request.json` from `arm::disarm` (not `remove_plan`'s `plan removed (refused ...)`) and
`failed: can't restore this snapshot: ...` because `Error::RestoreRefused` isn't in
`describe_error`'s refused list, although nothing ran. Candidate, deferred with the other
post-check fixes: say `refused:` for `RestoreRefused`. Also: a refused restart leaves
`restore.filter` behind (the next preparation clears it); a cancel removes it.

**The window** (owner, each step redone once to read the texts): after Stop `Restore stopped`;
the "Stop the restore?" dialog's body "The safety snapshot being made is deleted. Nothing on
the system has changed."; after Cancel `Restore cancelled`; the "Can't restore this snapshot"
dialog "Not enough space on the system disk to restore (needs 9.4G, 200M free)." in the error
colour, "Free some space, then try again.", "The preparation was dropped. Restore again to
measure afresh."; **after Close the status area still reads `Preparing restore · ready`**:
`on_restart_answered` opens the refusal dialog and returns without touching the status, and
the job's later `stopped` end never reaches that path. Candidate, deferred with the other
post-check fixes: on a refusal, set the status to `Restore stopped` (or a line naming the
refusal) when the dialog closes.

Left on apsis-test: the current build, five snapshots (the three of 4a plus the safety
snapshots `10-08-43` and `10-20-54`, either can go, check 5 needs one restore to cut). No
reinstall needed: no restore ran. Next: check 5, the power cut at about 30% of the copy.

## 2026-10-03 - check 5 passed on apsis-test: the power cut at about 30% of the copy

PLAN 6b.12 check 5, runbook `notes/check-5-runbook.md`: the baseline `2026-10-02_10-50-46`,
keep home, safety snapshot on; the power button held about five seconds roughly 20 s into
the offline boot's copy; the next power-on came back to the restore by itself and attempt 2
finished it. The runbook's journal lines were checked against the code first (core
`restore/apply.rs`, the helper's `apply.rs` and `service.rs`), not the design wording.

- **Preconditions**: the installed helper hashed `a764aa62…` (check 4 restored nothing, no
  reinstall). The USB held five snapshots, not seven: check 4's text redo left no safety
  snapshots. `10-20-54` and `10-08-43` deleted from the window one at a time (59 s and 69 s to
  `deleted`, `done` 11 s and 7 s later), three snapshots left, no staging folder, the three
  tag links sound, 12G free. Two markers this time: `/etc/apsis-test-marker` early in rsync's
  walk and `/var/lib/apsis-test-marker-late` late in it, so a finished copy is told from one
  that stopped partway; cowsay; the kept file.
- **The preparation**: `check-restore ... ok; home yes, root no, current format, apsis
  no-restore:0.4.2`; `started` 10:59:39; the restore's dry run 38 s; the safety snapshot's dry
  run 19 s; its create 3 min 33 s (`2026-10-03_11-00-37`); `ready` 11:04:41, 5 min 2 s after
  the click. The plan: the baseline, that safety snapshot, `home: keep`, kernel 7.1.5; no
  link, no unit before Restart now.
- **The cut boot** (`-2`): Restart now at about 11:06:50 (`armed; restarting`, the helper
  stopped 11:06:54). The boot's journal holds **one second**: first and last entry 11:07:15,
  journald's flush of 971 entries to `/var/log/journal`, then nothing. The unit's start, the
  Apsis line and `copying, attempt 1 of 3` were lost with the power (written after the flush,
  never written back). `-b -2 -u apsis-restore.service` and `-u pop-upgrade-init.service`
  both say `-- No entries --`. The cut itself, by the owner's count (the button held from
  15 s after the Apsis line appeared), at about 11:07:37; nothing on disk records it.
- **The retry boot** (`-1`, 61 s of journal): first entry 11:08:03; the unit started 11:08:04;
  `Restoring the system. Don't turn off the computer.`; **`copying, attempt 2 of 3`** 11:08:05,
  which is the proof of the count: `attempts` 1 was saved and fsynced before the cut boot's
  `copying` line, and `state.json` itself is removed by step 8; `the copy ended: rsync exited
  0` 11:09:00 (**55 s**, about check 1's uncut 53 s: rsync walks the whole tree again, the
  files already copied save transfer, not time; it sent 211 MB against a 9.58 GB tree,
  speedup 45); `refreshing the boot files` 11:09:02; kernelstub naming 7.1.5's vmlinuz and
  initrd and backing up 7.0.11 as the previous pair; `the restore ended: done` 11:09:03;
  `apply-restore ended: Finished(Done)`; `Deactivated successfully` 11:09:04. None of `the
  copy broke`, `cut off`, `given up`, `wasn't saved`, `the ESP backup of an earlier boot`,
  `panicked`. The normal boot's first entry 11:09:23; ssh answered by 11:09:38: **3 min 28 s**
  from Restart now to the desktop, with the cut, two offline boots and the normal boot in it.
- **The screen** (owner): the Pop logo with the Apsis line in both offline boots, nothing
  else; no fsck line, no boot menu, no greeter until the normal boot.
- **PLAN item 12, first part**: in the retry boot `pop-upgrade-init.service` was skipped on
  `ConditionPathExists=!/system-update/apsis-helper`; no `upgrade.sh`, `apt-get`,
  `system-upgrade` or `system-update-cleanup` line in that boot; afterwards `acpid` and
  `pop-upgrade` `disabled` (not masked), no `/upgrade-attempted`, no `/pop-upgrade`, the
  drop-in folder gone. The cut boot's skip line is among the lost entries, so nothing is
  claimed for it; the design needs the retry boot's, which is there.
- **`tools/restore-check.sh`**: every line ok (the arm gone including `state.json` and the
  drop-in folder, no disarm timer, no `esp-backup/`, the ESP byte for byte `/boot`'s, modules
  for 7.1.5, both loader entries, dpkg clean, 0 failed units, no Pop leftovers). Both markers
  "No such file" (attempt 2 went through `/etc` and `/var/lib`), the kept file there (10:58),
  cowsay gone. Before and after identical: `/home` 755 root:root, cryptswap on, `ping`
  `cap_net_raw=ep`, the seven Flatpak rows and 1.9G, acpid and pop-upgrade disabled, kernel
  7.1.5. Proton VPN starts. `result.json`: `done`, the baseline, `2026-10-03_11-00-37`,
  `home: keep`, `when` 11:09:03. The state folder: `result.json`, `restore.filter`,
  `rsync-log`. The helper's start after login removed no leftovers.
- **The window** after login: `System restored to 2026-10-02 10:50`; four rows (the 11:00
  safety snapshot, the baseline, the two 0.4 snapshots); the backup disk line `16G / 28G ·
  62% used · 10G free`. The same after the reinstall (read from `result.json`).
- **The 1a build back** (5h): `just deb`, scp, `apt install --reinstall` with no job running;
  the rebuilt helper hashes **the same `a764aa62…`** as the one installed on 2026-10-02: the
  build is reproducible. Logged out and in.

**Observed, not defects**:
- `rsync-log` holds attempt 2 only (303 lines, from 11:08:05, the retry's pid). rsync appends
  and nothing in the apply truncates the file between attempts (the preparation clears it),
  so attempt 1's lines went the way the journal's did: not written back before the cut. No
  `deleting` line names a temporary file of the cut copy (pattern `.*/\.[^/]+\.[A-Za-z0-9]{6}$`);
  whether one ever reached the disk isn't known. The markers and restore-check are the proof
  of the tree's state, not the log.
- The retry boot's kernel logged `FAT-fs (nvme0n1p1): Volume was not properly unmounted` and
  the same for `nvme0n1p2`: the ESP and the recovery partition were mounted at the cut, as in
  any power cut. Pop never fscks them (`fstab` pass 0); kernelstub wrote the ESP without
  complaint and the check passed byte for byte. ext4's root replayed its journal silently
  (no recovery line at warning level).
- `systemd-cryptsetup[795]: device-mapper: remove ioctl on cryptswap  failed: Device or
  resource busy` at 11:09:04, the retry boot's shutdown. Absent from the shutdowns of the
  four earlier offline boots (`-7`, `-9`, `-13`, `-15`: checks 3, 2, 1 run 3 and 1 run 2) and
  of three normal boots. Seen once; systemd's own shutdown ordering (closing the random-key
  swap while it's still on), not the apply's; cryptswap was on again in the normal boot.
  Watch for it in check 6.
- The journal of a boot cut by power is good for its first second here: anything a later
  check wants from a cut boot must be read from the next boot's effects, as this one was.

Left on apsis-test: the current build, four snapshots (the three base ones and
`2026-10-03_11-00-37`, which can go). Next: check 6, the backup disk unplugged before Restart
now's reboot (never started, normal boot, the message after login), then unplugged at about
30% of the copy and replugged (copy broke, attempt 2 finishes).

## 2026-10-03 - check 6 Part A passed; Part B found the apply finishing over a pulled disk (fixed)

PLAN 6b.12 check 6, runbook `notes/check-6-runbook.md`, the baseline `2026-10-02_10-50-46`,
keep home, safety snapshot on, the current build (`a764aa62…`) on apsis-test, two markers
(`/etc/apsis-test-marker`, `/var/lib/apsis-test-marker-late`), cowsay, the kept file.

**The pull is after Restart now, not before**: `check_and_arm` opens the shared mount and
reads the snapshot's `/boot` sizes for the ESP space check, so with the disk out at the
prompt the arm fails (`failed: backup device not found: <uuid>`, `plan removed:
request.json`, the job `stopped`) and no offline boot runs. PLAN's "unplugged before Restart
now's reboot" is the window from `armed; restarting` to the offline boot's 60 s wait; the
runbook pulls the disk when the screen goes dark.

**Part A passed** (the disk pulled during the shutdown, kept out through the login):
- `armed; restarting` 11:59:41; the offline boot 12:00:03 to 12:01:05; the unit's start line
  12:00:04; **12:01:04 `the restore ended: not-started: the backup disk <uuid> wasn't found
  within 60 seconds`**, `Finished(NotStarted)`, `Deactivated successfully`; the normal boot
  answered ssh at 12:01:45, 2 min 4 s after the click. Nothing about copying.
  `pop-upgrade-init` skipped on the condition for the whole wait.
- The screen: the Apsis line for the minute of the wait, then a normal boot (the design's
  "didn't start" screen line went with check 1's one-message decision; the journal and the
  window carry it).
- `result.json` `not-started`, the baseline, the safety snapshot `2026-10-03_11-52-29`,
  `home: keep`, the message with the disk's UUID. The state folder: `result.json` and
  `restore.filter`; no plan, no `state.json`, no link, no unit, no drop-in folder. The three
  test files, cowsay and the kernel untouched.
- **The window with the disk out** shows both at once: the list area `Backup disk not
  connected (UUID xxxx…). Plug it in.` with Try again, the Backup disk line `not connected`,
  and the status area `The restore didn't start · nothing was changed · reconnect the backup
  disk`; its tooltip is the helper's message with the UUID. After the plug-in the window's
  own list came back by itself at 12:02:25 and the two panel applets' two seconds later; the
  result line stayed.
- The preparation: `started` 11:51:28, dry runs 38 s and 19 s, the create 3 min 44 s,
  `ready` 5 min 20 s after the click. 2 min 4 s from Restart now to the desktop.

**Part B, first run (12:12 to 12:15): the pull landed in the wait, not the copy.** One
offline boot; `copying, attempt 1 of 3` came 24 s after the start line (1 s in every earlier
boot): the disk was out when the boot began and back about 23 s in, so `udevadm wait` found
it, the copy ran whole (54 s), `done`. Recorded as a "disk late by 24 s" variant of Part A
that passed; the broken-copy branch wasn't reached. The kernel log also shows the disk
leaving at 12:04:30 and 12:17:19 (two `lost async page write` each) and 12:23:54 (`lost sync
page write`) between the parts: the owner re-seated it between ports. **Pulling it with
writes pending is the one thing that can hurt the backup filesystem**: let the helper's job
end and the mount go before a pull, and `e2fsck -n` it before check 7 (runbook).

**Part B, second run (12:28 to 12:31): the pull landed in the copy and the apply finished
over it.** Preparation `ready` 12:28:33 (safety snapshot `2026-10-03_12-25-16`); the offline
boot 12:30:28 to 12:30:52; `copying, attempt 1 of 3` 12:30:30; the pull at 12:30:47 (ext4:
`I/O error while writing superblock`, `Aborting journal on device sda-8`; the mount shows
the `shutdown` option afterwards); **12:30:48 `the copy ended: rsync exited 23`; 12:30:50
`refreshing the boot files`**; kernelstub; `the restore ended: problems: some files couldn't
be written or deleted; see /var/lib/apsis/restore/rsync-log`; `Finished(Problems)`; a normal
boot; **the window `System restored to 2026-10-02 10:50`**. restore-check: every line ok
(the ESP, dpkg's audit and the units can't see a mixed tree).

The evidence (`rsync-log`, 355 lines): 40 `rsync: [sender] readdir(".../localhost/usr/share/
icons/…"): Input/output error (5)` lines from 12:30:47 on, `Number of deleted files: 59`,
`total size is 7,645,244,508`, `rsync error: some files/attrs were not transferred (see
previous errors) (code 23)`, and **no "IO error encountered -- skipping file deletion"
anywhere**. The tree afterwards: the `/etc` marker gone, the `/var/lib` marker there,
cowsay's binary gone with dpkg still listing it `ii` (`/var/lib/dpkg` never reached), the
helper a third hash (the baseline's 1a build: `/usr/libexec` reached). A mix, reported as
restored.

**Cause.** PLAN 6b.10 held that a disk going away looks to rsync like an unreadable folder,
which makes it print the "skipping file deletion" line, and `Copied::end` mapped a plain 23
to `Ended { problems: true }`. rsync 3.2.7 prints that line only when, with its I/O-error
flag set, it next reaches a folder's deletion pass (`--delete` is delete-during here). When
the whole disk vanishes every remaining `readdir` fails at once, no folder is entered again,
the walk collapses within a second, and the line never comes: a plain 23, exactly the code
of "a few files couldn't be read". The helper's segment reader was not the gap: the line
isn't in the log either.

**Fix (core `restore::apply`, gate green, 2026-10-03).** After a copy that exited 23 the
apply calls `find_snapshot` and `check_snapshot` once more, the check it already makes
before every copy. The snapshot gone (on ext4 the vanished disk's mount is in its forced
shutdown state and every `open` returns EIO, so `info.json` can't be read): **the copy
broke**, with the journal line `the copy broke (<rsync's last lines>; after the copy, the
snapshot's folder isn't on the backup disk: the disk went away during the copy, so the
system may be a mix of the snapshot and what was there before): restarting to try again,
attempt 2 of 3` and the usual retry over the kept link; the third such break ends `failed`
as the others do. The snapshot still there: `problems` as before. Tests:
`exit_23_with_the_snapshot_gone_after_the_copy_is_a_copy_that_broke` (the fake's disk
leaves during the copy; no ESP backup, no boot refresh, the ESP untouched, the retry
finishes it) and `exit_23_with_the_snapshot_still_there_is_a_copy_with_problems`. PLAN
6b.6 step 3 and 6b.10 "Exit 23" now say four cases. Not changed: `Copied::end` itself (the
deletions-skipped rule still holds for a folder that goes unreadable while the disk stays).

**Found with it, deferred with the other post-check fixes** (owner to decide):
1. **`problems` reads as `done` in the window**: `result_text` gives `Outcome::Done |
   Outcome::Problems` the same `System restored to <date>` line, and the helper's message
   is only in the tooltip. A line that says some files weren't restored, with the log named,
   is due.
2. **The result tooltip's home and safety snapshot are fixed strings** ("Home folders were
   kept.", "Safety snapshot: none."): `RestoreResult` on the wire carries the outcome, the
   snapshot, the message and the time, not `home` or `safety_snapshot`, though `result.json`
   has both. Seen after check 6's first Part B run. Adding the two fields is cheap while
   `Helper3` is unshipped; check 3's "Home folders were restored too." was never confirmed
   for this reason.
3. The apply logs nothing of rsync's errors on a plain 23 (the result names the log only).

**apsis-test now**: a mix of the baseline and the state before 12:30, with the 1a build's
helper. The way back: install the fixed build, restore the baseline with the disk in (the
fixed apply runs the copy), install the fixed build again, log out and in, the markers, then
Part B once more with the pull at 30 s after the Apsis line. Snapshots on the USB: the three
base ones, `11-52-29`, `12-09-56`, `12-25-16`.

## 2026-10-03 - check 6 passed on apsis-test: the backup disk pulled, twice (on the fixed build)

PLAN 6b.12 check 6, runbook `notes/check-6-runbook.md`; Part A and the first two Part B runs
are in the entry above. apsis-test was brought back first: the fixed build installed
(`6d6b01b5…`; `e2fsck -n` of the USB `clean` after the day's pulls), the baseline restored
with the disk in by the fixed apply (`copying` 13:00:36, `rsync exited 0` 55 s later, `done`,
both markers gone, restore-check ok, the 1a helper back), the fixed build installed again,
the markers and cowsay at 13:04.

**Part B, third run (the pull at 30 s after the Apsis line):**
- The preparation: safety snapshot `2026-10-03_13-07-14`; Restart now at about 13:10:40.
- **The broken boot** (`-2`, 13:11:03 to 13:11:19, 16 s): the start line 13:11:04; `copying,
  attempt 1 of 3` 13:11:05; the pull 13:11:18 (`usb 1-1: USB disconnect`, ext4 `error -5
  reading directory block` for `comm rsync`), rsync in `/var/lib` at the time; **13:11:18 `the
  copy ended: rsync exited 23`**, then **`the copy broke (rsync: [sender] opendir
  ".../localhost/var/lib/systemd" failed: Input/output error (5)` … twenty such lines …
  `rsync error: some files/attrs were not transferred (see previous errors) (code 23)`;
  `rsync skipped its deletions after a read error, so the system may be a mix of the snapshot
  and what was there before): restarting to try again, attempt 2 of 3`**; `apply-restore
  ended: Retry { attempt: 2 }`; `Deactivated successfully`; the restart over the kept link.
  No `refreshing the boot files`. The screen went dark right after the pull (owner).
- **This run broke through the old rule**: rsync printed `IO error encountered -- skipping
  file deletion` (`rsync-log` line 1092, after 1008 I/O errors), because the pull landed
  while it was listing `/var/lib` and it still reached a folder's deletion pass. At 12:30 it
  didn't. So rsync's behaviour depends on where the pull lands; both branches are in the
  code, this one proved live, the new post-copy check by the 12:30 evidence and its unit
  test, not yet live. One more Part B run would show it; optional (owner).
- **The retry boot** (`-1`, 13:11:37 to 13:12:36): the disk plugged in as the screen went dark
  enumerated at 13:11:37 with the boot (`usb 1-2: new high-speed USB device`); the unit
  13:11:39; `copying, attempt 2 of 3` 13:11:40, 1 s in; `rsync exited 0` 13:12:32 (52 s);
  `refreshing the boot files`; kernelstub naming 7.1.5; `the restore ended: done`;
  `Finished(Done)`; `Deactivated successfully`. The normal boot 13:12:55; ssh 13:13:28: about
  2 min 48 s from Restart now to the desktop with the pull, the broken boot and the retry in
  it.
- **PLAN item 12, first part, in both offline boots**: `pop-upgrade-init.service` skipped on
  `ConditionPathExists=!/system-update/apsis-helper` in `-2` and `-1`; no `upgrade.sh`,
  `apt-get`, `system-upgrade` or `system-update-cleanup` line; `acpid` and `pop-upgrade`
  `disabled`; no `/upgrade-attempted`, no `/pop-upgrade`, the drop-in folder gone.
- **After**: `tools/restore-check.sh` every line ok; both markers "No such file", the kept
  file (13:04) there, cowsay gone (`no packages found`, no binary); before and after
  identical (`/home` 755 root:root, cryptswap, `cap_net_raw=ep`, the seven Flatpak rows,
  1.9G, disabled twice, 7.1.5); Proton VPN starts; `result.json` `done`, the baseline,
  `13-07-14`, `home: keep`; the state folder `result.json`, `restore.filter`, `rsync-log`
  (1410 lines: `building file list` at 13:11:05 and 13:11:40, the first run's errors and its
  skipping line, then a clean second run; rsync appends, and with no power cut both runs
  are there); the helper's start after login removed no leftovers. The window: `System
  restored to 2026-10-02 10:50`.
- 6h: the fixed build installed again (owner). On the USB: the three base snapshots and five
  safety snapshots (`11-52-29`, `12-09-56`, `12-25-16`, the clean restore's, `13-07-14`),
  7.6G free; the safety ones can go one at a time.

**What check 6 established**, beyond PLAN's line: Restart now needs the disk (the arm reads
the snapshot's `/boot` sizes), so "unplugged before Restart now's reboot" is the window from
`armed; restarting` to the end of the offline boot's 60 s wait; the window shows the
"not connected" list message and the `didn't start` result line at once; a disk late by 24 s
is picked up by `udevadm wait` and the restore goes through; a disk that vanishes mid-copy
gives rsync a plain 23 with or without its deletions line depending on where the walk is,
and the apply now treats both as a broken copy (commit `62b1560`, the fourth exit 23 case in
PLAN 6b.10). Next: check 7, the boot files failing (a `kernelstub` that exits 1 on
apsis-test, put back afterwards): the ESP files back, the newer kernel boots, `boot-kept`
shown.

## 2026-10-03 - triage before check 7: two corrections, the check order, what's fixed before release (owner)

**Corrections (docs only).**

- **No scheduler on the roadmap.** PLAN's roadmap line (under "UI polish") no longer lists a
  scheduler: it was dropped on purpose (2026-09-28) and `phase-5.2-schedule` stays parked.
  Undock stays, as an optional late item. The entries of 2026-09-29 that call the scheduler
  "roadmap" are history and stand as written.
- **The collection PR and the posts wait.** The earlier collection PR, #114, is closed (the
  owner's statement; not looked up from the session, which doesn't talk to GitHub). A new PR
  and the posts on Reddit and chat.pop-os.org come once Apsis is complete, after 0.5.0 at
  the earliest. `docs/RELEASE.md` marks the collection entries that way and has the PR text
  rewritten for 0.5.0 standalone, with no comparison to Timeshift in it.

**The checks left (PLAN 6b.12).**

- **Order: 7, 11, 9, then 8 last.** The recovery drill (8) is the one most likely to end in a
  reinstall of the OS, which would take the harness (the sudoers entry, the test-only polkit
  rule) and check 11's preparation with it.
- **Check 12 is folded into the others.** Its after-restore state (`pop-upgrade-init` skipped
  on the drop-in's condition in the offline boot, `acpid` and `pop-upgrade` not masked, no
  `/upgrade-attempted`, the drop-in gone) goes into the verify step of checks 7 and 11. The
  `/pop-upgrade` refusal is a test without a reboot at the start of check 7's session.
- **Check 13 is rerun on 0.5.0**: `DeleteMany` moved to `Helper3`, and the fix for a
  half-deleted snapshot touches the delete.

**The deferred fixes, numbered here** (they were noted in three entries of 2026-10-03):

1. the package's `prerm` stops the helper mid-job ("a reinstall during a delete", gap 1);
2. a half-deleted snapshot is invisible and stuck (the same entry, gap 2);
3. a refused Restart now is logged `failed:`, not `refused:` (check 4);
4. the status area still reads `Preparing restore · ready` after a refused restart (check 4);
5. `problems` shows the same window line as `done` (check 6, item 1);
6. the result tooltip's home and safety-snapshot lines are fixed strings (check 6, item 2).

Beside them: the apply logs nothing of rsync's errors on a plain exit 23 (check 6, item 3),
and a refused restart leaves `restore.filter` behind (check 4).

- **Before release** (fixed after the checks, before the smoke test): fix 5; fix 6, either by
  carrying `home` and `safety_snapshot` in `RestoreResult` or by dropping those tooltip
  lines, since a fixed string that can be false never ships; and rsync's errors on a plain
  exit 23: **the apply logs the last 20 lines of rsync's standard error to the journal.**
  Why 20: it is the tail the runner already keeps (`STDERR_TAIL_LINES` in the helper's
  `apply.rs`, the lines a broken copy's message carries), so there is no second buffer and
  no second number to keep in step; rsync's closing line takes one and leaves 19 per-file
  errors, enough to tell what kind of failure it was (permissions, a read-only filesystem,
  I/O); a mass failure (check 6's 1008 I/O errors) can't flood the journal; and the full
  list stays in `rsync-log`, which the result names.
- **Batch after the checks**: fixes 1 to 4, and removing `restore.filter` after a refused
  restart. Fix 2 is one of the two reasons check 13 is rerun, so this batch lands before the
  release gate as well (Claude's reading of the owner's list, to be corrected if wrong).
- **Release gate after the fixes**, on the actual 0.5.0 .deb: one happy-path restore (check
  1's flow) and check 13.

**Noted while reading the code for check 7, not triaged yet:**

- The window gives `boot-broken` the same line and tooltip as `boot-kept` (`result_text`:
  "still boots the previous kernel", "the ones it had were put back"). For a put-back that
  failed that is false. `result.json` and the journal tell the two apart, so check 7's pass
  rule reads those, not the window alone.
- `tools/restore-check.sh` has no `boot-kept` case: after a real one (a kernel rollback whose
  boot refresh failed) its ESP lines would print FAIL although the ESP is right, because
  they compare the ESP with what `/boot/vmlinuz` links to. In check 7 the kernel doesn't
  change, so every line should still be ok there.

## 2026-10-03 - triage, second part (owner): all six fixes before release, the armed remove, boot-broken's line

The entry above stands; this adds the owner's answers to what it left open.

- **All six fixes land before release.** The reading in the entry above is right: the batch
  (fixes 1 to 4, and removing `restore.filter` after a refused restart) is done before the
  release gate, like fixes 5 and 6 and the 20 lines of rsync's standard error.
- **The `prerm` disarms an armed restore on `remove`: before release.** Found while reading
  for check 7: after `apt remove apsis` with a restore armed and no restart, the next boot
  still restores, on any later day. Nothing the package ships is needed for it: the link
  (`/system-update`), the unit and its wants link under `/etc/systemd/system/`, the helper
  copy, `state.json`, `request.json` and `restore.filter` under `/var/lib/apsis/restore/`
  and the drop-in are all written by the helper on arm or at the preparation, none is in
  dpkg's list, the package has no conffiles, and `postrm` names them only under `purge`. The
  one thing that would undo the arm, the 10-minute disarm timer, runs the packaged helper,
  which a remove takes away. It belongs in 0.5.0's own `prerm`: a remove always runs the
  installed version's scripts, and 0.5.0 is the first version that can arm. This is the
  armed branch of fix 1. Still open: the same on `upgrade` and `deconfigure` (Claude's
  recommendation; an upgrade is benign as it is, the timer disarms with the new helper).
- **`boot-broken` shown as `boot-kept`: before release**, in the same group as fix 5
  (`problems` shown as `done`). Both are a result line that says something milder than what
  happened.
- **`tools/restore-check.sh` has no `boot-kept` case: fixed before any check that could end
  `boot-kept` with a kernel change. Not a release gate.** Check 7 isn't such a check (the
  kernel is 7.1.5 on both sides).
- **The PR text** in `docs/RELEASE.md` has the compatibility fact back as one plain line,
  worded like the README: Apsis keeps Timeshift's on-disk layout, so existing snapshots
  stay usable. Still no description of Apsis by comparison.

**Before release, in one list**: fixes 1 to 6 (fix 1 with its armed branch on `remove`);
`restore.filter` removed after a refused restart; the last 20 lines of rsync's standard
error in the journal on a plain exit 23; a line and tooltip of its own for `boot-broken`.
Then the release gate on the real 0.5.0 .deb: one happy-path restore and check 13.
**Not gating the release**: the `boot-kept` case in `restore-check.sh`.

## 2026-10-03 - check 7 passed (and check 12): the boot refresh failed, the ESP was put back, `boot-kept`

Runbook `notes/check-7-runbook.md` (untracked). The baseline `2026-10-02_10-50-46`, keep
home, the safety snapshot on, the disk in, on the fixed build (helper `6d6b01b5…`). In the
offline boot `kernelstub` was a stand-in that exits 1. What is marked (owner) was seen on
apsis-test's screen or in a terminal; the rest is read from the check's logs.

**How the stand-in got to be the binary the apply calls.** The apply looks up the bare name
`kernelstub` on its fixed `PATH` (`/usr/local/sbin` first) after the copy, so a stand-in
placed live never reaches the call: in `/usr/bin` the copy writes the baseline's file back,
in `/usr/local/sbin` its `--delete` removes it. So the stand-in was a new file,
`/usr/local/sbin/kernelstub`, **bind-mounted onto itself before the preparation**: the
filter's ordinary mount rule then wrote its own exclude line for it, the copy left it
alone, and the restart dropped the mount. The real `/usr/bin/kernelstub` was never touched.

- At the Ready prompt `restore.filter` had exactly one such line, its line 35,
  `- /usr/local/sbin/kernelstub`, and was 29 bytes longer than check 6's filter (that line,
  owner). Before it:
  `findmnt` showed the mount, `mountinfo` had one line for it, `command -v kernelstub` on
  the helper's `PATH` gave the stand-in, `dpkg -V kernelstub` printed nothing.
- **The stand-in is a pass-through unless a restore is armed**: it runs the real kernelstub
  whenever `/system-update` isn't Apsis's link (its dry run through the stand-in exited 0),
  and fails only in the offline boot. That the link is still Apsis's at the call is read
  from core's `apply.rs`: `run` begins with the armed test (line 300), the link is removed
  only in `give_up`, `clean_up` and the panic hook, every early `finish` returns, the
  kernelstub call is at line 661 inside `boot_files`, and `clean_up` runs at line 786 after
  it; `- /system-update` is the filter's first rule. The journal below shows it held.
- Before failing it appended one line to the ESP's `cmdline` file, so that the put-back had
  a changed file to put right. The condition for that held in step 1: one `options` line in
  `Pop_OS-current.conf`, the running command line contains it, and kernelstub's sources
  only read `/proc/cmdline` and write the ESP's file (`copy_cmdline`).
- **What this changes compared with a real `boot-kept`**: one more exclude line; kernelstub
  resolved in `/usr/local/sbin`; the kernel is 7.1.5 on both sides, so the files put back
  are what a good refresh would have written, except `cmdline`.

**Step 0, check 12's refusal** (owner): with `/pop-upgrade` present, Restore on the
baseline opened "Can't restore this snapshot" with "A Pop!_OS upgrade is in progress." and
"Finish or cancel it first, then restore."; the journal's `check-restore` line said
`refused: pop-upgrade-pending`. The optional refusal at Restart now was **skipped on
purpose**: it becomes the regression test for fixes 3 and 4 (`failed:` for `refused:`, the
status area left at `Preparing restore · ready`).

**Step 1, the baseline**: kernel 7.1.5, modules for 7.0.11 and 7.1.5, the `/boot` links to
7.1.5 and `.old` to 7.0.11; seven files hashed on the ESP (the current and the previous
pair, `cmdline`, both entries); `vmlinuz.efi` and `initrd.img` the same as `/boot`'s 7.1.5
files; 359M free on the ESP; `acpid` and `pop-upgrade` `disabled`; none of the three Pop
names; both kernelstub hooks with `--preserve-live-mode`; `/usr/local/sbin` empty.

**The preparation**: the filter written 14:34, the safety snapshot `2026-10-03_14-34-57`,
the plan 14:39 (`"home": "keep"`, running kernel 7.1.5), no link before Restart now.

**The offline boot** (`-1`, 14:43:03 to 14:44:09), `apsis-restore.service`, exactly as the
pass rule had it:

| time | line |
|---|---|
| 14:43:04.726 | the unit starts |
| 14:43:04.743 | `Restoring the system. Don't turn off the computer.` |
| 14:43:05.588 | `copying, attempt 1 of 3` |
| 14:44:01.195 | `the copy ended: rsync exited 0` |
| 14:44:03.717 | `refreshing the boot files` |
| 14:44:03.726 | `kernelstub: check-7 stand-in: called as: kernelstub --verbose --preserve-live-mode --kernel-path /boot/vmlinuz-7.1.5-76070105-generic --initrd-path /boot/initrd.img-7.1.5-76070105-generic` |
| 14:44:03.726 | `kernelstub: check-7 stand-in: cmdline before: 7132d236…`, `cmdline after: 6b271557…`, `failing on purpose` |
| 14:44:03.819 | `the boot refresh failed (kernelstub exited with code 1: check-7 stand-in: failing on purpose): putting the boot files back` |
| 14:44:09.003 | `the restore ended: boot-kept: the boot refresh failed (kernelstub exited with code 1: check-7 stand-in: failing on purpose); the boot files from before were put back` |
| 14:44:09.513 | `apply-restore ended: Finished(BootKept)` |
| 14:44:09.519 | `Deactivated successfully` |

No `removed kernel`, no second attempt, no `boot-broken`. The ESP backup logs nothing when
it works: it and the `syncfs` are the 2.5 s between the copy's end and `refreshing`.
Nothing from `apsis-restore` at warning level or above.

**The result**: `result.json` `boot-kept`, the baseline, `14-34-57`, `"home": "keep"`, the
message of the journal's `the restore ended` line. The state folder: `result.json`,
`restore.filter`, `rsync-log`; no plan, no state, no helper copy, no `esp-backup/`.

**The ESP**: the seven hashes after the boot are the seven of step 1 (`ESP-IDENTICAL`), no
`.apsis-tmp` file. `cmdline` was `7132d236…` before, `6b271557…` after the stand-in wrote
to it, `7132d236…` after the boot: **the put-back rewrote a file that had changed**, on the
real vfat ESP, in 5.2 s for all seven files.

**Check 12, the after-restore state: passed.** In the offline boot `pop-upgrade-init.service
... was skipped because of an unmet condition check
(ConditionPathExists=!/system-update/apsis-helper)`; no `upgrade.sh`, `apt-get`,
`system-upgrade` or `system-update-cleanup` line; `acpid` and `pop-upgrade` `disabled`; no
`/upgrade-attempted`, no `/pop-upgrade`; the drop-in folder gone. With step 0's refusal and
the skips already seen in checks 5 and 6, every part of PLAN's item 12 is shown.

**After**: `tools/restore-check.sh` every line ok, 0 failed units. As predicted, it can't
tell `boot-kept` from `done` when the kernel doesn't change: its ESP lines compare with what
`/boot/vmlinuz` links to, which is the same 7.1.5. Both markers "No such file", the kept
file there, cowsay gone; the kernel 7.1.5 with its modules; before and after identical
(`NUMBERS-IDENTICAL`); the stand-in still in place (the exclude kept it) and no longer a
mount point.

**The window** (owner): `System restored · still boots the previous kernel · see README`,
the tooltip as in the strings file, no Restore again button; the same line after the
current build was reinstalled.

**Cleanup** (owner, and the cleanup log): the stand-in removed, `command -v kernelstub`
gives `/usr/bin/kernelstub`, `dpkg -V kernelstub` and `dpkg --audit` print nothing, both
hooks carry the flag, the real kernelstub's dry run exits 0. The safety snapshot deleted
from the window (14:50:11 to 14:51:20, one list per client after `done`): 12031221760 of
30120226816 bytes free on the USB with the three base snapshots, the same byte count as
before the check (owner). The safety snapshot had cost 1.2 GB. The current build
reinstalled.

**Timings**: the preparation about 5 minutes; the copy 55.6 s; `syncfs` and the ESP backup
2.5 s; the stand-in 0.1 s; the put-back 5.2 s; the unit 65 s in all (34.6 s of CPU); the
normal boot started 20 s after it ended; from the click to ssh answering about 3 min 15 s
(owner).

**Found with it:**

- **The `boot-kept` tooltip names the wrong cause** (owner): it says "The new boot files
  didn't check out", which is wrong when kernelstub itself failed and wrote nothing new.
  Wording that covers both causes is due, for example "The boot files couldn't be
  refreshed, so the ones from before were put back." **Before release**, with fix 5 and
  the line for `boot-broken`: the three are one pass over the result texts.
- **A partial data point for check 13's rerun** (owner): in section P the five old safety
  snapshots went in one bulk delete on the 0.5.0 build, with both panel applets and the
  window open: one `delete-many` job, five deleted, `done`, 14:20:33 to 14:26:09, and each
  client listed once after `done`. Whether a "Busy" line showed in a window was not
  observed, so check 13 is still to be rerun in full.
- **The backup disk's kernel name changed over the restart** (owner): `sda` before, `sdb`
  after. The window shows the kernel name; every lookup is by UUID. Harmless.

**What check 7 established**: a boot refresh that fails after the copy ends with the ESP
exactly as it was, the kernel the machine started with booting, the system restored, and
`boot-kept` in the result, the journal and the window; the put-back works on the real ESP
and restores a file that was changed in between; the apply's kernelstub call carries
`--kernel-path` and `--initrd-path` from the restored `/boot` links; the filter's mount
rule protects a file that is a mount point when the restore is prepared. Not shown by it:
`boot-kept` across a kernel change (the put-back restoring another kernel than the one
`/boot` links to), which is where `restore-check.sh` needs its `boot-kept` case first.

**Left**: checks 11, 9 and 8, in that order; then the fixes; then the release gate (one
happy-path restore and check 13 on the real 0.5.0 .deb).

## 2026-10-03 - check 11 passed: an owner by number, a device node, a file capability, an ACL and a user attribute come back

PLAN 6b.12 check 11, runbook `notes/check-11-runbook.md` (untracked, revised once). One
snapshot taken for the check, then one restore of it: keep home, the safety snapshot on,
the disk in, the real kernelstub, the fixed build (helper `6d6b01b5…`). It is the run on
hardware of the three ignored tests in `tests/restore.rs` (owners by number, a device node,
a file capability), with an ACL and a `user.*` attribute on a root-owned file. What is
marked (owner) was seen on apsis-test's screen; the rest is read from the check's logs.

**The first run stopped at step 1, and what it found.** The runbook's preparation refused
to write: `/opt/apsis-test-null` was already there. The preparation of 2026-10-01 is still
live on apsis-test and is inside "6b baseline" (`2026-10-01_17-19-36`) and "baseline 0.5.0"
(`2026-10-02_10-50-46`), one inode each across the two (link count 2), and **not** inside
"baseline1". Nothing was written. Those files became the items, with a fifth one added,
and the runbook was revised to create nothing under `/opt` and to clean nothing there:

| item | where | value |
|---|---|---|
| owner by number | `/opt/apsis-test-ids` | `54321:54322`, mode 644 |
| device node | `/opt/apsis-test-null` | `c 1,3`, `0:0`, mode 644 |
| file capability | `/usr/bin/ping` | `cap_net_raw=ep` (`security.capability`, 20 bytes) |
| ACL | `/opt/apsis-check.txt` (`0:0`, 644) | `user:65534:r--`, `mask::r--` |
| a `user.*` attribute | `/opt/apsis-check.txt` | `user.apsis` = `1` |

**Which snapshot, and why its own.** With the preparation inside the baseline, the
baseline could have been the snapshot restored. The check took its own all the same
(`2026-10-03_16-23-20`, comment "check 11", `"apsis-rsync-flags" : "-aAX --numeric-ids"`):
it holds the current build, so no reinstall followed, and its link counts are a second look
at the create (below). It and its safety snapshot were deleted at the end.

**A slip in the second run: step 2 was skipped, and the recovery.** The snapshot was not
taken, so the shell's `snap` was empty; the items script, given no name, looked at the live
system (its header said `== the live system`), and step 2b's `SNAPSHOT-HOLDS-ALL-FIVE` was
printed for a comparison of the live system with itself. Step 3's five changes were then
made with no snapshot to restore. Recovered without a restore: cowsay purged, the markers
and the kept file removed, then the runbook's put-back block (`chown`, `mknod -m 0644 ... c
1 3`, `setfacl -m u:65534:r--`, the attribute, `setcap cap_net_raw=ep`; every `rc=0`).
Afterwards the items block and the "before" numbers were identical to step 1's baseline.
One trace is left: the device node is a new one (its time is 16:22 of that day, the two
baselines hold the one of 2026-10-01); type, numbers, owner and mode are the same. The run
then went on from step 2. The lesson is a harness rule now (PLAN 6b.12, "A runbook script
never guesses its mode").

**The snapshot** (16:23:20 to `done` at 16:28:50, linked against the baseline; 1.24 GB).
Inside it, on the backup disk mounted read-only: the same items block as the baseline's,
this time with the header `== inside snapshot 2026-10-03_16-23-20`. Link counts: 3 for
`apsis-test-ids`, `apsis-check.txt` and `ping` (this snapshot and the two baselines: rsync
links a file to the earlier snapshot only when owner, mode, time, ACL and attributes match,
so the baselines' copies carry what the live files carry), 1 for the node (the put-back's
new node differs in time from the baseline's; the create's log shows `cDc.t......` for it
and `hf` for the other three). The dry run from the snapshot onto the live paths printed no
item line: the live system was the snapshot.

**The five changes** (after the markers, cowsay and the kept file): `chown root:root`, the
node removed, `setfacl -b`, the attribute removed, `setcap -r`. The changed log shows all
five different and **no size or modification time moved** on the three files. Then, as
root, read-only, `rsync -a -A -X --numeric-ids --dry-run -i` from the snapshot onto the
four live paths printed exactly:

```
.f.......ax apsis-check.txt
.f....og... apsis-test-ids
cD+++++++++ apsis-test-null
.f........x ping
```

(and `.d..t...... ./` for `/opt` itself). No `>f`: no file's data would be sent. This
settled, before any restore, the two claims the runbook had as unverified: rsync as root
sees an owner-only change and a missing `security.capability` on a file whose size and
time are the same. The ACL and the `user.*` attribute had been shown the same way on the
main machine as an ordinary user (rsync 3.2.7, the restore's flags: 0 files transferred,
both back).

**The preparation.** `check-restore ... ok; home yes, root no, current format, apsis
no-restore:0.4.2` (the version isn't bumped, so the dialog's Apsis line shows for this
snapshot as for the baseline). The flags, live, from the helper's `running` lines: the
restore's dry run `rsync -a -A -X --numeric-ids --delete --force --sparse --stats --dry-run
--no-human-readable --exclude-from=.../restore.filter <snapshot>/localhost/ /` (39 s), and
the safety snapshot's `rsync -aii -A -X --numeric-ids --recursive --verbose --delete
--force --stats --sparse --delete-excluded --info=progress2 --link-dest=<the check's
snapshot>/localhost/ ...` (dry run 15 s, create 1 min 32 s, 0.25 GB). `ready` 2 min 53 s
after the click. At the prompt: no rule in `restore.filter` names `opt` or `ping`; the one
rule that isn't anchored is `- home/Downloads`, from the config (observation a); the plan
`"old_format": false`, `"home": "keep"`, kernel 7.1.5, the safety snapshot
`2026-10-03_16-34-13`; the five still changed.

**The offline boot** (`-1`, 16:38:10 to 16:39:04), `apsis-restore.service`:

| time | line |
|---|---|
| 16:38:11.717 | the unit starts |
| 16:38:11.731 | `Restoring the system. Don't turn off the computer.` |
| 16:38:12.632 | `copying, attempt 1 of 3` |
| 16:39:00.618 | `the copy ended: rsync exited 0` |
| 16:39:03.125 | `refreshing the boot files` |
| 16:39:03.857 | kernelstub's lines: `--preserve-live-mode`, 7.1.5 as the kernel and initrd, 7.0.11 backed up as the previous pair |
| 16:39:03.955 | `the restore ended: done` |
| 16:39:04.468 | `apply-restore ended: Finished(Done)` |
| 16:39:04.474 | `Deactivated successfully` |

The copy 48 s, the unit 53 s (26.4 s of CPU); the normal boot started at 16:39:24, about
two minutes from Restart now to ssh answering (owner). `result.json`: `done`, the check's
snapshot, the safety snapshot, `"home": "keep"`, an empty message. `tools/restore-check.sh`:
every line ok, 0 failed units.

**The five, after the restore**: the items block identical to step 1's baseline
(`ITEMS-IDENTICAL`): `54321:54322`; `character special file 1,3`, `0:0`, mode 644;
`/usr/bin/ping cap_net_raw=ep` and the attribute's 20 bytes the same; the ACL with
`user:65534:r--` and `mask::r--`; `user.apsis` `1`; mode, size and modification time of the
three files as before the snapshot. The "before" numbers identical too (`/home` 755
root:root, cryptswap, the seven Flatpak rows, 1.9G, `disabled` twice, 7.1.5). Both markers
and cowsay gone, the kept file there. The helper's sha256 still `6d6b01b5…`: no reinstall.

**Check 12's after-restore state**: `pop-upgrade-init.service ... was skipped because of an
unmet condition check (ConditionPathExists=!/system-update/apsis-helper)`; no `upgrade.sh`,
`apt-get`, `system-upgrade` or `system-update-cleanup` line; `acpid` and `pop-upgrade`
`disabled`; no `/upgrade-attempted`, no `/pop-upgrade`, the drop-in folder gone.

**The window** (owner): `System restored to 2026-10-03 16:23`. The tooltip and the rows
were not noted before the deletes.

**The USB's free bytes**: 12031221760 before; 10793320448 after the check's snapshot;
10540732416 after the safety snapshot; 12031221760 after both were deleted, exactly back.

**Cleanup**: the kept file removed; `/opt` not touched (the old three as section P showed
them, the node with its new time); ping `cap_net_raw=ep`, the same bytes; `dpkg --audit`
clean; both snapshots deleted from the window one at a time (59 s and 73 s to `deleted`);
three snapshots left, no rsync, no `/system-update`.

**Two limits of what the check shows:**

- **`--numeric-ids` itself isn't told apart.** 54321 and 54322 have no names, and for an id
  without a name rsync uses the number with or without the flag; 65534 has one, but on one
  machine with one user database a name maps to the same number anyway. Shown: owner,
  group and the ACL's entry come back by number on the real root. The flag's own effect
  needs another user database (a recovery from a live USB).
- **`rsync-log` doesn't name attribute-only changes.** The restore's log has `cD+++++++++
  opt/apsis-test-null` and no line for the owner, the ACL, the attribute or the capability
  (0 error lines), as the experiment on the main machine had predicted for the ACL and the
  attribute. So the log can say what was created, deleted or rewritten, not which owners
  or attributes were set right.

**For check 9**: "baseline1" doesn't hold the old three, so restoring it deletes them from
the live system; "6b baseline" holds them (PLAN's harness note).

**Observation a: the config's filter `- home/Downloads` has no leading slash** (read-only,
nothing changed). apsis-test's config has `include_home = true` and two filters, in this
order: `+ /home/<user>/**` and `- home/Downloads`.

- *How it is written* (verified in code and on the disk): `exclude::for_backup` copies a
  filter into the list as it stands; only a `+` filter with an absolute path gets lines for
  its parent folders. The check's snapshot has it as line 58 of `exclude.list`, and the
  restore's filter as its line 111 (the snapshot's list is appended unchanged).
- *What it matches* (verified with rsync 3.2.7 on a temp tree, the main machine): a pattern
  without a leading `/` that contains a `/` is matched against the end of a path, whole
  names. Alone, the rule left out `/home/Downloads` and `/srv/home/Downloads` and copied
  `/home/user1/Downloads` and `/myhome/Downloads`. So it is not the user's Downloads
  folder. On apsis-test, on top of that, `+ /home/<user>/**` comes first and wins for
  everything in that home. The user's Downloads folder is in every snapshot.
- *Whether anything on apsis-test matches it*: unverified. Read-only: `sudo find / -xdev
  -path "*/home/Downloads" 2>/dev/null`.
- *Whose doing* (verified in code, as far as code can say): not a pick. Add Folder and Add
  File (`picked_filter`) refuse a path that isn't absolute and write a folder as
  `<path>/***`. Add Pattern (`typed_filter`) puts `- ` in front of whatever was typed, and
  `validate_signed_filter` accepts any pattern that isn't blank. So it is an entry typed as
  a pattern, or a hand edit of the file; which of the two only the owner knows. The UI did
  what it is written to do: a relative pattern is a legitimate rsync rule (`*.iso`), so it
  can't simply be refused.
- *Candidates, not started*: a hint in the settings when a typed pattern has a `/` in it
  but neither starts with `/` nor has a wildcard ("matches any path that ends this way;
  start it with / to mean one place"). And `+ /home/<user>/**` is redundant while
  `include_home` is on. The owner decides whether either is worth a change.

**Observation b: twelve `kernel:` warning lines with nothing after the colon**, at
16:38:13 in the offline boot (read-only). Not Apsis's and not new:

- Verified from the earlier logs: check 7's verify log has the same twelve (14:43:06, its
  offline boot). Check 1's log, taken without a filter, shows what they are: the empty
  lines inside the kernel's ACPI error dumps for three of the firmware's WMI methods
  (`\_SB.WMID.WQBC`, `WQBD`, `WQBE`: `ACPI BIOS Error (bug): Attempt to CreateField of
  length zero`, then `hp_bioscfg: Returned error 0x4`), four empty lines per dump. Check 5's
  log shows the same dumps in its retry boot.
- Why they showed alone: the runbooks' filter drops the dumps' text lines
  (`acpi|WQB|hp_bioscfg|Local[01]|Arg0`), but its `^\s*$` can't match a journal line whose
  message is empty: the line still starts with its time and `kernel:`. A hole in the
  filter, from check 7's runbook on.
- Unverified: that the same dumps come in a normal boot (expected: the driver loads in
  every boot). Read-only: `journalctl -b 0 -k --no-pager | grep -c WQB`.
- For the next runbooks: add `kernel: *$` to the filter.

**What check 11 established**: on the real ext4 root, as root, a new-format snapshot
restores an owner and group by number, a device node, a file capability, an ACL and a
`user.*` attribute, each after a change that left the file's size and modification time
alone (or, for the node, removed it), with `rsync exited 0` and `done`; the create carries
all five and `--link-dest` links their files only because the attributes match; the
create's and the restore's flags are the ones in `native::rsync_argv` and `argv::rsync`,
seen live. Not shown by it: `--numeric-ids`'s own effect. The three ignored tests stay
ignored.

**Left**: checks 9 and 8, in that order; then the fixes; then the release gate (one
happy-path restore and check 13 on the real 0.5.0 .deb).

## 2026-10-03 - check 9 passed: a snapshot that holds Apsis 0.4.1 restored, 0.4.1 ran, 0.5 installed again showed the result

PLAN 6b.12 check 9, runbook `notes/check-9-runbook.md` (untracked), 17:50 to 18:56. Two
restores: "6b baseline" (`2026-10-01_17-19-36`, the one that holds Apsis 0.4.1), then the way
back, "baseline 0.5.0" (`2026-10-02_10-50-46`). Both with keep home, the safety snapshot on,
the disk in, the real kernelstub, the current build (helper `6d6b01b5…`, dpkg `0.4.2-1`).
What is marked (owner) was seen on apsis-test's screen; the rest is read from the check's
logs.

**The version bump: not before this check (owner; the runbook's branch A).** This supersedes
"the bump comes before check 9" (2026-10-02, "check 1, steps 1a to 1c"). The bump happens
once, with the fixes, for the real 0.5.0 .deb: a bump now would have replaced the build the
other checks ran on and made a `0.5.0-1` that isn't the release. The check holds without
it, because the dialog's line is read from the snapshot's dpkg record (`apsis::in_snapshot`),
not from the running build's version: the same build said 0.4.1 for one snapshot and 0.4.2
for the other (below). **Added to the release gate**: the gate's happy-path snapshot is
taken with the real 0.5.0 .deb installed, and its Restore dialog shows no Apsis line (the
journal's `check-restore` line ends `apsis current`).

**Step 1, the read-only look** (one script with its mode as the first argument, run as
`live` and inside each snapshot, the backup disk mounted read-only):

- **"6b baseline"**: `apsis 0.4.1-1` installed; the new format (`-aAX --numeric-ids`); this
  installation; helper `a0caa94206b09ed3`, applet `678127d4b8ed4c9a`; the bus policy names
  `Helper2`; 5 polkit actions, none of them `restore`. The kernel block equal to the live
  one (7.1.5, 7.0.11 as the previous pair, both kernelstub hooks with
  `--preserve-live-mode`), so this is no kernel rollback. The `/opt` items and ping's
  capability as live. The harness all `same` (2 files in `sudoers.d`, 1 in polkit's
  `rules.d`, 1 in `/etc/netplan`, 0 in `sshd_config.d`). No leftover of checks 0.3 and 0.4.
  The package difference to the live system: exactly two lines, `apsis 0.4.2-1` against
  `0.4.1-1`, of 1745 packages.
- **"baseline1"**: `apsis 0.3.0-1`, the old format, `Helper1`, 7 actions; no `/opt` items,
  no capability on ping; the polkit rule missing; another config. Restoring it would have
  cut the ssh session's access to the helper and deleted check 11's files, as expected.
- **"baseline 0.5.0"**: `apsis 0.4.2-1`, helper `31ced6a22cb00911` (the first 0.5.0 build);
  the way back loses no package.
- **The config inside "6b baseline" is byte for byte the live one.** So an unchanged hash
  after the restore doesn't show that the config is protected. What shows it: the rule
  `- /etc/apsis/***` as line 6 of `restore.filter` at the Ready prompt (lines 1 to 6 were
  the protect list), and the temp-tree test with a real rsync
  (`everything_on_the_protect_list_is_untouched`, `tests/restore.rs`).

**Deviations from the runbook:**

- **`/etc/netplan` added to the harness comparison** (the inside script and the keyboard
  collection). On apsis-test the Wi-Fi connection is a netplan file;
  `/etc/NetworkManager/system-connections` has 0 files, so the runbook as written would
  have compared nothing for the Wi-Fi and still said the harness was the same (Claude's
  miss). Live: 1 netplan file, `same` in the snapshot.
- **The gate of step 8 (`OLD-RUNS`) ignored the `list ... failed: ... already mounted on
  /media/...` lines.** By the runbook's letter a `failed:` list from 0.4.1 was a fail. These
  came from the desktop mounting the backup disk (finding 1), after lists that had
  succeeded; no other `failed:` line was there. The owner passed the step on that reading.
- **Step 7's verify ran twice**: the first run went to a shell without the runbook's
  variables (its output was seen, not saved), the second saved the log. Nothing on
  apsis-test changed between the two and nobody was logged in.

**The dialog line** (owner, word for word): `This snapshot has Apsis 0.4.1. Install Apsis 0.5
again afterwards to restore again.` for "6b baseline", and the same line with `0.4.2` for
"baseline 0.5.0". No "Older format" line. The journal:

```
check-restore "2026-10-01_17-19-36" for :1.N: ok; home yes, root no, current format, apsis no-restore:0.4.1
check-restore "2026-10-02_10-50-46" for :1.N: ok; home yes, root no, current format, apsis no-restore:0.4.2
```

**The preparation.** Safety snapshot 1 `2026-10-03_18-03-11`, linked against "baseline
0.5.0", 1.26 GB (12031221760 to 10766716928 bytes free). `restore.filter`: lines 1 to 6 the
protect list, line 35 `- /run/apsis/backup`. The plan: `"home": "keep"`, `"old_format":
false`, kernel 7.1.5.

**The offline boot** (18:10:06 to 18:11:10), `apsis-restore.service`:

| time | line |
|---|---|
| 18:10:07.7 | `Restoring the system. Don't turn off the computer.` |
| 18:10:08.6 | `copying, attempt 1 of 3` |
| 18:11:06.6 | `the copy ended: rsync exited 0` |
| | `refreshing the boot files`, kernelstub's lines (7.1.5, `preserve_live=True`) |
| 18:11:09.99 | `the restore ended: done` |
| | `apply-restore ended: Finished(Done)`, `Deactivated successfully` |

The copy 58 s. ssh answered at 18:11:37, about three minutes after the click on Restart
now (owner). `result.json`: `done`, `2026-10-01_17-19-36`, the safety snapshot
`2026-10-03_18-03-11`, `"home": "keep"`, an empty message. `tools/restore-check.sh`: every
line ok. Check 12's after-restore state: `pop-upgrade-init.service` skipped on
`ConditionPathExists=!/system-update/apsis-helper`, nothing from pop-upgrade in that boot,
`acpid` and `pop-upgrade` `disabled`.

**So the apply finished while the copy replaced Apsis under it.** The process was the
helper copy under `/var/lib/apsis/restore/`, started before the copy; the copy put 0.4.1's
binaries, its bus policy (`Helper2`) and its polkit policy (no `restore` action) in place
of the packaged ones, and the steps after the copy (kernelstub, the result, the cleanup,
the restart) ran as with any other snapshot.

**After the restore**: dpkg `apsis 0.4.1-1`; the helper and the applet hash as the
snapshot's (`a0caa94206b09ed3`, `678127d4b8ed4c9a`); `dpkg -V apsis` clean; no `restore`
polkit action; the Apsis block and the whole package list equal to the snapshot's, the
kernel block and the `/opt` items equal to before (`RESTORED-TO`). Both markers and cowsay
gone, the kept file there. The device node under `/opt` is the one of 2026-10-01 again
(check 11's put-back had left one with a newer time; both baselines hold the older one).
`rsync-log`: 0 error lines. The "before" numbers identical, the config's hash as before.

**0.4.1 running.** (owner) The panel applets started. The window, after `Try again`
(finding 1), showed four rows (the three base snapshots and safety snapshot 1), no
`Restore` button and no result line. Settings showed the backup disk and the two filters:
0.4.1 read the version 2 config the current build had written. The helper's journal: `list
... ok, 4 snapshots` at 18:15:42, 18:15:43 and 18:21:32, and one `refused: busy with another
snapshot operation` (in 0.4.1 a list is a job, so lists refuse each other; fixed in 0.4.2).
The config and everything under `/var/lib/apsis` (`result.json` among it) were the same
bytes and the same inodes before and after 0.4.1 ran (`OLD-RUNS`): 0.4.1 neither reads nor
touches the state folder.

**0.5 installed again.** `sudo apt install -y <the .deb>`, an upgrade: `Unpacking apsis
(0.4.2-1) over (0.4.1-1)`, no script error, the helper `6d6b01b5c303cd03`. After a log out
and in (owner): the status line `System restored to 2026-10-01 17:19`; the tooltip
`Restored from snapshot 2026-10-01_17-19-36. Home folders were kept. Safety snapshot:
none.` ("none" is false here: fix 6, known); the `Restore` button back; four rows. No
`result.json:` line in the helper's journal, and the file's hash unchanged
(`8b5d5a9b502611d2`) from the offline boot through 0.4.1 and the install. Over ssh, `busctl
call ... RestoreResult` answered `(sssx) "done" "2026-10-01_17-19-36" "" 1791015069`: the
number is the second of the `done` line, and the test-only polkit rule covers the ssh
session for this method too.

**The way back** (not part of the pass rule; the run isn't over without it). "baseline
0.5.0" restored the same way: safety snapshot 2 `2026-10-03_18-41-12`, linked against
safety snapshot 1, 0.83 GB (10766716928 to 9940791296); the offline boot 18:45:58 to
18:46:59, the copy 18:46:00 to 18:46:55 (55 s), `done`, `Finished(Done)`; ssh answered at
18:47:25. `restore-check.sh` every line ok, the `pop-upgrade-init` skip line there; dpkg
`0.4.2-1` with the helper `31ced6a22cb00911`. Then the current build with `apt install
--reinstall` (no helper job running): `6d6b01b5c303cd03`. The status line `System restored
to 2026-10-02 10:50`, five rows. The Apsis, kernel, `/opt` and package blocks, the "before"
numbers and the config's hash as before the check (`BACK-TO-TODAY`).

**Cleanup.** The check's files in the home folder and the markers removed; `/opt` not
touched; `dpkg --audit` clean; both safety snapshots deleted from the window, one at a
time (`delete ... done` twice). Three snapshots, 12031221760 bytes free: exactly the number
from before the check.

**Findings, for the triage. None is fixed or started here.**

1. **The desktop mounted the backup disk, and the lists failed.** At the first login after
   the restore of "6b baseline", udisks (the desktop's volume monitor) mounted the backup
   USB read-write under `/media/<user>/<label>`, 4 s after the login. The helper's lists
   then failed with `mount failed: ... already mounted on /media/...`, and the window showed
   that raw error and `Backup disk: not connected`. That was 0.4.1; its raw `apsis-helper:`
   prefix is already gone in 0.4.2. It didn't happen in check 11's session or at the two
   later logins of this check: intermittent, cause unknown. How the foreign mount ended
   isn't in the run's notes. **Unverified: what the current build does with a backup disk
   that something else has mounted.** To check; and a plain message ("the backup disk is
   open in another app; unmount it") would beat the raw error.
2. **The ESP's and `/recovery`'s FAT filesystems log "Volume was not properly unmounted"**
   in 19 of the 47 boots in the journal, both offline boots and the boots after them among
   them. Not new (check 5's log has it). To find out: whether it follows Apsis's restarts
   only or plain reboots too.
3. **0.4.1's Settings: Cancel does nothing when no value was changed** (owner). To check in
   the current build.
4. **The current build: `Restore` with no row selected does nothing and says nothing**
   (owner; no `check-restore` call in the journal).
5. Still open from check 11: the filter `- home/Downloads` without a leading slash (live in
   apsis-test's config), and the blank `kernel:` warning lines.

Not findings: I/O errors on a second USB stick, the owner's own, pulled while mounted at
18:19 (that device only); one panel applet instead of two at the last login (the second
monitor was probably off).

**What check 9 established**: the Restore dialog names the Apsis a snapshot holds, from the
snapshot's own dpkg record; a restore of a snapshot with Apsis 0.4.1 ends `done` although
the copy replaces Apsis's binaries and its bus and polkit policies under the running
apply; afterwards the system is the snapshot's, package list included, and 0.4.1 starts,
reads the config and lists, and leaves `/var/lib/apsis` alone; 0.5 installed over it reads the kept
`result.json` and shows the result. **Not shown by it** (branch A): dpkg saying 0.5.0
after the install, and a snapshot with no Apsis line (both at the release gate); the
config's protection by a changed hash (the two files were the same); a retry boot after a
copy that had already replaced Apsis's files.

**Left**: check 8; then the fixes, with the version bump; then the release gate on the
real 0.5.0 .deb (one happy-path restore of a snapshot taken with that .deb, whose dialog
shows no Apsis line, and check 13).

## 2026-10-03 - check 9, finding 1: the owner didn't unmount the backup disk

A follow-up to "check 9 passed", finding 1, which left open how the desktop's mount of the
backup disk ended before the list at 18:21:32 succeeded. The owner's answer: the only disk
they handled was the second USB stick, their own FAT stick for carrying screenshots to the
main machine (the one pulled at 18:19, under "Not findings"). They did nothing to the
backup disk's mount under `/media`.

So that mount went away without the owner unmounting it, somewhere between the failed
lists and 18:21:32. By what is unknown, and so is whether pulling the other stick had a
part in it. **Unverified**; read-only, the journal of that stretch on apsis-test:

```
ssh apsis-test 'journalctl --no-pager --since "2026-10-03 18:11:30" --until "2026-10-03 18:22:00" | grep -i -E "udisks|/media/|unmount|apsis-helper"'
```

(Its lines carry the host's name, the account's name and the disk's label: none of them
goes into the repo.) For the triage of finding 1 this means two things aren't understood
yet, not one: when the desktop mounts the backup disk, and when it lets go of it.

## 2026-10-03 - check 9, finding 1: what the journal of that stretch shows

The read-only command of the entry above, run by the owner. Its lines, with the host, the
account, the disk's label and its UUID left out (`sda` is the backup disk, `sdb` the
owner's other stick; the helper is 0.4.1's):

| time | line |
|---|---|
| 18:15:42 | the login: `gvfs-udisks2-volume-monitor.service` starts in the user's session |
| 18:15:42 | the helper starts; one list `refused: busy with another snapshot operation`, one `ok, 4 snapshots`, then the kernel's `EXT4-fs (sda): unmounting filesystem` (the helper's own unmount) |
| 18:15:43 | a second `ok` list and its unmount |
| 18:15:46 | `udisksd: Mounted /dev/sda at /media/<user>/<label> on behalf of uid 1000` |
| 18:16:43 | the helper leaves |
| 18:17:58, 18:18:21 | `list for :1.N: failed: mount failed: mount: /run/apsis/backup: /dev/sda already mounted on /media/<user>/<label>.`, and mount's second line (`dmesg(1) may have more information after failed mount system call.`) as a journal line of its own |
| 18:18:52 | udisks mounts `sdb`; 18:19:02 `Cleaning up mount point ... (device 8:16 no longer exists)`: the stick pulled while mounted |
| 18:21:20 | the kernel's `EXT4-fs (sda): unmounting filesystem`; `udisksd: Cleaning up mount point /media/<user>/<label> (device 8:0 is not mounted)`; `udisksd: Unmounted /dev/sda on behalf of uid 1000` |
| 18:21:32 | the helper starts; `list ... ok, 4 snapshots` |

**What it settles, and what it corrects in the entry above:**

- **The mount didn't go away by itself: the owner ended it.** It ended at 18:21:20 with an
  unmount through udisks on behalf of the logged-in account (uid 1000), 12 s before the
  list that worked. Not the helper: it wasn't running, and its own unmounts leave no udisks
  line. The other stick had been gone since 18:19:02. Asked again, the owner corrected the
  answer in the entry above: they did eject a disk around then and can't say which. Every
  journal line of 18:21:10 to 18:21:25 (a second paste) adds this: an ssh session from the
  main machine opens in the very second of the unmount and closes a second later, with no
  sudo line, and another one-second session came at 18:21:12. So the unmount was either a
  click on apsis-test or a command over ssh as the account (`udisksctl unmount` needs no
  root for a mount the account's own session made); these lines can't tell the two apart.
  Either way it was the owner's doing, not the desktop's and not Apsis's. Of the two things
  the entry above calls not understood, one is left: when the desktop mounts the disk.
- **Two lists failed, not "every later list"**: 18:17:58 and 18:18:21, both between the
  desktop's mount and its unmount. The three `ok` lists are outside that stretch.
- **When the mount came**: 4 s after the volume monitor started and 3 s after the helper's
  second unmount. The helper had mounted and unmounted the disk twice within the monitor's
  first second. Whether that is the trigger (the monitor taking a disk that changes state
  while it starts for a newly plugged one) is **a guess of Claude's, unverified**; it would
  fit "intermittent", since it needs a list at just that moment of a login.

**The current build would fail the same way** (read in code, not run): the helper's mount
call is the same in `v0.4.1` and today (`native::mount_argv`: `mount -o
ro,nosuid,nodev,noexec /dev/disk/by-uuid/<uuid> /run/apsis/backup` for a list), and the
error is `mount`'s, for a read-only mount of a filesystem that is mounted read-write
elsewhere. Nothing in the helper looks for a foreign mount first. **Unverified**: what a
create does then; it mounts read-write, which the kernel may allow next to the desktop's
read-write mount, so a create could run while a list can't. For the triage of finding 1:
test both on apsis-test with the disk mounted by hand in the file manager; then decide
between a plain message and using the mount that is there.

## 2026-10-03 - check 8 passed: the recovery drill, the README's steps typed in Pop's recovery with the safety snapshot

PLAN 6b.12 check 8, the last one. Runbook `notes/check-8-runbook.md` (untracked), two
sittings on 2026-10-03. One restore through the window ("baseline 0.5.0",
`2026-10-02_10-50-46`, keep home, the safety snapshot S on), then the README's "If a
restore goes wrong" lines typed by hand in Pop's recovery with S's name, then the way back
to today's state. The build was the current one (helper `6d6b01b5…`, dpkg `0.4.2-1`); no
bump, no fix before it.

**Where this entry's facts come from.** The owner ran every command and wrote the result up
with a second guide (a claude.ai conversation, which also reviewed the runbook's revision);
Claude Code wrote the runbook and its scripts and saw none of the logs. What follows is
that write-up. Where something wasn't looked at or wasn't reported, it says so.

**Verdict: passed**, with the deviations and the gaps listed below. No stop point was
reached, and no stage of the way back was needed. Every pass word of the chain was printed,
in order: `S1-OK`, `SCRIPT-RUNS`, `K-AS-EXPECTED`, `LOOK-OK`, `REDEPLOYED`,
`STUB-LINES-KNOWN`, the ESP check after step 5b's dry run (the write-up calls it
`ESP-UNTOUCHED-BY-STUB-DRYRUN`; the runbook's text has `DRY-RUN-LEFT-THE-ESP`),
`MARKERS-SET`; at the window `PLAN-OK`, `READY-OK`, `RECOVER-AS-CODE`,
`SAFETY-HOLDS-TODAY`, `RESTORED-TO`, `ESP-IDENTICAL`, `GO-DRILL`; in the recovery
`MOUNTED-OK`, `DRY-OK`, `COPIED-OK`, `BOUND-OK`, `BOOTABLE`; afterwards `DRILLED-TO`,
`DRILL-PASSED`, `APSIS-RUNS`, `BACK-TO-TODAY`, `OUT-COPIED`, `NUMBERS-IDENTICAL` three
times, `CONFIG-AS-BEFORE-THE-CHECK`.

**The owner's answers to the runbook's six questions**: two sittings; "finish the same
restore" as a dry run only, no second by-hand restore; the README to the letter,
`update-initramfs -u -k all` included, except for the snapshot's name, an `umount` if the
recovery automounts, and no further line after a failed one; the save for the worst case
includes the netplan file; the ESP's `cmdline` file stays as the recovery leaves it; the
runbook's `bootback` mode only if the way back's stage 1 is reached and only after the
state has gone to the guide and the guide agrees (never needed).

### Sitting 1: what was settled before anything was changed

- **The baseline** (a read-only look, `S1-OK`): `apsis 0.4.2-1`, helper `31ced6a22cb00911`,
  applet `cba00afb3a45590a`; its kernel, `/opt` and package blocks equal the live ones
  (1745 packages, none different); the harness the live one (2 files in `sudoers.d`, 1 in
  polkit's `rules.d`, 1 in `/etc/netplan`, 0 in NetworkManager's `system-connections`, 0 in
  `sshd_config.d`); no leftovers. `/home` is part of the root. The config's hash
  `1e8d387f39671970`.
- **kernelstub 3.1.4 and the hooks, read on apsis-test** (the runbook had four of these
  from Claude's memory of the upstream source; three were right, one was corrected):
  - Live mode is a key in `/etc/kernelstub/configuration`, not a look at the environment.
    `application.py:145-157`: with `--preserve-live-mode` and `live_mode` true it warns and
    exits 0; otherwise it sets `live_mode` false. The installed configuration has
    `live_mode` false, `manage_mode` true, `setup_loader` true. Nothing in the package
    mentions casper, `boot=` or `/cdrom`.
  - The root and the ESP come from `/proc/mounts` and `findmnt -n -o UUID --mountpoint`
    (`drive.py:80`, `108`). Corrected: not from a listing of `/dev/disk/by-uuid`.
  - The loader entry's options line is built from the configuration
    (`application.py:254`, `325`, `installer.py:175`), never from `/proc/cmdline`. Only
    `copy_cmdline` (`installer.py:219-221`) reads `/proc/cmdline`, for the ESP's `cmdline`
    file.
  - `opsys.py:37` reads the running kernel's version (`platform.release()`); no use of it
    was found, and the drill showed none (below).
  - `update-initramfs` (initramfs-tools 0.142ubuntu25.5pop0): `-k all` through
    `get_sorted_versions`, `post-update.d` through `run_bootloader`, `backup_initramfs=no`,
    `MODULES=most`, `COMPRESS=zstd`. The two initrds are 214 MB each.
- **The recovery, from a look-only boot** (the root mounted by the README's first line, one
  script run, unmounted, restarted): Pop!_OS 24.04 LTS on **kernel 7.0.11, the same version
  as the installed system's previous kernel**; the live user has uid 1000 and sudo asks no
  password; 117 UEFI variables; rsync 3.2.7 with ACLs and xattrs (the installed one is
  3.2.7 too); no network connected and no ssh server; `getfattr` absent; the recovery
  partition mounted read-only at `/cdrom`; its clock in UTC. **The backup USB was not
  automounted**, in the look and in the drill, although the desktop's automount setting is
  on (a second stick the owner plugged in was automounted). The recovery's boots leave
  nothing in the installed system's journal: its boot list has no entry for them.
- **The saves for the worst case**: one archive of the harness files and the three `/opt`
  files and the list of manually installed packages, in a private folder on the main
  machine, never in the repo. The owner deletes it when the release gate is done.
- **The ESP before the check** (sha256, first 16 characters; bytes):

  | file | sha256 | bytes |
  |---|---|---|
  | `vmlinuz.efi` | `3ee40cb8f37bf9cb` | 17273344 |
  | `initrd.img` | `6b63d5e0ee34f38d` | 214288357 |
  | `cmdline` | `7132d2360e192580` | 167 |
  | `vmlinuz-previous.efi` | `efe0806b74477346` | 17056256 |
  | `initrd.img-previous` | `693e72765757298c` | 214015141 |
  | `Pop_OS-current.conf` | `c040f6946b412da4` | 257 |
  | `Pop_OS-oldkern.conf` | `d844b1a73f1cc5d5` | 275 |

  The current pair was `/boot`'s 7.1.5 pair, the previous pair the `.old` links' 7.0.11
  pair.

### The restore through the window

- 20:55:31 the dialog (as the runbook has it): `check-restore … ok; home yes, root no,
  current format, apsis no-restore:0.4.2`. **No password is asked on apsis-test**: the
  harness's test-only polkit rule covers `restore` too.
- 20:57:26 `restore … keep-home safety: started`; the restore's dry run 41 s, S's dry run
  15 s, S itself 4 min 45 s; `ready` 6 min 28 s after `started`. S cost 1314189312 bytes
  (12031221760 to 10717032448 free), linked against the newest snapshot.
- **At the Ready prompt, read-only**: the plan named the baseline, `"home": "keep"`,
  `"old_format": false`, kernel 7.1.5, no link yet. **`RECOVER.txt` on the backup disk
  equals `recover::text` for this restore byte for byte** (compared with the three UUIDs
  replaced by placeholders). The two snapshots' `exclude.list` files are the same bytes
  (61 lines), so the saved filter was also right for S. `restore.filter`: 114 lines, the
  six protect lines first, `- /home/***`, one `/root` rule, five rules for kernel 7.1.5, no
  mount rule outside `/dev`, `/proc`, `/sys`, `/run`. S held the ESP's seven files and
  `/boot`'s four kernel and initrd files as they were live, the installed helper build,
  both markers and cowsay.
- Armed 21:08:32. The offline boot: `apsis-restore.service` 21:09:24 to 21:10:24, the copy
  55 s (`copying, attempt 1 of 3`, `rsync exited 0`), the boot refresh 4 s (one kernelstub
  run), the journal in the designed order, ending `apply-restore ended: Finished(Done)`.
  ssh answered at 21:10:55.
- After it: `tools/restore-check.sh` every line ok; `result.json` `done`, the baseline, S,
  keep home (hash `8389da1773edd972`); `rsync-log` 378 lines, 0 error lines; the baseline's
  builds installed (helper `31ced6a2…`); both markers and cowsay gone; **the ESP's seven
  hashes as before the check**; `pop-upgrade-init` skipped on
  `ConditionPathExists=!/system-update/apsis-helper`.

### The drill in the recovery

Times are the recovery's clock (UTC; the installed system is 10 hours ahead).

| | line | result |
|---|---|---|
| | the UUIDs from `lsblk -f`, completed with Tab | |
| 1 to 3 | the three mount lines (root, ESP, backup read-only) | no output |
| | the script's `mounted`, 11:30 | the three mounts are the ones `RECOVER.txt` names; nothing armed; the ESP as after the restore |
| | the script's `dry`, 11:32 | both rsync lines as dry runs, below |
| 4 | the rsync line with S's name | ran twice (deviations); the second exited 0 |
| | the script's `copied`, 11:42 | a further pass as a dry run: 0 lines. Both markers, cowsay and the helper `6d6b01b5…` back; `result.json`, the config and the ESP untouched by the copy |
| 5 | the four `--rbind` mounts | no output |
| | the script's `bound`, 11:44 | the chroot sees `/` and `/boot/efi`, the UEFI variables are there; kernelstub's dry run below |
| 6 | `chroot /mnt update-initramfs -u -k all` | exit 0, about 2 minutes, no `W:` and no `E:` line |
| 7 | `chroot /mnt kernelstub --verbose`, 11:49 | exit 0; the ESP byte for byte as the hooks had left it |
| 8 | `rm -f /mnt/system-update` | no output (nothing was armed: a no-op) |
| | the script's `boot`, 11:50 | the ESP's current pair is what `/boot` links to; `BOOTABLE` |
| | "Restart." | the desktop panel's restart; nothing was unmounted by hand |

**The two dry runs before the copy.**

- "Finish the same restore" (the baseline's name): exit 0, 213 lines, 104 changes (73
  changed files, 11 deleted, 20 new folders; 204 under `var`, 8 under `etc`). More than
  the "few or none" the runbook expected, and all of it the live system's own writing
  since the offline boot: CUPS's state, `vconsole.conf`, kernelstub's configuration,
  rotated dpkg backups, chrony's cookies, logrotate's status, the empty
  `system-update.target.wants` folder. It was not run for real.
- "Pick the safety snapshot": exit 0, 331 lines, 169 changes (82 changed files, 5 deleted,
  58 new files, 22 new folders, 2 new links; 239 under `var`, 82 under `usr`, 9 under
  `etc`). Nothing under `/home`, the ESP, `/recovery`, `/var/lib/apsis`, `/etc/apsis`;
  not `fstab`, not `crypttab`; nothing for `/opt`, sudoers, polkit, netplan, ssh, `passwd`,
  `shadow` or `group`; nothing for `/boot`. Lines for both markers, cowsay and the helper.

**kernelstub inside the chroot.**

- It did not follow the running kernel. The recovery runs 7.0.11; the dry run named
  `vmlinuz-7.1.5` and `initrd.img-7.1.5` (the newest in `/boot`, which the links name) and
  the installed configuration's options, with nothing of the recovery's command line.
- `bootctl is-installed` says yes inside the chroot, `ischroot` knows it is one, and **no
  `kernel-install` line appeared** during `update-initramfs`.
- **`update-initramfs -u -k all` ran kernelstub twice, once per kernel**, both times with
  `--preserve-live-mode`, both writing the 7.1.5 pair. The README's own `kernelstub
  --verbose` then changed nothing. So three writes of the ESP, not the five the runbook
  expected from PLAN 6b.6 and the checks 0.1 to 0.3 entry ("twice per kernel", read from
  the hooks, never counted in a run). Why `kernel-install`'s path didn't reach kernelstub
  here, and whether it does on the installed system outside a chroot, is **not known**.
- kernelstub saves its configuration file on every run (the time changes, not the bytes).

**What the README's lines changed on the ESP and in `/boot`** (the script's diff, nothing
else differed):

| file | before | after |
|---|---|---|
| `initrd.img` | `6b63d5e0ee34f38d`, 214288357 bytes | `898ffd73f4349f62`, 214288328 bytes |
| `initrd.img-previous` | `693e72765757298c`, 214015141 bytes | `661395e0b26df26c`, 214014854 bytes |
| `cmdline` | `7132d2360e192580`, 167 bytes | `69a070220c8681b5`, 224 bytes |

`/boot`'s two initrds changed the same way. `vmlinuz.efi`, `vmlinuz-previous.efi` and both
entry files have the bytes of before (the entries were rewritten with the same bytes). The
`cmdline` file now holds the recovery's command line (its initrd, `boot=casper`, a host
name parameter, `noprompt`, the live medium): kernelstub copies `/proc/cmdline`, and in the
chroot `/proc` is the recovery's. Nothing boots from that file; the entries' options come
from kernelstub's configuration. Left as it is (the owner's answer); the next kernelstub
run from the installed system writes it again.

### The first normal boot, and the system afterwards

- It started by itself at 21:54:19 (local), no menu, no text console, no wait; ssh
  answered 8 s later.
- **The system was S's** (`DRILLED-TO`): the Apsis block, the whole package list, the
  kernel block, the `/opt` items and both markers with cowsay equal S's, and S's harness
  files equal the live ones. Helper `6d6b01b5c303cd03`, dpkg `0.4.2-1`. `result.json`'s
  hash unchanged by the drill, the config's hash `1e8d387f39671970`, 0 failed units,
  `restore-check.sh`'s part 3 (the ESP against `/boot`, the modules, the entries) all ok.
- The live tree against S after that boot, as a read-only dry run with the saved filter:
  101 lines, 84 changes, 82 of them under `var`; outside it `boot/initrd.img-7.0.11`
  (rebuilt; 7.1.5's has no line because the filter's kernel rule keeps it out), ten
  `modules.*` files of 7.0.11 with new times (depmod), `vconsole.conf`, CUPS's state and
  kernelstub's configuration. Nothing under `usr/bin`, `usr/sbin`, `usr/libexec`, `opt`,
  sudoers, polkit or netplan.
- **Apsis afterwards**: an applet process running, the helper's `list … ok, 4 snapshots`,
  `check-restore` ok for the baseline. At that look the dialog's own Restore was clicked
  by mistake: **Stop worked** (`stopped`, rsync's group ended, nothing armed, nothing
  written, four snapshots and the same free bytes afterwards). So `APSIS-RUNS` is a
  mechanical word in this run.
- **The way back**: the simulated purge named cowsay alone; both markers and cowsay
  removed; `BACK-TO-TODAY` (Apsis, the package list, the kernel block, the `/opt` items
  and the empty leftovers block as before the check), the numbers and the config's hash
  as before. The recovery's 25 log files copied to the main machine and compared by hash
  before the check's folder was removed. S deleted from the window. **The USB: the three
  base snapshots, 12031221760 bytes free, exactly as before**; `RECOVER.txt` still there,
  naming the baseline.

### The README against what had to be typed

1. **The UUIDs came from `lsblk -f` only.** `RECOVER.txt` is on the disk that the third
   line mounts, so it can't supply the first three lines (F5). A first attempt at a mount
   line was wrong (a character of the UUID and ` /mnt` missing) and was caught before
   Enter. Paths by UUID and Tab completion worked.
2. No `umount` was needed: the recovery didn't automount the backup USB.
3. **S's name isn't in `RECOVER.txt`** (0 times, F3). It came from the plan at the Ready
   prompt, the runbook's `names` file and the script's printed line.
4. **The rsync line as printed restores the baseline again, not S**; the name has to be
   changed by hand. In this run the three-line command was taken from the script's output
   with S's name already in it (a deviation from "edit `RECOVER.txt`'s text"). It pasted
   as one command and ran at once; the README's one-line form was then typed as well.
   Both forms work as one command.
5. `sudo` never asked for a password.
6. `update-initramfs -u -k all`: both initrds rebuilt, kernelstub twice, about 2 minutes.
7. `kernelstub --verbose`: nothing the hooks hadn't done.
8. "Restart." was the panel's restart; the README says nothing about unmounting, and
   nothing was unmounted.
9. **What the README doesn't say and a person would need**: nothing about the installer's
   window that the recovery opens (a click on its install choices is a reinstall); nothing
   about what a failing line means; no check that the result boots (the runbook's script
   did that); no way to find the safety snapshot's name; no ssh in the recovery, so
   whatever goes wrong is read from the screen.

### Findings. None is fixed or started; all go to the triage with the fixes

**From reading the code before the run (the runbook's F1 to F8):**

| | finding | this run |
|---|---|---|
| F1 | the saved filter always keeps out the kernel that ran when the restore was prepared (`prepare.rs:185`; nothing rewrites `restore.filter`). After a kernel rollback the safety snapshot holds that kernel, and the README's line with the saved filter would not bring its modules and `/boot` files back, while dpkg's record and the `/boot` links name it | read only: 7.1.5 on both sides |
| F2 | "finish the same restore" by hand doesn't finish a rollback: the kept kernel stays, no README line removes it, and the bare `kernelstub --verbose` takes the newest kernel in `/boot`, where the apply names the snapshot's | read only |
| F3 | `RECOVER.txt` never names the safety snapshot, although it is written after the snapshot is made (`recover::text` gets only the restored snapshot's name) | shown |
| F4 | `RECOVER.txt`'s `-A -X` follows the restored snapshot's format; a safety snapshot is always new format, so after restoring an old-format snapshot the note's line is wrong for it | read only: both new format |
| F5 | `RECOVER.txt` is on the disk whose mount line it contains | shown |
| F6 | the README's steps rebuild both initrds and rewrite the previous pair on the ESP, so "the Pop_OS-oldkern entry is never touched by a restore" doesn't hold for the README's own recovery steps | shown: `initrd.img-previous` has new bytes |
| F7 | by hand there is no backup of the ESP and no check of what was written; the apply has both | a statement about the README's text; the runbook's script supplied the check |
| F8 | `result.json` is protected, so after a by-hand restore the window still names the restore that was undone | the mechanism shown (`result.json`'s hash unchanged by the drill, still naming the baseline while the system is S's); the window's line itself was not reported |

Also from reading: with `restore.filter` missing, rsync 3.2.7 stops with exit 11 before it
copies or deletes anything (run on the main machine, not on apsis-test), so the README's
line fails safely and then offers nothing. Relevant to the planned removal of the filter
after a refused restart: after a refusal nothing needs recovering, but the filter must
stay after a restore that ran.

**New from the run (the write-up's N1 to N8):**

- **N1. The FAT "Volume was not properly unmounted" lines** (check 9's finding 2) for the
  ESP and the recovery partition appear at every boot since check 5, also after clean
  unmounts (systemd unmounted both at 21:09:03; the boots at 21:09:23, 21:10:44 and
  21:54:19 warned). The recovery doesn't cause them: the boot after the look-only session
  warned although that session never mounted the ESP. Consistent with the vfat driver
  leaving a dirty flag that is already set until `fsck.fat` clears it, with check 5's
  power cut as the candidate. **Unverified**; `fsck.fat` was not run.
- **N2.** Preparing a restore deletes the previous restore's `rsync-log` (as designed:
  `clear_leftovers` in the helper's `prepare.rs`). A Stop leaves a `restore.filter` of the
  stopped preparation and no `request.json`. So the filter in the state folder is the
  last preparation's, not necessarily the last restore's: after a restore, a preparation
  that is then stopped or cancelled replaces the filter the README's line would use.
- **N3.** Each restore leaves an empty `/etc/systemd/system/system-update.target.wants/`
  (the arm makes the folder; `remove_arm_files` removes the link in it, not the folder).
- **N4.** The mistaken click on Restore, and Stop working (above).
- **N5.** The recorded terminal session is noisy (history keys, typed-ahead lines); the
  script's per-step logs are the reliable record. `sudo script` fails on a file in `/tmp`
  owned by another user; plain `script` worked.
- **N6.** `/etc/vconsole.conf` is a regular file in S; each boot makes it a link to
  `default/keyboard`, and the desktop login writes a regular file again. Harmless churn
  that a dry run shows as a new file.
- **N7.** The greeter boot's warnings (COSMIC's theme watchers, a D-Bus denial for
  `pop-upgrade-notify`, wireplumber's assertions) were not compared with an earlier
  normal boot.
- **N8.** With no applet running (the greeter only), no `apsis-*` unit is loaded.

### Deviations from the runbook

- Sitting 1: an env file sourced per shell (the main machine's `ls` and `cat` are aliased;
  `grep` and `diff` are colour wrappers, left as they are); two `| tee` replaced by a
  redirect; two more read-only greps of kernelstub's source into files of their own; the
  look's pre-check also asked for `K-AS-EXPECTED`; the first command in the recovery
  mistyped once (sudo printed its usage, nothing ran).
- After sitting 1 the runbook took seven proposals from the owner's side and two review
  notes: the script reads kernelstub's dry-run lines for the kernel, the initrd and the
  boot options itself; the drill starts only with `ESP-IDENTICAL`; the cleanup only after
  the recovery's logs are on the main machine with the same bytes; a simulated install
  and purge showing cowsay alone; "Cancel, never Restore" where a dialog is only looked
  at; power and an idle inhibitor before the long lines; an ESP check after the dry run
  on the installed system. The script was copied to apsis-test a second time for it.
- **The rsync line ran twice, both times with S's name** (item 4 above). The first run's
  exit status was not captured; the second exited 0 and the pass after it found nothing
  left to do.
- At step 13 the dialog's Restore was clicked instead of Cancel (N4).

### What check 8 established

The README's commands, typed in Pop's recovery as it is on this machine, turned a system
restored to one snapshot into the safety snapshot's system and left it starting by itself:
the recovery has what the lines need (rsync with ACLs and attributes, sudo, the UEFI
variables, the disks by UUID); the saved filter works unchanged against `/mnt/` and keeps
`/home`, the config, the state folder, the ESP, `fstab`, `crypttab`, `/opt` and the harness
out of the copy and of the deletions; `update-initramfs` and
kernelstub work in a chroot entered from the recovery, with kernelstub taking the newest
kernel in `/boot` and the installed configuration's options, not the running kernel and
not the recovery's command line; `RECOVER.txt` on the backup disk is `recover::text`
word for word; and Apsis lists and checks a restore afterwards.

**Not shown by it:**

- a system that doesn't start: nothing was broken, the root, the ESP and the installed
  tools were good before the first line was typed;
- a restore that failed or broke mid-copy (a mixed tree, the link still there);
- a reinstall, and so the saved harness archive, which was never used;
- a kernel rollback by hand (F1, F2), an old-format snapshot (F4), a restore with home
  restored, a separate `/home`, an encrypted root, a live USB in place of the recovery
  partition, a person who has never seen the steps;
- **`--numeric-ids`, ACLs, attributes and capabilities coming through the recovery's
  rsync**: the files that carry them (the three under `/opt`, ping) were the same in the
  baseline and in S, so the by-hand copy had no line for them. They were as before
  afterwards, which shows only that the copy left them alone. Check 11 showed them for the
  apply, not for the recovery;
- **the systemd-boot menu's entries**: not read word for word in either sitting (the owner
  recalls a new-kernel entry, an old-kernel entry and Recovery);
- **the window after the drill**: the status line, its tooltip, the rows, the `Last
  snapshot` and `Backup disk` lines were not reported;
- **editing `RECOVER.txt`'s own rsync line before it runs**: the line was taken from the
  script with the name already in it;
- the first rsync run's exit status and the copy's duration in the recovery;
- whether the idle inhibitor of the runbook worked: not reported;
- the runbook script's `bootback` mode and its by-hand equivalent: never run on a real
  machine.

### Left on apsis-test

Today's system as before the check (the package list, the kernel block, the `/opt` items,
the config, helper `6d6b01b5…`, dpkg `0.4.2-1`), **except**: both initrds in `/boot` and
the two initrds on the ESP are the ones rebuilt in the chroot (the filter's kernel rule
keeps the running kernel's initrd through every later restore, so 7.1.5's stays until the
next kernel or initramfs update); the ESP's `cmdline` file holds the recovery's command
line; `result.json` is this check's restore; kernelstub's configuration and log are
rewritten. By N2 and N4 the state folder should now hold `result.json` and the stopped
preparation's `restore.filter` and no `rsync-log`: an inference of Claude's, not read
back. No marker, no cowsay, no file of the check; `/opt` untouched. The USB: the three
base snapshots, 12031221760 of 30120226816 bytes free.

**Left**: every check of PLAN 6b.12 has now passed (13 is rerun at the gate). Next the
triage of this entry's findings together with the fixes of the two triage entries, the
version bump, and the release gate on the real 0.5.0 .deb (one happy-path restore of a
snapshot taken with it, whose dialog shows no Apsis line, and check 13).

## 2026-10-04 - corrections to the check 8 entry (owner)

The entry above ("check 8 passed") stands as written; these four points correct it.

1. **`ONLY-COWSAY` was printed.** It is step 6's simulated install (apt would install
   cowsay and nothing else) and the gate for the markers: `MARKERS-SET` requires it. The
   entry's list of pass words left it out. It belongs to sitting 2.
2. **Which words belong to which sitting.** Sitting 1 is the runbook's steps 1 to 5:
   `SCRIPT-RUNS`, `S1-OK`, `K-AS-EXPECTED`, `LOOK-OK`. Sitting 2 starts at step 5b:
   `REDEPLOYED`, `STUB-LINES-KNOWN`, the ESP re-check word, `ONLY-COWSAY`, `MARKERS-SET`,
   and then the window's, the recovery's and the later words as the entry lists them. The
   entry gave one list with no boundary between the sittings.
3. **The ESP re-check word.** `ESP-UNTOUCHED-BY-STUB-DRYRUN`, the word the owner saw, is
   the second guide's own: its re-check of the ESP after step 5b's kernelstub dry run,
   saved as the "esp after stub" log. The runbook's name for that check is
   `DRY-RUN-LEFT-THE-ESP`. One check, two names; the entry gave both without saying whose
   each was.
4. **The state folder after the mistaken Restore and Stop was read back, not inferred.**
   After the Stop at step 13 (22:03) it held `restore.filter` (2181 bytes) and
   `result.json` (hash `8389da1773edd972`, this check's restore) and nothing else: no
   `rsync-log`, no `request.json`. So in "Left on apsis-test" the sentence marked "an
   inference of Claude's, not read back" is **shown**, and with it N2: the preparation
   removed the previous restore's `rsync-log`, and the Stop left the stopped preparation's
   filter and no plan.

## 2026-10-04 - triage of checks 1 to 9 and the check 8 findings, and the owner's decisions

Every check of PLAN 6b.12 has passed (13 is rerun at the gate). This entry is the triage of
what they found: the numbered fixes and their companions from the two triage entries of
2026-10-03, check 9's findings, check 8's F1 to F8 and N1 to N8, the nine "README against
what had to be typed" items, and the kernelstub count. **Nothing here is built yet**, and
the version isn't bumped. The older entries stand as written.

**How to read the tables.** Evidence: **V** = seen on apsis-test or in a run; **R** = read
in the code or reasoned, never run. Decision: **before** = fixed in code before 0.5.0;
**docs** = README or docs only, before 0.5.0; **after** = after 0.5.0; **no fix** = not a
fix. Every decision is the owner's, marked (owner): stated on 2026-10-03 (the two triage
entries), or on 2026-10-04, when the owner decided the pulls and the fixes' shapes and
confirmed Claude's recommendations for all the other rows. Nothing is left open; the last
three points to be decided are listed at the end.

### The items of the two earlier triage entries

| | item | found | evidence | decision | why, or how |
|---|---|---|---|---|---|
| 1 | fix 1: the `prerm` stops the helper mid-job | "a reinstall during a delete" | V | before (owner) | "Fix 1 and 1b" below |
| 1a | the window says nothing when the helper vanishes mid-job | the same entry | V | after (owner) | with fix 1 the script no longer cuts a job; what is left is a helper that crashes |
| 1b | a package operation while a restore is armed | reading for check 7, `notes/armed-upgrade.md` | R; that the timer's service fails once the packaged helper is gone is unverified | before (owner) | "Fix 1 and 1b" below |
| 2 | fix 2: a half-deleted snapshot is invisible and stuck | the same entry as fix 1 | V | before (owner) | "Fix 2" below |
| 3 | fix 3: a refused Restart now is logged `failed:` | check 4 | V | before (owner) | `Error::RestoreRefused` joins `describe_error`'s refused list. It then also covers the preparation's refusals, all of which come before anything is written |
| 4 | fix 4: the status area stays `Preparing restore · ready` after a refused restart | check 4 | V | before (owner) | `on_restart_answered` sets the status when it opens the refusal dialog |
| 5 | fix 5: `problems` shows the window line of `done` | check 6 | V | before (owner) | a line of its own that says some files weren't restored and names the log |
| 6 | fix 6: the result tooltip's home and safety-snapshot lines are fixed strings | check 6; seen false in check 9 ("Safety snapshot: none") | V | before (owner): carry both fields | "Fix 6 and RECOVER.txt" below |
| 7 | `boot-broken` shown as `boot-kept` | reading for check 7 | R | before (owner) | a line and tooltip of its own; "put back" is false for it |
| 8 | the `boot-kept` tooltip names the wrong cause | check 7 | V | before (owner) | wording that covers a kernelstub that failed and a check that failed; the README's two places that say "didn't check out" change with it |
| 9 | nothing of rsync's errors is logged on a plain exit 23 | check 6 | V | before (owner) | "Item 9 and N3" below |
| 10 | `restore.filter` left behind after a refused restart | check 4 | V | before (owner), as part of Pull 1 | "Pull 1" below |
| 11 | no `boot-kept` case in `tools/restore-check.sh` | reading for check 7; check 7 showed it can't tell | R | after (owner, 2026-10-03) | due before any check that could end `boot-kept` across a kernel change; it can't be exercised without such a restore |
| 12 | the refusal at Restart now with `/pop-upgrade` present | check 7's step 0, skipped on purpose | not run | at the release gate (owner) | the regression test for fixes 3 and 4 and for Pull 1's removal |
| 13a | Stop logs `stopping, SIGTERM ...`, the runbook expected `ok` | check 4 | V | no fix (owner) | the runbook's expectation was the wrong one; the line says more than `ok` |
| 13b | a refused restart logs `plan removed: request.json`, not `plan removed (refused ...)` | check 4 | V | before (owner) | falls out of Pull 1: one path removes a plan, with one wording |
| 13c | `failed:` where `refused:` is due | check 4 | V | fix 3 | |

### Check 9's findings

| | item | evidence | decision | why, or how |
|---|---|---|---|---|
| C1 | the desktop mounts the backup disk and the lists fail with `mount`'s raw error | V on 0.4.1 (the journal); R for the current build (the same `mount_argv`); a create or a restore's preparation beside a foreign mount: unverified; what makes the desktop mount it: unknown | a probe at the release gate (owner); one README troubleshooting row (owner); the plain message and "use the mount that is there": after (owner: nothing beyond the probe in 0.5.0) | the probe decides whether a write path misbehaves; if one does, that is a new finding |
| C2 | the FAT "Volume was not properly unmounted" lines (also N1) | presence V; the cause (a dirty flag left by check 5's power cut) unverified | no fix (owner) | not Apsis's; `fsck.fat -n` was **not** run; nothing about their cause goes into the README |
| C3 | Settings' Cancel does nothing when no value was changed | seen on 0.4.1; R for the current build: the button is disabled until a value changes (`on_press_maybe(dirty && !busy)`), the back arrow leaves | before (owner) | Cancel is always clickable and leaves the page |
| C4 | Restore with no row selected does nothing and says nothing | R: the button is disabled without exactly one real snapshot selected, as Delete is; that fits "no `check-restore` call" | no fix in 0.5.0 (owner) | PLAN 6b.8 says no tooltip; the same behaviour as Delete |
| C5a | the filter `- home/Downloads` without a leading slash | V (the code; rsync on a temp tree on the main machine); whether anything on apsis-test matches it: unverified | docs: one sentence in the README's Filters section (owner); a hint in Settings: after (owner) | a relative pattern is a legitimate rsync rule and can't be refused |
| C5b | the blank `kernel:` warning lines | V | no fix (owner) | the firmware's ACPI dumps; the runbooks' filter gained `kernel: *$` |

### Check 8: F1 to F8

| | item | evidence | decision | why, or how |
|---|---|---|---|---|
| F1 | the saved filter keeps out the kernel that ran at the preparation, so a by-hand go-back after a rollback doesn't bring that kernel's files back | R | docs now, the fix after (owner) | "Pull 2" below |
| F2 | "finish the same restore" by hand doesn't finish a rollback | R | docs now, the fix after (owner) | "Pull 2" below |
| F3 | `RECOVER.txt` never names the safety snapshot | V | before (owner) | "Fix 6 and RECOVER.txt" below |
| F4 | the note's `-A -X` follows the restored snapshot's format | R | before (owner) | each command gets its own flags |
| F5 | the note is on the disk whose mount line it holds | V | before (owner) | the arm keeps a copy of the note in the state folder, readable after the first mount line; the README says the UUIDs come from `lsblk -f` |
| F6 | "Pop_OS-oldkern is never touched" is false for the by-hand steps | V | docs (owner) | the sentence is corrected in the README, the note and PLAN 6b.11; `update-initramfs -u -k all` stays in the steps, as drilled |
| F7 | by hand there is no ESP backup and no check of what was written | a statement about the text | docs (owner) | the README gains the compare lines the drill's script ran, and says that no backup of the ESP is made by hand |
| F8 | `result.json` is protected, so after a by-hand restore the window names the restore that was undone | the mechanism V; the window's line was not reported | docs (owner): one sentence; no fix in code | the protection is on purpose |

### Check 8: N1 to N8

| | item | evidence | decision | why, or how |
|---|---|---|---|---|
| N1 | the FAT lines | as C2 | no fix (owner) | as C2 |
| N2 | a preparation deletes the last restore's `rsync-log`, and a stopped one leaves its own filter in place of the last restore's | V (the corrections entry, point 4) | before (owner) | "Pull 1" below |
| N3 | each restore leaves an empty `/etc/systemd/system/system-update.target.wants/` | V | before (owner) | "Item 9 and N3" below |
| N4 | the mistaken click on Restore; Stop worked | V | no fix (owner) | what it cost is N2 |
| N5 | the recorded terminal session is noisy | V | no fix (owner) | a runbook convention: the script's per-step logs are the record |
| N6 | `/etc/vconsole.conf` changes at every boot and login | V | no fix (owner) | a runbook expectation for dry runs |
| N7 | the greeter boot's warnings weren't compared with an earlier boot | not looked at | no fix (owner) | |
| N8 | with no applet running, no `apsis-*` unit is loaded | V | no fix (owner) | the helper is started by the bus on demand, by design |

### The README against what had to be typed

| | item | decision |
|---|---|---|
| R1 | the UUIDs came from `lsblk -f` only | docs (owner), with F5 |
| R2 | no `umount` was needed in Pop's recovery; another recovery or a live USB may automount the backup disk (unverified) | docs (owner), worded as "if it is mounted already" |
| R3 | the safety snapshot's name is nowhere | F3 (owner), and one read-only line in the README to find it by hand (owner) |
| R4 | the printed rsync line restores the same snapshot again; the README's one-line form has run-together spaces | docs (owner): the note's two labelled commands |
| R5 | `sudo` asked no password | no fix |
| R6 | `update-initramfs` takes about two minutes and rebuilds both initrds | docs (owner) |
| R7 | `kernelstub --verbose` changed nothing after the hooks | no fix (owner); the line stays |
| R8 | "Restart." says nothing about unmounting | docs (owner): the panel's restart, nothing unmounted, as drilled |
| R9 | not said: the installer window the recovery opens, that a failing line means stop, a check that the result boots, that there is no ssh | docs (owner) |

### Pull 1, as decided (owner): the filter's lifetime

Item 10 pulled against the README's rsync line, which needs the filter of the restore that
ran, and against N2: today `restore.filter` belongs to the last preparation, not to the
last restore, and a preparation also deletes the last restore's `rsync-log`.

- **`restore.filter` stays the preparation's working file.** It is removed on Stop, on
  Cancel, on a refusal (while preparing or at Restart now), and with the arm's files after
  a finished restore or a disarm.
- **The note has a working copy too, `restore.note`.** The preparation writes it beside
  `restore.filter` in the state folder, with the text it writes to the backup disk. It has
  exactly `restore.filter`'s lifetime and removal sites, and the protect list covers it.
- **Where the two working files are removed** (the call sites were read on 2026-10-04):
  on Stop, Cancel and a refusal; and next to the removal of `request.json` in core's
  `clean_up` and `give_up` (the apply's ends, after `result.json` and step `end` are saved
  and the link is gone), in the helper's `disarm()` and in `clean_at_start()`. **Never
  through `remove_arm_files`**: `clean_leftovers` calls that from `check_and_arm` right
  before the arm, on the plan that is about to be armed, and would delete the files the
  arm is about to copy. With this, no call removes them before the apply has ended: a
  `Retry` end and a `LinkStuck` end remove nothing.
- **The arm keeps a pair.** Before anything of the arm is written it copies
  `restore.filter` to `last-restore.filter` and `restore.note` to `last-restore.note`,
  all in the state folder, each copy written under a temporary name and renamed (the state
  folder's own writer). **The arm is refused if either copy fails**: no link, the plan
  dropped, an error in the status line.
- **`rsync-log` is cleared at the arm**, not at the preparation.
- **"Last restore" means "last arm."** An arm that is disarmed, or a restore that ends
  `not-started`, has still replaced the pair and cleared the log.
- **The note on the backup disk carries one line saying which file to trust.** It is
  written at every preparation, so after a restore followed by a preparation that was
  stopped, cancelled or refused it names that later one. The pair in the state folder is
  the last arm's and is the one to trust.
- **`RECOVER.txt` and the README name `last-restore.filter`.** With no such file (never
  restored, or after a purge) the by-hand rsync exits 11 before it copies or deletes
  anything (run on the main machine, 2026-10-03); the README says so.
- **The new files are covered by the existing protect list** (`restore.note`,
  `last-restore.filter`, `last-restore.note`): `/var/lib/apsis/***` matches the state
  folder and everything in it, for the apply and for the by-hand line.
- Item 13b falls out of this: a refused restart goes through `remove_plan`, with its
  wording.

### Pull 2, as decided (owner): what the by-hand section promises

- **Drilled** (check 8): the same kernel on both sides, Pop's recovery partition, the
  safety snapshot.
- **Not drilled**: a by-hand go-back after a restore across a kernel change (F1), and
  finishing such a restore by hand (F2). Both are read from the code only.
- **0.5.0: README only.** The section says what was drilled and what was not, names the
  kernel case as a known limit without promising an outcome, and says that when the system
  still starts from either boot entry the way back is the window (shown by checks 2 and 9).
- **0.5.x**: a filter without the kernel lines for by-hand use, with its own by-hand
  rollback drill.

### Fix 1 and 1b, as decided (owner): the package scripts

- **The `prerm` refuses with one line while a job runs.** The helper holds a `flock` on a
  file under `/run` for a job's duration, and the script tests the lock. No marker file
  whose existence is the signal: a killed helper would leave it stale, while the kernel
  drops a lock with its process.
- **The script holds the lock through the stop**, so no job can begin between the test and
  the stop. **The helper also takes it around the arm** (`check_and_arm` isn't a job), so
  the script can't land inside one.
- **A ready plan holds no lock.** An upgrade at the prompt goes on, the plan dies with the
  helper, and the window gets "The preparation is gone."
- **The refusal also holds for the new package's `prerm failed-upgrade`**: without that,
  dpkg goes on with the upgrade after the installed script refused. That sequence is from
  Debian Policy, not run here; the gate shows what dpkg really does.
- **1b: the `prerm` disarms on `remove`, `upgrade`, `deconfigure` and `failed-upgrade`.**
  Order: refuse first on a running job, then disarm, then stop the helper, so a refused
  operation changes nothing.
- **The `/system-update` link is removed only after its target is checked to be Apsis's
  own.** If it then can't be removed, the script fails and prints the exact manual command.
- **`disarm()` syncs after the link's removal, on both paths** (the timer's and the
  script's). Found while reading: it has no sync today, so a power cut seconds after a
  disarm could bring the link back with the filesystem's journal.
- **`postrm remove` also removes Apsis's link and the unit files** (the lines `purge` has
  today); the state folder and the config stay purge-only.

### Fix 2, as decided (owner): the delete renames first

- **A delete first renames the folder from `snapshots/` into `apsis-staging/`**, after
  today's checks (the name, the `O_NOFOLLOW` walk, `info.json` a regular file, nothing
  mounted inside), then removes the tag links, then removes the folder there. A cut delete
  is then a leftover, which the list, Delete and the next create already handle.
- **A folder already half-deleted in `snapshots/`** (a readable `info.json`, no
  `exclude.list`) is listed as such a row, and Delete removes it.
- **Nothing is removed from `snapshots/` unasked**: the next create removes leftovers in
  the staging folder only.
- **A folder in `snapshots/` with no `info.json` stays a known limit**: the delete refuses
  it on purpose, and it still needs a root `rm` by hand.

### Fix 6 and RECOVER.txt, as decided (owner)

- **`RestoreResult` carries `home` and `safety_snapshot` on the wire.** `Helper3` is
  unshipped, so the signature changes without a new interface.
- **`RECOVER.txt` carries two complete, labelled commands**, "restore the same snapshot
  again" and "go back to the safety snapshot", each with its own flags (F4). Nobody has to
  edit a line.
- **With no safety snapshot, the note says so in one line.**
- **One caveat line about the kernel limit** (Pull 2), so the note promises no more than
  the README.
- **The note is written atomically** (a temporary name, then a rename). Today it is a
  plain write onto the backup disk, which a cut would leave truncated.

### Item 9 and N3

- **Item 9**: on a plain exit 23 with the snapshot still there, the apply writes the last
  20 lines of rsync's standard error to the journal, one line each, through `Runner::say`.
  In the real runner `say` writes the journal only (since check 1); the trait's doc comment
  still says "the journal, the console and the boot screen" and is corrected with the fix,
  as is `Runner::refresh_boot`'s, which still names `update-initramfs`.
- **N3**: `remove_arm_files` removes the wants folder when it is empty, as it does the
  drop-in's folder; `postrm` does the same.

### Corrections: the kernelstub count (K1 and K2)

- **K1, PLAN 6b.6 step 5**: "twice per kernel, five ESP writes" was read from the two
  hooks and never counted. "Twice per kernel" was wrong: in check 8's drill, inside a
  chroot, the hooks ran kernelstub **once per kernel**. kernelstub ran three times in all
  (two from the hooks of `update-initramfs -u -k all`, one from the README's own line), and
  the third changed nothing. How many of those runs rewrote files on the ESP was **not
  counted**. Outside a chroot the count is still **unverified**. PLAN's sentence is
  corrected.
- **K2, this file, the "checks 0.1 to 0.3" entry** ("what the boot refresh runs"): the
  same claim, with the same correction. That entry stands as written; this is its
  correction.
- The decision those sentences supported is unchanged: the apply runs kernelstub only and
  no `update-initramfs`, for the other reasons given there.

### What 0.5.0 knowingly does not show or fix (owner)

- A by-hand go-back after a restore across a kernel change (Pull 2).
- The upgrade from 0.4.2: it runs 0.4.2's `prerm`, which has no guard.
- The seconds between Restart now and the reboot: a package operation on Apsis then
  cancels the arm, and only apt's output says so.
- `just uninstall` bypasses the maintainer scripts: one README sentence, nothing more.
- The FAT "not properly unmounted" lines: `fsck.fat -n` was not run, and nothing about
  their cause goes into the README.
- The window's texts for `problems` and `boot-broken` have never been seen in the real
  window. There is no step with a hand-placed `result.json`; the applet's and the layout's
  tests are what shows them.
- Item 9's plain-23 path is shown by a unit test and the real-rsync test only: as root on
  apsis-test a plain 23 needs a real I/O error.
- A folder in `snapshots/` with no `info.json` (fix 2).
- C1 beyond the gate's probe.

### What follows, and the last three points decided

The order of work, where the version bump goes and the release gate's list are in PLAN
6b.13 ("Update 2026-10-04").

**Decided last** (owner, 2026-10-04):

1. C3: Settings' Cancel is always clickable and leaves the page. Before 0.5.0, in the
   small README and applet commit (step 12 of the order of work). C4 stays "no fix".
2. The three result texts (fix 5, `boot-broken`, the `boot-kept` tooltip) are worded in
   their own commit, for the owner to edit.
3. The unit's `ConditionFileIsExecutable=` hardening from `notes/armed-upgrade.md` is
   after 0.5.0: it changes the unit's text and wants an offline boot of its own.

## 2026-10-04 - as built: the fixes before 0.5.0, the version bump, and the release mechanism

The fixes and records decided in the triage entry above ("triage of checks 1 to 9 and the
check 8 findings, and the owner's decisions") are built, in PLAN 6b.13's order of work, one
commit each on `restore-6b-core`: `176c898` (fix 3), `9142449` (fix 4), `ab75045` (fix 6),
`ca59fcf` (fix 5 and the result texts), `9f83218` (item 9), `917e980` (Pull 1), `0f5a6c7`
(the disarm's flush, N3), `c251e49` (the armed-state gap), `114e2aa` (row 12), `f8c2db2`
(`RECOVER.txt`), `546abe7` (the README's recovery section), `32d1dff` (fix 2), `9d23dd8`
(a README limit), `9692f8c` (fix 1, 1b, row 5), `f918845` (step 12), `0ee8dda` (the
version bump). This entry records, by topic, what the code decided beyond the rulings, and
which earlier rulings it replaces. PLAN 6b marks each built bullet "Built (...)"; the older
entries stand as written.

**Labels.** "Read, not run" or "read from the code" = no run has shown it; "unverified" = not
known. Every decision marked (owner) is the owner's, 2026-10-04.

### The helper and the job lock

- **The lock file** is `/run/apsis/job.lock`, root's, 0600, opened `O_NOFOLLOW | O_CLOEXEC`
  and taken with `LOCK_EX | LOCK_NB` after the `running` flag and before anything is
  announced, so a write refused by a package script was never a job. A symlink there, or a
  file not owned by root, refuses the job as `Busy` with a journal line and leaves the file
  as it is; any mode but 0600 is set to 0600 with `fchmod` first (`flock` works on a
  read-only descriptor, so a 0644 file would let any user hold the lock). Left: a user who
  opened a 0644 file before that `fchmod` keeps the descriptor until a restart; only an
  unreleased build without the `prerm`'s `umask 077` could have made such a file.
- **Unlocked, not only closed.** The lock's `Drop` calls `LOCK_UN`: a child forked a moment
  earlier holds a copy of every descriptor until it execs, and a `flock` lasts until the
  last copy closes. `O_CLOEXEC` stays, for a helper that dies.
- **One guard, not two.** The triage's "the helper also takes it around the arm" is built
  wider: `State::take_ready` marks the plan as taken in its slot and takes the lock, and that
  guard (`Ready`) is what "Restart now", a cancel and the starter-gone path hold until the
  plan has ended. The guards own the lock, not `State`; a ready plan's lock is dropped
  before the plan goes back in its slot. A ready plan still holds no lock, as ruled.
- **Row 5 is closed**: in the seconds between "Restart now" taking the plan and the link
  being made, a second `Restore`, `Delete`, `DeleteMany`, `Create` and `WriteConfig` are
  `Busy` (owner: kept as `Busy`), lists and `CheckRestore` go on, and `Job()` shows the
  restore at 100%. A second "Restart now" and a cancel then are `Busy` too. The window is
  unchanged: `Busy` closes the prompt with the busy line, and the plan ends right after.
- **A taken plan dropped unended** (a panic) leaves its slot and frees the lock with no
  announcement, as a job dropped unended does.
- **Reads, `--disarm` and `--apply-restore` take no lock.**
- **New journal lines**: `the job lock <file> is held: a package script runs`, `the job
  lock <file> couldn't be taken: <error>`, `the plan's window left the bus, but the plan
  stays: <error>`.
- **The helper's errors cross the bus in their own words** (`f918845`): `encode_error`
  sends `Error::Helper`'s message without the `apsis-helper: ` its Display puts in front;
  the journal keeps the prefix (`describe_error` uses Display). `41bcac9` (0.4.2) had taken
  it off only for errors made in the window's own process; a helper error that crossed the
  bus showed it in every 0.4.x (checked against the tags' code) and on this branch until
  `f918845`.
- **This replaces, in part**, check 9's entry's "its raw `apsis-helper:` prefix is already
  gone in 0.4.2" (this file, lines 5372-5373): that holds for errors made in the window, not
  for helper errors that crossed the bus, which kept the prefix until `f918845`. Where C1's
  mount error got its prefix in 0.4.1 is unverified (in this build a failed `mount` is
  `Error::Native`, `mount failed: <stderr>`, with no prefix).
- **The journal says `refused:` for a refused restore** (fix 3, `176c898`):
  `Error::RestoreRefused` joins `describe_error`'s refused list, so the line is `refused:
  can't restore this snapshot: <word>` at the preparation's end and at "Restart now", where
  the same text also goes into the plan's removal line.

### The restore: the preparation, the arm and what they write

- **The armed-state guard** (`c251e49`). A second preparation while `/system-update`
  exists is refused before it touches anything: `prepare::guarded` looks at both names in
  front of the cleanup of leftovers and of the removal after a failed preparation.
  Apsis's own link is `Refusal::RestoreArmed` (`restore-armed`); anything else at either
  name stays `PendingUpdate`. `CheckRestore` makes the same split (core's `System` gained
  `restore_armed`, set from a read of the link; at the apply it is false, the link being
  the restore's own), keeping the pending update's place, last, in 6b.7's order.
- **B1 (owner)**: `Restore` refuses while armed before the password and reads the link again
  after it, both before `State::begin`; `guarded` under the lock is the third look. This
  reverses an earlier ruling of the same day (the refusal after the password) that was
  made in a review round and not written in this file. The order in `Restore`: the name
  check, the link, busy, the password and the caller's uid, the link again,
  `State::begin`. The method's error is `Failed` with `restore refused: restore-armed`, so
  the window opens its "Can't restore this snapshot" dialog; the journal line is
  `refused: can't restore this snapshot: restore-armed`. A foreign link is not refused
  before the lock: the password is asked and `guarded` refuses it as `pending-update`, a
  job that ends `failed`, as before.
- **Pull 1, the filter's lifetime** (`917e980`), as built:
  - `give_up` removes the working files but not `request.json`: it never removed it (the
    helper's next start removes the plan). This corrects the premise of the triage's
    "next to the removal of `request.json` in core's `clean_up` and `give_up`" (this file,
    "Pull 1, as decided", line 5959); the removal sites are otherwise as ruled.
  - The arm's two renames are consecutive, not one step: a power cut between them leaves a
    mixed pair (this plan's filter, the earlier arm's note), with no link. Temporary files
    of a cut arm (`last-restore.*.apsis-tmp`) stay until the next arm.
  - A late arm failure (after the pair is kept, for example the helper copy) has already
    replaced the pair and cleared `rsync-log`.
  - A refused "Restart now" still calls `disarm` first; the plan's end is one line,
    `plan removed (refused: ...)`, with `arm undone: ...` only if something of an arm was
    there. No unit test covers that routing; the release gate's refused "Restart now"
    does.
  - A preparation can't run while another plan is ready (`State::begin` answers `Busy`),
    so the removal at a failed preparation's end touches its own files only.
  - `copy_all` stages every copy before it renames any; a source that can't be read leaves
    both targets as they were. The copies are not capped at 64 KiB as the JSON files are.
  - The cleanup of leftovers leaves `result.json`, `rsync-log` and the kept pair, so Cancel
    no longer deletes the last restore's log.
- **The disarm's flush** (`0f5a6c7`): `sync()` returns no error, so the failure policy
  covers the flush of the link's folder only; an error takes the place of the list of
  removed names; a removal that fails after a flush that failed wins (the flush's error is
  lost). A restart that logind refused now logs `couldn't disarm: ...` instead of dropping
  `disarm`'s result. An empty wants folder is removed whoever made it, as the drop-in's
  folder is. The flush failure is injected in tests; no test shows a real power cut, a real
  `fsync` failure, or that `sync()` ran.
- **`RECOVER.txt`** (`f8c2db2`): `restore.note` is written first (the state folder), then
  `RECOVER.txt` on the backup disk; if the second fails, the preparation fails and removes
  `restore.note` again. Each write is a temporary file, `fsync`, rename, `fsync` of the
  folder; a failed write leaves the earlier note whole. Modes: `restore.note` 0600,
  `RECOVER.txt` 0644 exactly (it holds only UUIDs and snapshot names). A note that can't
  be written fails the preparation after the safety snapshot was made: the safety snapshot
  stays as a list row; the error names neither (owner: a README sentence, no code). The
  filter warning line sits after the commands, before `Then:`. rsync in Pop's recovery is
  3.2.7 (check 8); a missing `--exclude-from` file gives exit 11 before anything is copied
  (shown on the main machine with 3.2.7 and the note's flags); a live USB's rsync is
  unchecked. A test asserts that the "same snapshot again" command drops `-A -X` for an
  old-format snapshot and the safety snapshot's keeps them; there is no golden file for it.
- **Which pair a recovery finds** (read from the code): every state where the note may be
  needed comes after an arm, and the arm copies the pair first or fails with no link; after
  that only the next arm or a purge replaces or removes `last-restore.*`, and
  `/var/lib/apsis/***` is protected from the copy. A Stop never rewrites `RECOVER.txt`.
- **Item 9** (`9f83218`): on a plain exit 23 with the snapshot still there, rsync's last
  error lines go to the journal as `apsis-helper: rsync stderr: <line>` (owner: the prefix
  `rsync stderr: `), empty lines left out, said after the copy's `syncfs` and before the
  step is saved. Core does not cut to 20; the helper's tail does, and one test ties the
  two. The journal can carry file names, including files in another user's home if home
  was restored (accepted, owner: the journal is readable by `adm`, the log is root's).
  Nothing is said line by line after exit 0 or 24, after a copy that broke, or after a 23
  with the deletions skipped or the snapshot gone.
- **Fix 6** (`ab75045`): `RestoreResult` is `(sssxss)`. A ready plan sends `""` for home
  and the safety snapshot. `from_wire` refuses an outcome with no home word or one it
  doesn't know; the wire's home words are the file's, `keep` and `restore`. `result.json`'s
  format did not change (it had both fields from its first commit). A window and a helper
  that disagree on the type get an error for the call and the window shows no result line
  (read from the code, not run).

### Delete and the staging folder

- **Fix 2** (`32d1dff`), beyond the ruling: the move is `RENAME_NOREPLACE` (any error
  refuses, nothing moves, no plain-rename fallback); the staging folder is made with
  `mkdirat` 0755 (the umask applies) and opened `O_NOFOLLOW`, and a refused move removes it
  again if it is empty. Two refusal texts, both `InvalidInput` and before anything changed:
  `apsis-staging/<name> is already there` (`EEXIST`) and `can't move it into
  apsis-staging/ (<errno>)`. The order after the move: the line `moved <name> to
  apsis-staging/`, `fsync` of `snapshots/` then of `apsis-staging/` (an error fails the
  delete with the folder whole in the staging folder), the tag links, the walk, the
  staging folder if empty, the unchanged `deleted ...` line.
- **The row rule**: a snapshot name, a real folder, a regular `info.json` that parses, no
  `exclude.list`. A symlinked `info.json` with no `exclude.list` stays the warning
  `incomplete: no exclude.list`. Every leftover row reads "Unfinished snapshot or delete ·
  Delete removes it" (owner).
- **Left as is (owner)**: a name half-deleted in both `snapshots/` and `apsis-staging/` is
  one row whose Delete is refused (`EEXIST`) until the next create removes the staging one;
  a dangling tag link after a cut stays until the next create.
- **Row 12** (`114e2aa`): every `Delete` and `DeleteMany` is refused while Apsis's own link
  is in place (owner), read before the password and again after it, both before
  `State::begin`: no job, no announcement, no mount. The name check still comes first. Not
  checked a third time under the lock (a refusal there would be a `failed` job that every
  window shows); since `9692f8c` a delete in row 5's seconds is `Busy`. On the wire:
  `Error::RestoreArmed`, the text `restore armed`, the existing `Failed` error. Another
  tool's link refuses no delete. An older window with the new helper shows "Delete failed:
  restore armed" (read from the code, not run).
- **No path but `Delete` and `DeleteMany` removes a finished snapshot** (read from the
  code); a create removes folders in the staging folder only.

### The window

- **Fix 4** (`9142449`): when "Restart now" is refused (`RestoreRefused`), the window opens
  the "Can't restore this snapshot" dialog with its dropped-plan line, as before, and now
  also sets the status line to "Restore stopped"; before, it returned without setting it,
  so `Preparing restore · ready` stayed. Any other error at "Restart now" keeps its error
  line, unchanged.
- **The result texts** (`ca59fcf`; the strings are the owner's): `problems` has two lines,
  told apart by the helper's message naming `rsync-log` ("some files were not restored")
  or not ("a cleanup step failed"); brittle and deliberate, a typed field would change the
  wire. `problems` is the warning colour; `boot-broken` has its own line and tooltip in the
  error colour; the `boot-kept` tooltip says "keeps the ones it had" (it also covers boot
  files never touched); the `failed` tooltip has its own string when no safety snapshot was
  taken. Colours are tested as tones, not pixels. Known limits (owner: not in 0.5.0): the
  `failed` tooltip's advice is always about the backup disk; a `done` result can carry a
  note about the previous kernel's boot files that no window shows; `rsync-log` needs root
  to read. None of these lines has been seen in the real window.
- **The result's tooltip** (fix 6) names the safety snapshot by its folder name (owner);
  a result with no home word, not reachable over the bus, leaves the home sentence out.
- **The armed refusals**: the "Can't restore this snapshot" dialog with its two lines for
  B1, and no wait for a job end for it (`expect_own_end`; before, any restore job's
  announcements were taken for its own for five seconds). A delete while armed: "Not
  deleted: a restart to restore is waiting." with the tooltip "Restart the computer, or
  wait for it to time out. Nothing was deleted."; no list, the selection stays.
- **Settings' Cancel** (C3, `f918845`) is always clickable and leaves the page, dropping
  any change; before, it was disabled until a value changed. A test clicks the centre of
  every laid-out part of the page with the headless renderer and checks which messages
  come out, the way a button's enabled state is now tested; it runs with the layout tests,
  `APSIS_LAYOUT_TEST=1 cargo test -p apsis -- fit settings_cancel`.

### The package scripts

- **The `prerm`** (`9692f8c`): A's line (`apsis: the restore that was waiting for a restart
  is cancelled`) on stdout, the refusals and the warning on stderr. It sets `umask 077`
  before it makes the lock file, after the `mkdir`, so `/run/apsis` keeps its mode. The
  timer stop isn't guarded by the systemd check (`|| true` covers it). After `--disarm`
  the link decides, not the exit status, as PLAN 6b.9 records.
- **`--disarm`'s lines**: `disarm: nothing armed`, `disarm: removed <names>` (owner: the
  shorter line; "no restart in 10 minutes;" is gone, false under `apt remove`).
- **`postrm`**: the reload of systemd and the bus comes after the unit files go (for purge
  it came before); the wants folder and the drop-in's folder go when empty; purge's
  removals are `rm -rf "$ETC" "$LIB"` (`/etc/apsis`, `/var/lib/apsis`), last.
- **`postinst`** was touched for one line: all three scripts name the systemd check's path
  as `SYSTEMD=/run/systemd/system`, so the tests don't depend on the build machine running
  systemd.
- **The test harness** runs copies of the scripts with a `PATH` of one folder: a fake
  `systemctl`, `busctl` and helper, and links to `flock`, `mkdir`, `readlink`, `rm`,
  `rmdir`, `true`. It refuses root, any absolute path outside its folder but `/dev/null`
  and `/org/freedesktop/DBus`, and `..`. A new command in a script needs a line there or a
  fake.
- **Read, not run**: every statement about what dpkg does around the refusal (the new
  package's `prerm failed-upgrade`, apt's wording). The release gate's list in PLAN 6b.13
  shows them.
- **The version bump** (`0ee8dda`): the workspace version is 0.5.0 (`Cargo.lock` follows,
  the three apsis crates only), and the three release entries that go into the .deb come
  with it, dated 2026-10-04: `resources/deb/changelog`'s `0.5.0-1` entry (signed with the
  repo's commit identity), the metainfo's 0.5.0 release, the man page's `.TH` line. The
  changelog entry and the metainfo say that an upgrade from 0.4.x does not wait. This
  **replaces** "the 0.5.0 entry is the owner's at release, with `Cargo.toml`'s version
  bump" and "also the owner's at release: the metainfo's release entry" (this file, lines
  3998 to 4001) and "the `.TH` line ... the owner's at release, with the version" (line
  3994): they are in the bump commit (owner). No test checks that the versions agree.
- **lintian** on the 0.5.0 .deb built locally: 0 errors. `desktop-mime-but-no-exec-code`
  is kept (owner): it comes from the applet entry's empty `MimeType=` since 0.4.2 dropped
  `%F` (`Exec=apsis`), and 0.5.0 doesn't change it. It joins the warnings kept since
  2026-09-26 (this file, line 785 on); `maintainer-script-calls-systemctl` now has three
  sites (`prerm` twice, `postrm` once).
- **The metainfo's screenshots stay as they are for 0.5.0** (owner): they point at an
  older commit's copies.

### The docs

- **README** (`546abe7`, `9d23dd8`, `f918845`, and the fix commits): "If a restore goes
  wrong" matches `RECOVER.txt`, with the boot check and `last-restore.filter`; the
  half-deleted snapshot that looks whole is a known limit; the troubleshooting rows for
  the new result lines, for a backup disk the desktop mounted (unmount it in Files, don't
  eject, then Refresh; what makes the desktop mount it is unknown), and for a filter
  without a leading slash (one sentence under Filters); one sentence that `just uninstall`
  runs none of the package's checks; a paragraph "The upgrade from 0.4.x doesn't wait"
  (v0.4.0, v0.4.1 and v0.4.2 ship one and the same `prerm`).
- **The man page** carries the same filter sentence and names `last-restore.filter`;
  `groff -man -ww -z` is clean.
- **CHANGELOG.md**'s 0.5.0 section stays "Unreleased" until the release commit.
- **No per-fix entries here**: this entry is the one record of what the fixes decided
  (owner).

### Knowingly left (owner), as PLAN 6b records

- Timeshift making a snapshot in place could show as a deletable row while it runs (not
  known whether it ever has `info.json` without `exclude.list`); Delete needs a click, a
  confirm and a password.
- A job begun after the `prerm` has exited, during an upgrade's unpack, runs the old binary
  to its end.
- The upgrade from 0.4.x runs 0.4.x's `prerm`, which never refuses (it stops the helper,
  even in the middle of a snapshot or a delete).
- A snapshot half-deleted in place by Apsis 0.4.x or Timeshift that still has `info.json`
  and `exclude.list` is listed as a whole snapshot, can be the `--link-dest` base and is
  offered for Restore; the safety snapshot is the way back. Not detectable reliably.
- Settings' Cancel during a save that is still running drops the edits and leaves the page
  while the save still writes; its answer still sets the status line and reloads the
  settings. Read, not seen.
- `removed_or` puts the error's full text into `DeviceRemoved`'s reason, so it can carry
  the `apsis-helper:` prefix if an `Error::Helper` gets there. Unverified whether any does
  (rsync's and `mount`'s errors are `Error::Native`); the code is left as it is.
- A bare `Error::Helper` text that equals or starts like an encoded kind (`stopped`,
  `refused: `, ...) would decode as that kind. No guard: `Error::Native`, `InvalidConfig`
  and `InvalidSettings` have had the same exposure all along, and none of today's
  `Error::Helper` texts does so.

The triage entry's list "What 0.5.0 knowingly does not show or fix" stands.

**After 0.5.0 (owner)**: a window opened while a restore is armed has no line that says
so; a refusal before the lock that the window doesn't know as "no job" (an invalid name,
for example) still gets the five-second wait; in the armed window, a click on Restore
clears "Restarting..." and leaves the line empty after the dialog closes.

### Release mechanism (owner)

**The release gate tests the .deb that CI builds from the tag.** The release commit
(`CHANGELOG.md` dated, README's Status line, SECURITY's Supported row) comes first, and the
tag is pushed on it, so the gate runs on the released commit. The tag's push makes the .deb
and a draft release that only the owner can see; the owner runs the gate on apsis-test
with that file and publishes the draft only if the gate passes. On a failure, the tag and
the draft are deleted, and the fix is tagged again.

Why, from a read-only check of the workflow and the local build (2026-10-04):

- **A local .deb never hashes the same as CI's.** CI's Rust is unpinned stable
  (`.github/workflows/release.yml:31`, `dtolnay/rust-toolchain@stable`; no toolchain file
  in the repo) and cargo-deb is unpinned (`release.yml:33-34`, `cargo install cargo-deb
  --locked`). Known the same: the sources and the lock (`release.yml:36`, `just deb
  --locked`, passed to `cargo build` by the `deb` recipe).
- **The binaries embed the build machine's cargo home path.** No `[profile]`, no
  `trim-paths`, no remap flag and no `RUSTFLAGS` exist in the repo. In the local build the
  cargo home's path appears 1186 times in `/usr/bin/apsis` and 233 times in
  `/usr/libexec/apsis-helper` (the repo's own path 0 times: the workspace's sources appear
  as relative paths). A .deb built on the owner's machine would carry the builder's
  account name in those paths and must not be uploaded over CI's.
- **cargo-deb's dates**: `SOURCE_DATE_EPOCH` is set nowhere; the local .deb's entries are
  all dated 2026-10-01 10:00, neither the build time nor the commit's. Where cargo-deb takes
  that date is unverified.
- **The .deb's contents are final at the bump.** The three release entries are in
  `0ee8dda`; the release commit only dates `CHANGELOG.md` and edits README's Status line
  and SECURITY's Supported row, none of which goes into the .deb. The workflow names the
  file from `Cargo.toml`'s version, not from the tag (`release.yml:43` uploads
  `target/debian/*.deb`; the tag only names the release).
- The workflow runs on pushed `v*` tags only (`release.yml:7-9`); it makes a draft only if
  the tag has no release yet (`release.yml:41-42`) and uploads with `--clobber`.

This makes "the real 0.5.0 .deb" precise, in the version bump's ruling (this file, check
9's entry, "The version bump: not before this check", line 5236 on) and in PLAN 6b.13's
release gate: it is CI's file from the tag, not a locally built one.

## 2026-10-05 - the release gate passed on CI's 0.5.0 .deb (apsis-test)

PLAN 6b.13's release gate, on the .deb the Release workflow built from the tag `v0.5.0`.
Runbook `notes/release-gate-runbook.md` (untracked), steps G0 to Z. The tag was made at
2026-10-04 19:40 (+1100, git); the gate ran after it, in two sittings by the handoff's
times: G0 to D2, then C1 to Z. Times below are the laptop's (UTC+11; the runbook said
UTC+10).

**Where this entry's facts come from.** The owner ran every command and wrote the result up
with a second guide (a claude.ai conversation), as for check 8. Claude Code wrote the
runbook and its script and saw none of the logs; it checked the findings against the code
(each says so). S1's details and R2's first try come from the guide's own record of the
owner's logs, given after the handoff. Where something wasn't looked at or wasn't reported,
it says so.

**Verdict: passed.** No step failed; two first tries missed. L1: an empty `$pw` hung the
command. R2: the script's own guard printed `no plan is ready: stop`, nothing changed, and
the log was kept as `gate-r2-first-try.txt`. The pass-word file holds
all 25 words in the runbook's order: `G0-FILE`, `P1-START`, `P2-ENV`, `P3-DEB`,
`I1-INSTALLED`, `I2-LOOKED`, `T0-FOUR`, `L1-LOCK-75`, `L2-REFUSED`, `L3-REFUSED`,
`L4-KILLED`, `D1-ROW`, `A1-ARMED-BRANCH`, `A2-ARMED`, `A3-OTHER`, `D2-CHECK13`,
`C1-PROBED`, `S1-CANCEL`, `R0-BASE`, `R1-REFUSED`, `R2-GONE`, `R3-READY`, `R4-VERIFIED`,
`R4-SHOWN`, `Z-CLOSED`. None of the findings below blocks the release (the guide's view;
the owner decides).

**Release state at the handoff.** `main` was fast-forwarded to `a79ac39`, then the release
commit `6e5a5a5` ("release: 0.5.0": CHANGELOG, README's Status, SECURITY), pushed. The tag
`v0.5.0` (annotated, like `v0.4.2`) is on `6e5a5a5`; the Release workflow was green; the
draft release `Apsis 0.5.0` holds `apsis_0.5.0-1_amd64.deb`. **Not published**: publishing
is the owner's.

**Ruling (owner, 2026-10-04), recorded here because no entry had it:** `restore-6b-core` is
merged into `main` first (a fast-forward from `10ac37c`), then the release commit is made
on `main`.

### The file under test

- the .deb `258aac7807fc43972ecf0d9b8269663d328f629402f4d849f90e7c5f7ba184c8`;
- `/usr/bin/apsis` `6699c133c1f6246443e8a0b9113274a0fb0673425f5b908a5a91d544f5cf03ec`;
- `/usr/libexec/apsis-helper` `e49007b9262f382938586f302dd0f83de1ebd52bf84b4aa7ebe7954c7c5d15ac`.

The binaries' short hashes after I1, after R4 and after Z are those values. They differ
from the local build of `0ee8dda` (apsis `559e8037…`, helper `18063d60…`), as the release
mechanism expects (unpinned stable Rust, the cargo home's path).

### What each step showed (condensed)

- **L1**: `/run/apsis/job.lock` is `600 root root`; `flock -n -E 75` printed 75 during a
  create. (A first try hung: `$pw`, the pass-word file's variable, was empty.)
- **L2**: `apt install --reinstall` during a create was refused: `apsis: an Apsis job is
  running; try again when it has finished`, dpkg's `pre-removal script subprocess returned
  error exit status 75`, apt exit 100; still `0.5.0-1`, `dpkg --audit` clean; the create
  ended `done`. **L3**: `apt remove` during a delete, refused the same way (`Processing was
  halted because there were too many errors.`), still installed. The handoff quotes these
  lines, not the whole of dpkg's output (open item below).
- **L4**: the helper killed with `kill -9` during a create, then a reinstall: no `apsis:`
  line, apt exit 0, no rsync left, no stale lock. The window: the full error view "Remote
  peer disconnected" with "Try again", the red line "Create failed: the helper stopped
  before it finished". The leftover row deleted: `Delete this snapshot?` / `unfinished
  snapshot or delete from 2026-10-04 20:57` / `This can't be undone.`, then `Deleted
  unfinished snapshot or delete from 2026-10-04 20:57` (5 s).
- **D1** (fix 2): the helper stopped by hand during a delete: "Delete failed: the helper
  stopped before it finished" and a dimmed row; Delete removed the row (68 s).
- **A1**: a hand-made Apsis link, then a reinstall: `apsis-helper: disarm: removed
  /system-update`, `apsis: the restore that was waiting for a restart is cancelled`, apt
  exit 0, the link gone, no disarm timer. Another tool's link: no `apsis:` line, link kept.
- **A2** (armed by Apsis): the dialog "Can't restore this snapshot" / "A restart to restore
  is already waiting." / "Restart the computer, or wait for it to time out."; by hand
  `restore refused: restore-armed`; a delete of one row and of two (`Delete 2 snapshots?`):
  "Not deleted: a restart to restore is waiting." **A3** (another tool's link): "A system
  update is waiting for a restart." / "Restart first, then restore."; refused
  `pending-update`; a delete went through (85 s). A2 shows the armed-gap fixes in the real
  window (`c251e49`, a second preparation refused while armed; `114e2aa`, Delete and
  DeleteMany refused while armed); A3 shows that another tool's link refuses no delete.
- **D2** (check 13): `Deleting 1 of 3: …` to `3 of 3: …`, then `Deleted 3 snapshots`; one
  `delete-many … started` in the journal, no `refused`, no `busy`; both monitors on.
- **C1**: with the backup disk mounted by the desktop, the create "gate c1" worked, then
  every list failed (`mount: <backup>: /dev/sda already mounted on <masked>`): the full
  error view with "Try again", no Refresh icon, the status line still "Snapshot created",
  Restore did nothing. No `apsis-helper:` in any window text. After Unmount (not Eject) the
  list came back.
- **S0**: Settings, then Cancel with nothing changed: Cancel not greyed out.
- **S1** (from the guide's own record; Claude Code saw no logs): the filter
  `/var/tmp/apsis-gate-c3/***` added as an unticked row (shown as "-"), Save, then Cancel
  at once. `Saving the settings…` was not seen (the save was too fast); the status line
  read "Settings saved" and the window was on the list. After: the config's short hash
  `b00cf898df037b02`, `filter-count 1`, journal `write config for :1.191: written` (no
  `started` or `done` line). Settings opened again: the new filter listed, and the footer
  already read "Settings saved" (finding 5, seen in the window). Removed and saved: the
  short hash `1e8d387f39671970` again (equal to the first), `filter-count 0`. See "S1:
  Cancel right after Save" below.
- **R0**: the dialog "Restore the system?", "Keep my files as they are now", the safety
  snapshot ticked, the experimental line, no Apsis line; journal `check-restore … ok; home
  yes, root no, current format, apsis current`.
- **R1** (fixes 3 and 4): `/pop-upgrade` present at the prompt, "Restart now": "Can't
  restore this snapshot" / "A Pop!_OS upgrade is in progress." / "Finish or cancel it
  first, then restore." / "The preparation was dropped. Restore again to measure afresh.",
  status "Restore stopped"; the working files gone, the kept pair unchanged.
- **R2**: a reinstall at "Ready to restore": no `apsis:` line, apt exit 0, the helper
  stopped, the plan's three files still there; "Restart now" did not restart and said "The
  preparation is gone. Start the restore again."; journal `leftovers removed at start:
  request.json, restore.filter, restore.note`. (A first try stopped at the script's own
  guard, `no plan is ready: stop`; nothing changed; the log was kept as
  `gate-r2-first-try.txt`.)
- **R3**: the happy path, "gate base" (`2026-10-05_16-29-14`), home kept, safety snapshot
  `2026-10-05_16-55-45`. At the ready prompt `restore.filter` hashed as at R1 and
  `restore.note` equalled `RECOVER.txt` on the backup disk. The offline boot showed
  "Restoring the system. Don't turn off the computer." with the Pop!_ logo (no progress bar
  in the photographs). `bootctl list` was the same four entries before and after.
- **R4**, before login: no link, `restore-check.sh` all `ok`, 0 failed units, `dpkg --audit`
  clean, the running kernel the ESP's, the oldkern entry untouched, `pop-upgrade-init`
  skipped by its condition, the system marker gone and the home marker kept; the kept pair
  equal by hash to the ready prompt's (`FILTER-KEPT-BY-HASH`, `NOTE-KEPT-BY-HASH`; the
  values weren't printed). `RestoreResult` by busctl: `sssxss "done"
  "2026-10-05_16-29-14" "" 1791180142 "keep" "2026-10-05_16-55-45"` (the number is
  `result.json`'s `when`). After login: `System restored to 2026-10-05 16:29`, tooltip
  `Restored from snapshot 2026-10-05_16-29-14. Home folders were kept. Safety snapshot:
  2026-10-05_16-55-45.`, 7 rows; the panel applet on both monitors: "Last snapshot 12m
  ago" / "15m ago", "sda 18G / 28G · 67% used · 8.8G free", five rows and "2 older".
- **Z**: the gate's four snapshots deleted one at a time (each dialog named its row, no
  password), then `ok, 3 snapshots, 12031221760 of 30120226816 bytes free (statvfs)`.

### Numbers

- Free bytes on the backup disk at P1 and at Z: 12031221760 of 30120226816, no
  difference. 1745 packages and kernel `7.1.5-76070105-generic` at both.
- Creates: the first about 7m40s (about 1.8 GB), each later one 2m03s to 2m28s (about 0.2
  to 0.3 GB each). Deletes: 1m12s for one; D2's three in one job 3m31s.
- R3, from the helper's journal: the restore started 16:54:59 and was `ready` at 16:57:59
  (3 min): the plan's dry run 31 s, the safety snapshot's dry run 15 s, its copy 1m37s,
  then 37 s with nothing logged. Which status line covers which phase is inferred. The
  safety snapshot took about 253 MB.
- "Restart now" at 17:00, `armed; restarting` at 17:00:46; `apsis-restore.service` 17:01:29
  to 17:02:23 (54 s, 27.117 s CPU; the copy attempt 1 of 3, rsync exit 0; the boot files
  refreshed; `apply-restore ended: Finished(Done)`); the last boot began 17:02:37. Click to
  greeter: about 2 minutes.

### Deviations from the runbook

- R3's "pick the default entry" would have picked the recovery: `bootctl` marks it
  `(default)`. The owner picked `Pop_OS-current.conf`, the `(selected)` one.
- R4's expected rows and Z's deletes left out "gate t5b"; the owner deleted it too, so the
  count came back to 3. PLAN's line says "the four rows"; there were 7.
- R3's status lines between the click and `ready` were not captured, except the first and
  `ready` (the journal gives the timings); R1's click-to-prompt time was not recorded.
- Z's `systemctl list-units --all "apsis-*"` printed nothing: the list is empty while the
  helper isn't loaded (`list-unit-files` shows the units).

### S1: Cancel right after Save shown; Cancel during a running save not shown

From the guide's own record of S1 (its write-up of the owner's logs, given after the
handoff); Claude Code saw no logs. **Shown**: Cancel clicked at once after Save drops the
edits, leaves the page (the window was on the list, the status line "Settings saved"), and
the save has written (`filter-count 1`, journal `written`). **Not shown**: Cancel landing
while the save is still writing: the save ended before the click could be seen (`Saving
the settings…` never showed). PLAN's gate line ("Settings' Cancel during a running save")
is shown for its "right after Save" part only.

### Findings, with what the code says. None is fixed or started

1. "Create failed: the helper stopped before it finished" and "Delete failed: …": the
   prefix is `apsis.ftl:73` and `:79`, the reason is English text in core
   (`helper/client.rs:453`), and neither is in the README's table or the man page.
   "Remote peer disconnected" is zbus's error text, shown as it is in the error view.
2. The "Not deleted: a restart to restore is waiting." tooltip needs a few seconds of
   hover. The README's table (line 369) has the line; a sentence about the tooltip is the
   owner's call.
3. "A restore started elsewhere failed" never showed after a refused by-hand restore: as
   designed. When a restore job that another caller started ends, the window refreshes the
   list only if it has no plan ready (`self.ready.is_none()`, `app.rs:2365-2369`);
   otherwise it does nothing. This branch sets no status line itself (`app.rs:2364`,
   "that window says how it went"). A2's refusal starts no job; A3's
   (another tool's link) runs a job that ends `failed`, which is such a job. The ftl key
   `restore-failed-elsewhere` (`apsis.ftl:217`) has no caller: an unused string.
4. With the disk mounted by the desktop, after a create that worked: the full mount-error
   view, the status line still "Snapshot created", no Refresh icon, Restore doing nothing.
   The failing mount itself is check 9's finding 1 and its README row (README line 361);
   the rest is new.
   Not traced in the code.
5. "Settings saved" stays on the status line when Settings is opened again: seen at S1,
   and confirmed in the code. `open_settings` (`app.rs:2389-2395`) switches the page, and
   reloads the settings unless the open settings have unsaved edits (`app.rs:2392`).
   Neither it nor the read's handler (`on_settings_read`, `app.rs:2421-2428`) touches the
   status line. The only `self.status = None` in `app.rs` are in `open_restore` and `run`
   (`app.rs:1528`, `app.rs:2086`), and at S1 the window kept the line.
6. The helper logs `write config for :1.N: written` (or `unchanged`), one line
   (`service.rs:729`). The runbook expected `started` and `done`: **the runbook's error**,
   against the rule that expected journal lines come from the code's strings.
7. `System restored to <date>` from an old `result.json` before any restore of the gate: as
   designed (UI.md, "After login"): `RestoreResult` answers from the last restore's
   `result.json`, check 8's here, until the next restore.
8. The panel applet clips a long comment at its right edge ("Safety snapshot, before
   restorin") on both monitors. Not traced.
9. The systemd-boot menu shows `pop_os-…` in lowercase and no default or selected tags,
   unlike `bootctl`; `bootctl` marks the recovery `(default)`. Not Apsis's; a runbook line.
10. kernelstub logged `NVRAM entry #: -1` in the offline boot. Not checked whether that is
    normal on this machine.
11. `g.sh verify` printed `-- No entries --`: journalctl's own line when a filter matches
    nothing. The runbook should have said so.
12. At 17:00:46, as the restart began: `polkit check of …Apsis.list for :1.109 failed:
    …NameHasNoOwner` and `list for :1.109: refused: not authorised`. The caller was gone
    by the check; probably the restart's race, not checked.
13. Confirmed in the code: only the preparation writes `timeshift/apsis-restore-RECOVER.txt`
    (`apsis-helper/src/prepare.rs:358`); no delete touches it. After Z the file names
    "gate base" and its safety snapshot, both deleted. The README says it is written again
    at every preparation, not that a delete leaves it. The file on the disk was not looked
    at.
14. Runbook corrections for the next one (the runbooks are untracked): `list-unit-files`
    for the units; UTC+11; `stopdel` prints only the lines after the stop; the journal
    names `:1.N`, not the user; `c1-look` can run before the desktop has mounted the disk;
    in S1 the "-" is the unticked state, not typed text; the expected rows and Z's list
    with "gate t5b" and the real counts; R3's boot menu line (9).
15. Not shown, knowingly (PLAN 6b.13): the window's lines for `problems`, `boot-broken`,
    `boot-kept` and `failed`.
16. PLAN's gate line writes `RestoreResult`'s answer as `(sssxss)`; busctl prints `sssxss`
    without brackets.

### Left on apsis-test

Apsis `0.5.0-1` (CI's .deb), the helper started on demand only. The system files are
"gate base"'s, the home folder kept, kernel `7.1.5-76070105-generic`, 1745 packages. No
`/system-update`, no `/pop-upgrade`, no timer, no rsync, nothing armed. The state folder
holds `result.json`, `last-restore.filter`, `last-restore.note` and `rsync-log`. The backup
disk holds the three base snapshots ("baseline1", "6b baseline", "baseline 0.5.0") and
12031221760 bytes free. `/opt`'s three items as at P1. The window's line reads `System
restored to 2026-10-05 16:29`. The owner stays on `0.5.0-1`. The gate's folder on the
laptop and the logs on the main machine are the owner's to clear.

### Open items

- `notes/session-2026-10-01.md` is still tracked at `6e5a5a5` (since `c211d0b`). Taking it
  out is a `git rm` in a commit; the owner's call.
- From the owner's logs, not in the handoff: L2's and L3's tables in full (dpkg's lines,
  the new package's `prerm failed-upgrade` attempt), and the leftover row's own text in
  the L4 and D1 screenshots (`Unfinished snapshot or delete · Delete removes it` by the
  code, `apsis.ftl:46`).

## 2026-10-06 - restore on an encrypted system disk: the spike passed on apsis-test, and what 0.6 builds on it

Branch `luks-restore-spike`, not on main, not pushed: the spike's four commits (`18e0d3b`,
`2b6e161`, `50161bc`, `888ea01`), then `410dcef`, `21c4013` and `3474b3f`. No design change
to Restore. The results are my two sittings on apsis-test (2026-10-05 and 2026-10-06) as
the guide's handoff relays them, from my pastes and screenshots; what wasn't checked says
so. The main machine appears to have the same encrypted layout: nothing was restored or
installed there. Claude Code only read code and built there.

### Found by reading (2026-10-05)

The refusal on an encrypted or LVM system disk was one comparison: lsblk's `TYPE` of the row
holding `/` had to be `part` (`refusal::check`). Nothing else in the restore looks at the
device type. The apply runs in the unlocked system, finds `/` by mount point and filesystem
UUID, keeps the live fstab and crypttab (the filter's `DISKS`), rebuilds no initrd and leaves
the ESP to kernelstub. So the same-installation case needed no new mechanism, only proof
that the boot files kernelstub writes can still unlock the disk.

A same-kernel restore doesn't give that proof: the filter's rule 10 keeps the running
kernel's files, so the snapshot's initrd is never used. The test that counts is a restore
back across a kernel update (Run B).

### The spike build

A cargo feature, `luks-spike`, off by default and refused by `just deb`: the comparison also
accepted `crypt` and `lvm`, and after kernelstub the refreshed ESP was compared with the
backup taken before the refresh (the same `root=` option, and the initrd still holds
cryptsetup, lvm and a non-empty `cryptroot/crypttab` if the one before did). A failure is
the boot refresh's failure, so the boot files are put back (`boot-kept`). The dev .deb was
`0.5.0+luksspike-1`, sha256 `82f48379bd5b706927df58d5b91b58ab4328e952f98a3d0671f6c2532e4d34f4`,
built with `--remap-path-prefix`, installed on apsis-test only.

### The layout `pre` found (apsis-test, re-installed with "Encrypt drive", Pop!_OS 24.04)

- `nvme0n1p3` holds `cryptdata` (lsblk `crypt`, `LVM2_member`), which holds `data-root`
  (`lvm`, ext4) at `/`. `cryptswap` is under `nvme0n1p4`. `/boot/efi` and `/recovery` are
  plain vfat partitions.
- The boot entry says `root=UUID=`, not a `/dev/mapper` path, and the ESP folder is named
  `Pop_OS-<uuid>` as on an unencrypted machine. That settles the 2026-09-25 note that
  `sys-uuid` from findmnt was "not verified on LVM/LUKS roots": it is the name kernelstub
  uses there too.
- `/boot/initrd.img` holds `cryptroot/crypttab` (62 bytes), `usr/sbin/cryptsetup` and
  `usr/sbin/lvm`, and the image's own crypttab is the `cryptdata` line. That settles the
  2026-10-01 claim ("an encrypted root's line is carried"), which had no machine then.
- `lsinitramfs` and `unmkinitramfs` are both there.

### Run A (2026-10-05): the same kernel

Passed on all five counts: `PRE-OK`, `MARKERS`, the unlock prompt at both restarts,
`VERIFY-OK`, `AGAIN-OK` (one more restart after it). The restore boot took about a minute.
Outcome `done`, the markers undone, the disk tables' and the boot option's sums the same,
the ESP's initrd equal to `/boot`'s, no failed units. A safety snapshot was taken. Run A
kept the running kernel's boot files, so it did not test a switch of kernel.

### Run B (2026-10-06): back across a kernel update

1. A full upgrade on the laptop: 337 upgraded, 8 newly installed; `update-initramfs` made
   the initrd for `7.1.5-76070105-generic`. After a restart that kernel ran; the snapshot
   "luks spike" (`2026-10-05_21-30-47`) was of `7.0.11-76070011-generic`.
2. Restore with the safety snapshot ticked was refused for space (finding 1). With the box
   unticked it went through to "Ready to restore".
3. Restart at 07:45 by my clock: unlock prompt, passphrase taken, the Pop logo with the
   restoring line, a second restart on its own, unlock prompt, passphrase taken, greeter at
   07:52. My words then: "exact same behaviour as last time".
4. `verify` ended `VERIFY-OK`. From its log: no `/system-update`; `"outcome": "done"`; the
   journal's `luks-spike: the initrd before Pieces { cryptsetup: true, lvm: true, crypttab:
   Some(62) }, after Pieces { cryptsetup: true, lvm: true, crypttab: Some(62) }; root= is the
   same`; `removed kernel 7.1.5-76070105-generic: not in the snapshot`; `apply-restore ended:
   Finished(Done)`; the sums the same; the running kernel `7.0.11-76070011-generic`; root
   `/dev/mapper/data-root` ext4; the ESP's initrd equal to `/boot`'s, with
   `cryptroot/crypttab` (62 bytes), `usr/sbin/cryptsetup` and `usr/sbin/lvm` in it; no line
   with FAIL; no failed units.

The pass file on the main machine holds `PRE-OK`, `MARKERS`, `VERIFY-OK`, `AGAIN-OK`,
`RUN-B-OK`.

**What this proves.** After a kernel update, the restore put the snapshot's kernel back as
the boot entry, removed the newer kernel, left the ESP's initrd equal to `/boot`'s with all
three unlock pieces, and the laptop unlocked and reached the greeter from it. The 2026-10-01
decision stands: kernelstub only, no `update-initramfs` in the apply.

**Not shown, or not tested.**

- Whether the restore copied the 7.0.11 boot image from the backup disk or left the file in
  place because it already matched. The laptop's `rsync-log` can answer it; nobody read it.
- The journal's line reads the same on both sides (62 bytes), so it alone doesn't show that
  the two images differ. The kernel versions show the switch.
- The comparison never failed (outcome `done` in both runs), so the put-back (`boot-kept`)
  has not run on real hardware on this layout.
- The runbook's rescue lines (`cryptsetup luksOpen`, `vgchange -ay` before the recovery
  note's steps) were never needed, so they are untried.
- Home wasn't in the snapshot (the fresh install's default settings exclude it; the dialog
  said "Home folders aren't in this snapshot, so they stay as they are."), so a restore
  that includes home was not tested on this layout.
- One laptop, the default "Encrypt drive" layout, the same installation. Nothing else.
- No extra restart after Run B. The guide read the key lines of the verify log, not all of
  it. The window after the Run B restore (status line, rows) was not recorded.

**Findings from Run B.**

1. The space refusal: "Can't restore this snapshot", "Not enough space on the backup disk
   for a safety snapshot (needs 7.3G, 1.9G free)." and "Delete old snapshots, or turn off
   the safety snapshot.", Close only; the window showed 1.9G free of 28G. Unticking the box
   worked as the line says. The heading is the same as for a refusal that can't be got
   around. Secondary, wording.
2. The restore boot took about 7 minutes (restart 07:45 by my clock, the apply's end
   07:52:08 in the journal), against about a minute in Run A: it had a 337-package
   upgrade to undo.
3. The backup disk is small and was nearly full, so Run B has no safety snapshot.
4. Seen again in Run A: a create's progress running backwards (99 percent with 3 s left,
   then 77 percent with 4m 34s left). Already on the after-0.5.0 list.

### What I decided

- 2026-10-05: the recovery note's unlock and LVM steps are a 0.6 item.
- 2026-10-06: the spike becomes the 0.6 feature, in this order: the real rule, the recovery
  note, the dialog's wording, this entry, then a recovery drill and a gate. The feature
  comes first; the secondary findings above wait.
- 2026-10-06: I let the rule through only for the layout that was run: a partition, then
  dm-crypt, then LVM. LUKS without LVM and LVM without LUKS stay refused until a machine has
  shown them to work.
- 2026-10-06: I kept the comparison after kernelstub where the spike had it. A refusal
  before arming (listing the snapshot's initrd while preparing, so nothing is touched) is a
  later item of its own.
- 2026-10-06: the work stays on `luks-restore-spike`, one item at a time, and each commit
  waits for my yes.

### AI help at a glance

- I ran every command of the runbook, did both sittings at the laptop, and made the
  decisions above. Nothing is committed or pushed without my yes.
- Claude Code, on the main machine, read the restore code and wrote the spike build, the
  runbook and its script, then the rule, the recovery note, the wording and this entry. It
  ran the builds, the tests, gitleaks and the commits there. It ran nothing on apsis-test
  and did not read the run logs.
- A second Claude chat (the guide) reviewed the runbook, gave me each step with its expected
  output, checked my pasted results line by line and wrote the handoff this entry draws on.
  It read a few files in the repo and ran nothing on either machine.

### Built after the spike

- **The rule** (`410dcef`). `/` may be on a plain partition, or on an `lvm` volume with
  exactly one parent that is a `crypt` mapping with FSTYPE `LVM2_member`, itself with
  exactly one parent that is a `part` with FSTYPE `crypto_LUKS`, which the live crypttab
  opens under the mapping's name by `UUID=` or `/dev/disk/by-uuid/` of that partition
  (`refusal::encrypted_root`). lsblk's rows are walked by `PKNAME` (`Device::parents`).
  Everything else is `root-device` as before: no new word on the wire. The cargo feature,
  its marker and `just deb-luks-spike` are gone.
- **The comparison** (`410dcef`), in `apsis-helper/src/unlock.rs`: runs at the end of the
  boot refresh unless the root is on a plain partition, so a plain install runs no
  `lsinitramfs` and behaves as 0.5.0 did. Its journal line now starts `unlock check:`.
- **The recovery note** (`21c4013`). On the encrypted layout, before the root's mount line:
  `sudo cryptsetup luksOpen /dev/disk/by-uuid/<the LUKS partition's> <name>` and `sudo
  vgchange -ay`, where `<name>` is the mapping's name in the live crypttab.
  `update-initramfs` in the chroot looks the root's mapping up in `/etc/crypttab` by name,
  so a disk opened under another name would get an initrd that can't unlock it; the note
  says so, and says what to do when the live system already opened the disk under another
  name. A name or UUID that isn't a plain word is refused as `root-device`. The plain note
  is byte for byte what it was. The note and the README say these lines are untried in a
  recovery.
- **The wording** (`3474b3f`): "Restore doesn't support the way this system disk is set up
  yet." / "It works on a plain partition and on Pop!_OS's standard encrypted install.
  Snapshots still work: their files are on the backup disk, under timeshift/snapshots."
  The second line is this refusal's own.

Unverified in what was built: the lsblk fixture for the encrypted layout is written by hand
from the layout above, not copied from the laptop's `lsblk --json`; the form of the root's
line in the live `/etc/crypttab` is taken to be `cryptdata UUID=... none luks` (the initrd's
62 bytes fit it, but that is the image's copy). The first Restore dialog on apsis-test with
a build of this branch settles both: if it opens without the refusal, the rule reads the
real thing.

### Corrections to assumptions

- Apsis has no snapshot browser and no single-file restore (removed in 0.4.0), so the
  refusal's text can't point to them.
- `just deb` doesn't need `just vendor`.
- The rule first proposed, "every UUID the snapshot's fstab names exists now", doesn't fit:
  Apsis never restores the snapshot's fstab, so its UUIDs can't break the restored system,
  and checking them would refuse restores that work. The crypttab half is covered by
  `crypttab-differs`, the live crypttab check above and `other-installation`.

### Not what Timeshift does, on purpose

Timeshift rewrites the target's fstab and crypttab and reinstalls GRUB because it restores
onto a mapped target (`Main.vala`: `fix_fstab_file`, `fix_crypttab_file`; read 2026-10-05).
Apsis restores in place and keeps both files. Timeshift's rewriting is where "restore onto a
reinstalled disk" would start; that is out of scope for 0.6. So are encrypted backup disks:
a separate item, after 0.6, that shares nothing with this one (the system disk is unlocked
by the initrd before Apsis runs; a backup disk would have to be unlocked by Apsis, also in
the offline boot).

### Open

- The recovery drill on apsis-test: the note's steps typed in Pop's recovery on the
  encrypted layout, which is where the unlock lines, the name's role in `update-initramfs`
  and the recovery image's tools get proven. Then a gate.
- The `rsync-log` question above; the `boot-kept` path on real hardware; a restore with
  home in the snapshot.
- A build of this branch is still version `0.5.0-1`, the release's: a .deb for apsis-test
  needs a version of its own before the drill.
- apsis-test is now encrypted, so the plain-partition layout that 0.5.0 was gated on has no
  machine. The plain path's code is unchanged; a 0.6 gate on a plain install needs a second
  device or another reinstall.
- A refusal before arming for an initrd that can't unlock; the man page, the metainfo and
  the CHANGELOG for 0.6; `tools/restore-check.sh` knows nothing of the unlock pieces.
- Not covered by anything here: a keyboard layout changed since the snapshot (the initrd
  carries the keymap for the passphrase prompt); a key file, TPM2 or FIDO2 unlock; a second
  LUKS volume for `/home`.
- Secondary, from the runs: the space refusal's heading (finding 1); the create progress
  (finding 4).

### Left on apsis-test

As the handoff has it, with what I added later; Claude Code checked none of it: Pop!_OS 24.04
with "Encrypt drive", the dev build `0.5.0+luksspike-1` still installed (the return to the
release build is not done), kernel `7.0.11-76070011-generic` with 7.1.5 removed by the
restore, so its updates should be pending again. The backup disk should hold the three base
snapshots only ("baseline1", "6b baseline", "baseline 0.5.0", never touched): I deleted
"luks spike" and Run A's safety snapshot after Run B, so any later test there starts with a
fresh snapshot.

## 2026-10-06 - the 0.6 drill passed on apsis-test: home restored, boot-kept forced, the note's lines run in the recovery

Branch `luks-restore-spike` at `a3414c3`, nine commits ahead of main, not pushed. The dev
build `0.5.0+dev1` (`just deb-dev`, sha256
`c661e7d39099f79f699663801cd5b3a1b4845b471b103b26cebccbd5ba8bd562`) was installed on
apsis-test only. The runbook is `notes/drill-runbook.md` with `notes/drill-d.sh` (both
untracked), five parts chained by pass words. All five passed in one sitting; the pass
file holds `LOG-READ`, `PRE-OK`, `RULE-OK`, `MARKED`, `HOME-OK`, `FAKE-IN`, `FAKE-OUT`
(twice each, finding 4), `UPGRADED`, `KEPT-SEEN`, `ESP-BACK`, `KEPT-OK`, `MARK1`,
`ARMED-FOR-DRILL`, `DRILL-OK`. The results are my sitting as the guide led it, from the
script's masked logs, which I kept outside the repo and Claude Code read on my say-so.
Nothing was run on apsis-test by either AI.

### Part 1, `LOG-READ`: Run B copied the snapshot's initrd

Run B's `rsync-log` (10 MB, 07:46:10 to 07:51:41) has three lines naming a kernel file in
`boot/`: the two symlinks `initrd.img` and `vmlinuz` repointed to 7.0.11, and
`>f.st...... boot/initrd.img-7.0.11-76070011-generic`. The `>f` means rsync sent the file,
`s` and `t` that its size and time differed. So Run B's boot image came from the backup
disk and was not the one the laptop already had. That closes the first open question of the
2026-10-06 spike entry. There were no `*deleting` lines for the 7.1.5 files: the filter's
rule 10 keeps the running kernel's files, so rsync never saw them (runbook finding 6).
Verified: the state folder survived the later downgrade of the package (`PRE-OK` listed
`rsync-log` and `result.json` still there).

### Part 2, `RULE-OK`: the real rule reads the real laptop

`pre` found the spike's layout: a `crypto_LUKS` partition, `cryptdata` (crypt,
`LVM2_member`), `data-root` (lvm, ext4) at `/`, root `/dev/mapper/data-root`. The live
`/etc/crypttab` line is `cryptdata UUID=<uuid> none luks`: the form the rule accepts, and
the one the spike entry called unverified. The boot entry says `root=UUID=`. Kernel
`7.0.11-76070011-generic`; the three pieces in `/boot/initrd.img`: `cryptroot/crypttab`
62 bytes, `usr/sbin/cryptsetup` 231320, `usr/sbin/lvm` 3156712. The helper's hash was the
one built here. 337 updates were waiting. The log also holds lsblk's JSON as the helper
reads it, masked: the hand-written fixture can be replaced by it.

"drill base" with "Every user's home folder" ticked took about 40 minutes and about 9.3G
(11G free before, 1.7G after; the window's figures, the script's `space` found no mount).
Restore on it opened the confirm dialog ("Restore the system?", the Home folders choice,
the safety snapshot box), cancelled. So the rule let an encrypted Pop!_OS install through
with no spike code, which the spike entry left to this dialog to settle.

### Part 3, `HOME-OK`: a restore with the home folders

Markers: `/etc/apsis-drill-m0`, cowsay installed, a new file in the home folder, a file
that said `before` changed to `after`. Restore of "drill base" (`2026-10-06_09-31-52`)
with "Restore them too" and the safety snapshot ticked (`2026-10-06_10-14-58`): about 3
minutes, the unlock prompt and the passphrase at both restarts. Outcome `done`. The
journal's new line, whole: `apsis-helper: unlock check: the initrd before has cryptsetup,
lvm, cryptroot/crypttab of 62 bytes; after, cryptsetup, lvm, cryptroot/crypttab of 62
bytes; root= is the same`. `SUMS-SAME` (crypttab, fstab, the `root=` option); the ESP
boots 7.0.11 with the three pieces; no failed units; system marker gone, cowsay gone, the
new home file gone, the changed file `before` again. Window: `System restored to
2026-10-06 09:31`, tooltip with `Home folders were restored too.` Part 3's safety
snapshot was deleted between parts.

### Part 4, `KEPT-OK`: boot-kept forced across the kernel update, and the way back

Nothing in the build forces it. A four-line stand-in for `/usr/bin/lsinitramfs` that exits
1 was in place while "drill fake" (`2026-10-06_10-49-00`) was made, then the real one was
put back (`FAKE-OUT`: dpkg's verify clean, the three pieces listed). The upgrade of Run B
again: 337 packages, kernel `7.1.5-76070105-generic`, `UPGRADED` after the restart.

K5, the restore of "drill fake" with the safety snapshot unticked, ended `boot-kept`. The
journal: `apsis-helper: the boot refresh failed (lsinitramfs couldn't list
/var/lib/apsis/restore/esp-backup/initrd.img: apsis drill: lsinitramfs stand-in, failing on
purpose): putting the boot files back`, then `the restore ended: boot-kept: ... the boot
files from before were put back` and `apply-restore ended: Finished(BootKept)`. After it
the laptop ran 7.1.5 while `/boot` linked to 7.0.11; `ESP-BOOTS-7.1.5`; the ESP's initrd
had the three pieces; `SUMS-SAME`; no failed units; the stand-in was in place, as the
snapshot had it. The window, for the first time on hardware: `System restored · still
boots the previous kernel · see README`, and the tooltip starting `Restored to <date>. The
boot files couldn't be refreshed, so the computer keeps the ones it had and runs the kernel
from before the restore.`, word for word as the code has them.

K7, predicted from the code and now seen: Restore on "drill base" was refused with `Can't
restore this snapshot`, `The boot files on this computer don't match the kernel it started
with.` and `Install the latest kernel update, restart, then try again.`

K8, by hand, the helper's own kernelstub call with 7.0.11's paths: `ESP-BACK`; kernelstub
said `No old kernel found, skipping`. K9, the restore of "drill base" with the box
unticked: outcome `done`, the `unlock check:` line with `root= is the same`, `removed
kernel 7.1.5-76070105-generic: not in the snapshot`, 7.0.11 running and on the ESP, the
real `lsinitramfs` back, no 7.1.5 file in `/boot`. "drill fake" was deleted between parts.

### Part 5, `DRILL-OK`: the note's lines run in Pop's recovery

Marker 1, then the restore of "drill base" with the safety snapshot ticked
(`2026-10-06_11-31-44`), home kept: outcome `done`, marker 1 gone. The kept note had the
unlock line, `vgchange -ay`, four mount lines, two rsync lines (a) and (b), the chroot
lines. Marker 2 after it. So the safety snapshot held marker 1 and not marker 2.

In the recovery (session recorded with `script`, 13 minutes), the lines came from my own
USB stick, plugged in there to copy and paste them from. `command -v` found
`cryptsetup`, `vgchange` and `rsync`; `lsblk -f` showed the `crypto_LUKS` partition with
nothing under it and the backup disk unmounted. `luksOpen` asked for the passphrase;
`vgchange -ay` said `1 logical volume(s) in volume group "data" now active`; `lsblk` then
showed `cryptdata` and `data-root` under the partition. The four mount lines, rsync (b),
the bind mounts, `update-initramfs -u -k all` (it rebuilt 7.0.11's initrd and ran
kernelstub itself; no `cryptsetup: WARNING` or `ERROR` line), `kernelstub --verbose` and
`rm -f /mnt/system-update` all went through. Before the restart: the chroot's
`lsinitramfs` listed the three pieces with `cryptroot/crypttab` at 62 bytes, and the boot
entry had one `root=UUID=`. A normal restart, the unlock prompt, the greeter.

Verify: marker 1 back, marker 2 gone, the initrd built 14 minutes after marker 2 (the one
made in the chroot), `ESP-BOOTS-7.0.11`, three pieces, `SUMS-SAME`, `system: running`, no
failed units.

**What this proves.** On the standard encrypted install, the same installation: a restore
with the home folders works; the boot refresh's failure path puts the boot files back and
the window and the next refusal say what they should; the recovery note's unlock and LVM
lines, run as written, bring the disk up, and `update-initramfs` in the chroot builds an
initrd that unlocks it, because the mapping was opened under the crypttab's name. The
spike entry's three "never run on hardware" items (the put-back, the rescue lines, a
restore with home) are run.

**Not shown.**

- The README's advice for the boot-kept state: K8 ran the helper's kernelstub line by hand
  instead of following the README's steps.
- The unlock check's own verdict on an initrd that lacks the pieces: the stand-in made the
  listing fail, so the comparison never ran.
- The note's "in use" paragraph (a disk the live system opened under another name).
- The by-hand lines after a restore that changed the kernel: Part 5's restore kept 7.0.11.
- Any other laptop, any other layout, LUKS without LVM, LVM without LUKS.
- The sitting took the dev build down from `0.5.0+luksspike-1`; a fresh install of
  `0.5.0+dev1` over the release build was not what ran.

**Findings** (runbook and script, nothing in the build).

1. D6's copy line used a shell glob for a file that did not exist yet, and failed. A `for`
   loop over `/mnt/home/*/drill` worked.
2. The script's `log` mode printed an empty line count: `wc` read the root-only file
   without sudo.
3. The journal tail of 70 lines in Part 3 did not reach back to `refreshing the boot
   files`; the guide inferred it from the kernelstub lines that were there.
4. K1 to K3 can be run out of order, and were: the stand-in in, out, then the create. That
   "drill fake" was deleted and the three steps redone; the pass file took a second
   `FAKE-IN` and `FAKE-OUT` without complaint. The surviving `fake` log reads
   `ALREADY-FAKE` (10:47:59, before the 10:49 create): the stand-in was in place, and the
   log of the run that printed `FAKE-IN` was overwritten.
5. A safety snapshot needs its size plus 1 GiB free (`space::backup_needs`); the
   runbook's "at least 1G" at D1 was too low and H2 had no number. 1.5G was the working
   figure, with 1.6G to 1.7G free each time.
6. L1 expected `*deleting` lines for the 7.1.5 files; rule 10 protects them, so there
   were none, and `INITRD-COPIED` was the answer either way.
7. Part 4's "Nothing in the build" had no proof line; "The way back" leaned on Part 5's
   lines before they were proven; P3 did not record the "drill base" row's date and time.
8. The note and the README say the unlock lines are untried in a recovery. They are tried
   now: the lines change (a commit of its own).

### What I decided

- 2026-10-06: still one item at a time, each commit on my yes, nothing pushed until I say.

### AI help at a glance

- I sat at the laptop for all five parts, pasted the recovery's lines from my USB stick,
  read the dialogs and the window, and wrote the results above in my own words.
- The guide (a second Claude chat) gave me each step with its expected output, read short
  greps of each log as I pasted them, and said when a pass word was missing. It ran nothing.
- Claude Code, on the main machine, wrote the runbook, the script and the dev build's
  recipe before the sitting, and afterwards read the logs I named and drafted this entry.
  It ran nothing on apsis-test and installed nothing here.

### Left on apsis-test

`0.5.0+dev1` installed, kernel 7.0.11 running and on the ESP, 337 updates pending again,
the real `lsinitramfs`. The backup disk holds the three baselines only, 11G free. The
markers are removed; `~/drill/` stays, its `session.txt` holds the laptop's UUIDs and
does not leave it.
