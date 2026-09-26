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
    `no-manual-page`, `initial-upload-closes-no-bugs` (Debian archive only).
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
