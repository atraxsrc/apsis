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
