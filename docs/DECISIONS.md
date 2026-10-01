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
