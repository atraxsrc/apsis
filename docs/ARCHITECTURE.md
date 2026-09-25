# Architecture

## Crates

| crate | kind | depends on | job |
|---|---|---|---|
| `apsis-core` | lib | no UI crates | Snapshot model, `Backend` trait, Timeshift CLI backend, output parser |
| `apsis` | bin (applet) | libcosmic, apsis-core | Panel button + terminal-style popup |
| `apsis-helper` | bin (phase 4) | zbus, apsis-core | Root D-Bus service guarded by polkit |

`apsis-core` must stay UI-free and testable without root or a COSMIC session.

## Backend trait (as built in Phase 1)

```rust
pub trait Backend {
    fn list(&self) -> Result<SnapshotList>;
    fn create(&self, comment: &str) -> Result<()>;   // empty comment = no comment
    fn delete(&self, name: &str) -> Result<()>;      // name must be YYYY-MM-DD_HH-MM-SS
}

pub struct SnapshotList {
    pub device: Option<String>,   // as reported by timeshift; None when "Not Selected"
    pub uuid: Option<String>,     // backup device UUID; preferred for --snapshot-device
    pub mode: Option<Mode>,       // Btrfs | Rsync
    pub snapshots: Vec<Snapshot>,
}

pub struct Snapshot {
    pub name: String,             // YYYY-MM-DD_HH-MM-SS
    pub created: jiff::civil::DateTime,  // local time, from the name
    pub tags: Vec<Tag>,           // OnDemand | Boot | Hourly | Daily | Weekly | Monthly
    pub comment: Option<String>,
}
```

Implementations:
- `TimeshiftCli<R: Runner>` - builds argv (`Vec<OsString>`, `argv[0] = "timeshift"`) and runs it
  through the `Runner` trait so tests can inject canned output. Never passes user text through a
  shell; the comment is a single argv element. After each successful `list()` it remembers the
  device (UUID, else path) and adds `--snapshot-device` to later calls. `create`/`delete` refuse to run
  (`Error::NoSnapshotDevice`) until a list has shown a device, so they never fall back to
  Timeshift's configured default. The applet uses
  `PkexecRunner` (`pkexec --disable-internal-agent /abs/path/timeshift ...`, see DECISIONS.md).
- `helper::HelperClient` (phase 4) - the applet's client for `apsis-helper` over the system bus.
  It is async (zbus on the applet's tokio) rather than a `Backend`: create/delete wait for a
  signal, which doesn't fit a blocking trait. Same results and errors as the CLI backend.
- `Native` (phase 5) - btrfs / rsync.

The applet calls the backend on a background task (libcosmic `Task`) and never blocks the UI thread.

## Privilege model

- The applet runs as the user and is never setuid and never run as root (Wayland GUIs must not run as root).
- Phases 2–3: `pkexec timeshift …` — COSMIC's polkit agent shows the password prompt.
  Downside: a prompt per call. Since Phase 4 this is the fallback when the helper isn't installed.
- Phase 4: `apsis-helper` system service, D-Bus activated. Each method checks a polkit action:
  - `<app-id>.list` → `allow_active=yes`
  - `<app-id>.create`, `<app-id>.delete` → `auth_admin_keep`
  - `<app-id>.restore` (phase 6) → `auth_admin` (no caching)
- Input validation in the helper: snapshot names must match Timeshift's pattern
  (`YYYY-MM-DD_HH-MM-SS`); comments are length-limited and passed as argv, never a shell.

## apsis-helper (phase 4)

All names (bus name, path, interface, methods, signal, error names, polkit action IDs, unit) are
constants in `apsis_core::helper::names`; the helper's tests check its interface and the files in
`resources/helper/` against them.

| | |
|---|---|
| bus name | `io.github.atraxsrc.Apsis.Helper` (system bus, owned by root only) |
| object / interface | `/io/github/atraxsrc/Apsis/Helper`, `io.github.atraxsrc.Apsis.Helper1` |
| `List() -> (sssa(sss)as)` | `(device, uuid, mode, [(name, tags, comment)], warnings)`; polkit `list`, not interactive |
| `Create(s comment)` | polkit `create`, interactive; returns once started |
| `Delete(s name)` | polkit `delete`, interactive; returns once started |
| `Finished(s op, b ok, s message)` | signal, sent only to the caller that started the create/delete |
| errors | `...Helper1.Error.{NotAuthorized,Busy,InvalidInput,NotInstalled,DeviceNotFound,Failed}` |

Each call, in order:
1. Input checked again (`validate_comment`, snapshot name pattern); the applet isn't trusted.
2. Create/Delete: refused with `Busy` if Timeshift is already running, before any password dialog.
3. polkit `CheckAuthorization` with subject `system-bus-name` = the caller's unique bus name (not
   a PID, so a reused PID can't inherit an answer); `AllowUserInteraction` for create/delete.
   No answer from polkit counts as a no.
4. The single-operation lock is taken; a second call gets `Busy`, never waits in a queue.
5. `timeshift` runs with a fixed argv, found on a fixed `PATH`, with a cleared environment
   (`HOME=/root`, `USER`/`LOGNAME=root`, `LC_ALL=C.UTF-8`) and stdin null. Create and delete run
   a fresh `--list` first and target the device it reports; delete only deletes a name in it.
6. Create/Delete return as soon as Timeshift starts. When it ends, the lock is released and
   `Finished` is sent to the caller. A Timeshift failure's message carries the exit code and
   Timeshift's last 5 lines (its `E:`/`W:` lines, which it prints on stdout, plus stderr); a
   missing disk carries the device (`helper::encode_error`).
7. Every call and its result is logged to the journal (`journalctl -u apsis-helper`); comments
   are cut to 40 characters.

Long operations don't depend on any D-Bus call timeout: the only long wait inside a method
call is the password dialog, and zbus sets no call timeout by default. The applet waits for
`Finished`, or for the helper to leave the bus without sending one, which it reports as an error.

The helper exits after 60 s with no call open and nothing running. It never exits while
Timeshift runs or a call (including one waiting for the password dialog) is open, and it waits
until each `Finished` is sent. The next call starts it again.

The applet asks the bus before each list/create/delete whether the helper is installed
(activatable) or running. If it is, the helper is used. If not, or there's no system bus, it
falls back to pkexec. An installed helper that fails is reported as an error, not replaced by
pkexec.

## Theming

- libcosmic widgets pick up the computed COSMIC theme automatically and update live.
- Use theme tokens only (`cosmic::theme::active()` palette / container styles). No hex literals.
- Monospace everywhere in the popup via libcosmic's monospace font.

## Files installed (by `just install`, run by the user)

- `/usr/bin/apsis`
- `/usr/share/applications/<app-id>.desktop` (with `X-CosmicApplet=true` as the template sets)
- `/usr/share/icons/hicolor/{scalable,symbolic}/apps/<app-id>{,-symbolic}.svg`
- `/usr/share/metainfo/<app-id>.metainfo.xml`
- `/usr/libexec/apsis-helper` (0755)
- `/usr/share/dbus-1/system-services/io.github.atraxsrc.Apsis.Helper.service` - D-Bus activation
- `/usr/share/dbus-1/system.d/io.github.atraxsrc.Apsis.Helper.conf` - bus policy
- `/usr/lib/systemd/system/apsis-helper.service` - `Type=dbus`, no `[Install]`, not sandboxed
  (Timeshift needs the whole filesystem and mounts)
- `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy` - the three actions

The activation file and unit come from `resources/helper/*.in` with `@libexecdir@` filled in.
Afterwards `just install` runs `systemctl daemon-reload` and the bus's `ReloadConfig` (dbus-broker
doesn't watch its config directories); both are skipped when `rootdir` is set for packaging.
`just uninstall` stops `apsis-helper.service` first.
