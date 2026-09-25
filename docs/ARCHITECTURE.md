# Architecture

## Crates

| crate | kind | depends on | job |
|---|---|---|---|
| `apsis-core` | lib | no UI crates | Snapshot model, `Backend` trait, Timeshift CLI backend, output parser, native rsync backend |
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
- `native::NativeRsync<R: Runner>` (phase 5, rsync only; btrfs is 5.1) - Timeshift's rsync
  snapshots without `timeshift`, in Timeshift's exact layout (see DECISIONS.md, Phase 5), so
  either tool reads, uses and deletes the other's. `NativeConfig` says where: `repo` (the backup
  device's mount), `source` (`/` for real), `sys_uuid`, `sys_distro`, the `exclude` list
  (`native::exclude::for_backup`, Timeshift's algorithm) and `dry_run`.
  - `list`: reads `timeshift/snapshots/*/info.json`; folders Timeshift would count as incomplete
    are warnings.
  - `create`: `plan` works out the name (local time), the `--link-dest` snapshot (newest valid
    one with this `sys-uuid`), `exclude.list`, the rsync argv and `info.json`. With `dry_run`
    the plan is only logged. Otherwise it's built in `timeshift/apsis-staging/<name>/` (rsync
    through `QuietRunner`: fixed `PATH`, stdout discarded, stderr tail kept), checked like
    Timeshift checks it (a total size in `rsync-log`), renamed into `snapshots/`, and the
    `snapshots-<tag>/` links rebuilt. A failure removes the staging folder.
  - `delete`: removes the folder and rebuilds the links. The applet doesn't use it (it deletes
    through Timeshift); it's there for the trait and the tests.
  - Refuses to write through a symlinked `timeshift/`, `snapshots/` or staging folder.

The applet calls the backend on a background task (libcosmic `Task`) and never blocks the UI thread.

## Privilege model

- The applet runs as the user and is never setuid and never run as root (Wayland GUIs must not run as root).
- Phases 2–3: `pkexec timeshift …` — COSMIC's polkit agent shows the password prompt.
  Downside: a prompt per call. Since Phase 4 this is the fallback when the helper isn't installed.
- Phase 4: `apsis-helper` system service, D-Bus activated. Each method checks a polkit action:
  - `<app-id>.list` → `allow_active=yes`
  - `<app-id>.create`, `<app-id>.delete` → `auth_admin_keep`
  - `<app-id>.configure` (phase 4.5, writing Timeshift's settings) → `auth_admin_keep`
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
| `ReadSettings() -> (ssa(ssb)b)` | `(timeshift.json text, lsblk JSON, [(user, home, encrypted)], timeshift-gtk open)`; polkit `list`, not interactive |
| `WriteSettings(s expected, (sbbabauas) settings) -> s` | polkit `configure`, interactive; writes `/etc/timeshift/timeshift.json`, returns once done (see below) |
| `NativeList() -> (sssa(sss)as)` | native backend list, same shape as `List`; polkit `list`, not interactive |
| `NativeDryRun(s comment) -> s` | the native create's plan as text, also logged; nothing written; polkit `list`, not interactive |
| `NativeCreate(s comment)` | native rsync snapshot; polkit `create`, interactive; returns once started, `Finished("create", ..)` follows |
| `Finished(s op, b ok, s message)` | signal, sent only to the caller that started the create/delete |
| errors | `...Helper1.Error.{NotAuthorized,Busy,InvalidInput,NotInstalled,DeviceNotFound,Failed,Changed}` |

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

### Native backend (phase 5)

`NativeList`, `NativeDryRun` and `NativeCreate` build the backend from the system
(`apsis-helper/src/native.rs`), holding the single-operation lock:

1. Timeshift's settings file (read only): the backup device UUID and the user filters. btrfs
   mode is refused (rsync only for now), as is a device that isn't connected, is encrypted, or
   isn't a Linux filesystem (Timeshift unlocks LUKS; the native backend doesn't).
2. `lsblk` for the device, `findmnt --noheadings --output UUID --mountpoint /` for `sys-uuid`,
   `/etc/lsb-release` or `/etc/os-release` for `sys-distro`, `/etc/fstab` and `/etc/passwd`
   (with ecryptfs folders) for the exclude list.
3. Whatever is mounted at `/run/apsis/backup` is unmounted, then `mount -o
   ro|rw,nosuid,nodev /dev/disk/by-uuid/<uuid> /run/apsis/backup`: read-only for list and dry
   run, read-write for create. Unmounted when the call ends.
4. `NativeCreate` is refused (`Busy`) while a Timeshift holds its lock
   (`/var/run/lock/timeshift/lock` with a live `timeshift` PID), checked before the password
   dialog and again after it.

Native create runs rsync as root over the whole of `/` with Timeshift's filters, like Timeshift.

### Settings (phase 4.5)

`WriteSettings(expected, settings)` edits `/etc/timeshift/timeshift.json` and nothing else. The
settings are `(backup device UUID, btrfs mode, include @home, [5 schedules], [5 counts],
[filters])`, levels monthly, weekly, daily, hourly, boot. In order:

1. Refused with `Busy` if Timeshift is running, then polkit `configure` (password, cached).
2. The single-operation lock is taken, so no list, create or delete runs meanwhile.
3. `lsblk` (fixed argv, `settings::LSBLK_ARGS`) for the connected devices.
4. The file is read; if it isn't exactly `expected` (what the caller read), `Changed`.
5. `settings::edit` checks and applies them (`InvalidInput` with the reason if not): a new
   device must be connected, unencrypted and a Linux filesystem (btrfs in btrfs mode); btrfs mode
   needs a btrfs filesystem; counts 1-999; filters not blank, no control characters, no
   duplicates. Only Timeshift's own fields change, all written as strings like Timeshift writes
   them; every other field stays in place. The result must read back as exactly the settings.
6. The old file is copied to `timeshift.json.bak`, then the new text replaces the file: temp file
   in `/etc/timeshift/`, `fsync`, `rename`, `fsync` of the folder. The mode is kept. The file is
   checked against `expected` again just before the swap.
7. The remembered `--snapshot-device` is dropped, and one `timeshift --list` runs: every Timeshift
   run syncs its cron jobs (`/etc/cron.d/timeshift-{hourly,boot}`) with the settings on exit, so
   this puts the new schedule in place. If that list fails, the settings stay written and the
   returned text says so; an empty text means all went well.

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
- `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy` - the four actions

The activation file and unit come from `resources/helper/*.in` with `@libexecdir@` filled in.
Afterwards `just install` runs `systemctl daemon-reload` and the bus's `ReloadConfig` (dbus-broker
doesn't watch its config directories); both are skipped when `rootdir` is set for packaging.
`just uninstall` stops `apsis-helper.service` first.
