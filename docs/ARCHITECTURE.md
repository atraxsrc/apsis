# Architecture

Apsis 0.2.0 is standalone: it takes rsync snapshots itself and never runs `timeshift`. It keeps
Timeshift's on-disk layout, so Timeshift-made snapshots keep working. Manual only: no schedule,
no automatic deletion. (Up to 0.1.x Apsis drove the `timeshift` command line; that design is in
git history, DECISIONS.md and TIMESHIFT-CLI.md.)

## Crates

| crate | kind | depends on | job |
|---|---|---|---|
| `apsis-core` | lib | no UI crates | Snapshot model, `Backend` trait, the native rsync backend, Apsis's config (and the import from Timeshift's settings), keep-last-N, file-level restore, the helper's wire types and client |
| `apsis` | bin (applet) | libcosmic, apsis-core | Panel button + terminal-style popup |
| `apsis-helper` | bin | zbus, apsis-core | Root D-Bus service guarded by polkit |

`apsis-core` must stay UI-free and testable without root or a COSMIC session.

## Backend trait

```rust
pub trait Backend {
    fn list(&self) -> Result<SnapshotList>;
    fn create(&self, comment: &str) -> Result<()>;   // empty comment = no comment
    fn delete(&self, name: &str) -> Result<()>;      // name must be YYYY-MM-DD_HH-MM-SS
}

pub struct SnapshotList {
    pub device: Option<String>,   // /dev/sdX1; None when no device is chosen
    pub uuid: Option<String>,     // backup device UUID
    pub mode: Option<Mode>,       // Rsync (the wire format still has btrfs)
    pub snapshots: Vec<Snapshot>,
    pub warnings: Vec<String>,    // incomplete folders, a leftover staging folder
    pub usage: Option<DiskUsage>, // statvfs of the backup device, when known
}

pub struct Snapshot {
    pub name: String,             // YYYY-MM-DD_HH-MM-SS
    pub created: jiff::civil::DateTime,  // local time, from the name
    pub tags: Vec<Tag>,           // OnDemand | Boot | Hourly | Daily | Weekly | Monthly
    pub comment: Option<String>,
}
```

The one implementation is `native::NativeRsync<R: Runner>`: Timeshift's rsync snapshots in
Timeshift's exact layout (see DECISIONS.md, Phase 5), so Timeshift reads, uses and deletes
Apsis's and the other way round. `NativeConfig` says where: `repo` (the backup device's mount),
`source` (`/` for real), `sys_uuid`, `sys_distro` and the `exclude` list
(`native::exclude::for_backup`, Timeshift's algorithm, from the config's filters).

- `list`: reads `timeshift/snapshots/*/info.json`; folders Timeshift would count as incomplete,
  and a leftover staging folder, are warnings.
- `create`: `plan` works out the name (local time), the `--link-dest` snapshot (newest valid
  one with this `sys-uuid`), `exclude.list`, the rsync argv and `info.json` (tag `ondemand`).
  It's built in `timeshift/apsis-staging/<name>/` (rsync through `QuietRunner`: fixed `PATH`,
  `ionice -c 3 nice -n <to 19>`, `--info=progress2` for the progress), checked like Timeshift
  checks it (a total size in `rsync-log`), renamed into `snapshots/`, and the
  `snapshots-<tag>/` links rebuilt. A failure removes the staging folder.
- `delete` (`delete_snapshot`), as root, only a plain snapshot folder, refused before anything
  is deleted otherwise:
  - the name matches `YYYY-MM-DD_HH-MM-SS` (so never `snapshots/` itself, `..` or a path);
  - `timeshift/`, `snapshots/` and `<name>/` are each opened with `openat(O_NOFOLLOW |
    O_DIRECTORY)` from the mount: a symlink anywhere on the way is refused;
  - `<name>/info.json` is a regular file;
  - nothing is mounted at or below `<name>/` (`/proc/self/mountinfo`, `usage::mounts_under`):
    a bind mount has the same `st_dev`, so the walk alone couldn't tell;
  - removal is `native::prune::remove_at`: every folder opened with `O_NOFOLLOW` relative to
    the one above, symlinks removed as links, and a folder on another filesystem stops it;
  - then `snapshots-<tag>/<name>` links are removed (only symlinks, only that name).
- Refuses to write through a symlinked `timeshift/`, `snapshots/` or staging folder.

The applet calls the helper on a background task (libcosmic `Task`) and never blocks the UI
thread. `helper::HelperClient` is the applet's client: async (zbus on the applet's tokio),
since create, delete and restore wait for a signal.

## Config

`/etc/apsis/config.toml` (`apsis_core::config`), written only by the helper:

```toml
# Written by apsis-helper; change it in Apsis's settings.
# filters: rsync patterns, first match wins; "+ " in front includes.
version = 1
backup_device_uuid = "8cecb045-975d-49d0-bd57-1ec9f6eb77b5"
filters = [
    "+ /home/user1/**",
    "/var/lib/libvirt/**",
]
```

- `filters` is one ordered list in Timeshift's format, home folder patterns included
  (`<home>/**`, `+ <home>/.**`, `+ <home>/**`): rsync takes the first match, so the order is
  kept as it is. The settings view shows a home row per user, read from the list.
- **Import**: while there's no `config.toml`, `config::effective` imports Timeshift's
  `/etc/timeshift/timeshift.json` (only read): its `backup_device_uuid` and `exclude` list, with
  notes on what was and wasn't taken. List and create use it until it's saved. No
  `timeshift.json`: an empty config. A `config.toml` that can't be read is an error, not an
  import.
- `config::validate`: a device that differs from the saved one must be connected and a plain,
  unencrypted Linux filesystem (`Device::selectable`); filters not blank, no control
  characters, not repeated.

## Privilege model

- The applet runs as the user and is never setuid and never run as root (Wayland GUIs must not
  run as root).
- `apsis-helper` is a system service, D-Bus activated. Each method checks a polkit action:
  - `<app-id>.list` → `allow_active=yes` (list, read the config)
  - `<app-id>.create`, `<app-id>.delete` → `auth_admin_keep`
  - `<app-id>.configure` (write `/etc/apsis/config.toml`) → `auth_admin_keep`
  - `<app-id>.browse` (browsing a snapshot's files and restore dry runs) → `auth_admin_keep`:
    snapshots hold root-only files
  - `<app-id>.restore` (folder mode) → `auth_admin_keep`
  - `<app-id>.restore-original` (original mode) → `auth_admin`, asked every time
- Without the helper the applet does nothing but say so; there's no `pkexec` fallback.
- Input validation in the helper: snapshot names must match `YYYY-MM-DD_HH-MM-SS`; comments are
  length-limited (they go into `info.json`); configs and restore requests are checked again.

## apsis-helper

All names (bus name, path, interface, methods, signals, error names, polkit action IDs, unit)
are constants in `apsis_core::helper::names`; the helper's tests check its interface and the
files in `resources/helper/` against them.

| | |
|---|---|
| bus name | `io.github.atraxsrc.Apsis.Helper` (system bus, owned by root only) |
| object / interface | `/io/github/atraxsrc/Apsis/Helper`, `io.github.atraxsrc.Apsis.Helper1` |
| `NativeListWithUsage() -> ((sssa(sss)as)a{st})` | `(device, uuid, mode, [(name, tags, comment)], warnings)` and the backup device's usage in bytes (`total`, `used`, `free`, all or none, `statvfs` while mounted); polkit `list`, not interactive |
| `NativeCreate(s comment)` | a snapshot; polkit `create`, interactive; returns once started, `Finished("create", ..)` follows |
| `Delete(s name)` | one snapshot (see Backend above), only a name the fresh list has; polkit `delete`, interactive; returns once started, `Finished("delete", ..)` follows |
| `ReadConfig() -> (s(sas)sa(ssb)as)` | `(config.toml text or empty, the config in effect, lsblk JSON, [(user, home, encrypted)], import notes)`; polkit `list`, not interactive |
| `WriteConfig(s expected, (sas) config) -> s` | writes `/etc/apsis/config.toml` if it still reads `expected` (empty: none yet); polkit `configure`, interactive; returns once done |
| `Browse(s snapshot, s path) -> (a(sstxuuussstx)b)` | one folder of a snapshot, see below; polkit `browse`, interactive |
| `Restore(s snapshot, as paths, s destination, b dry_run)` | polkit `browse` (dry run), `restore` (folder) or `restore-original` (original), interactive; returns once started, `Finished("restore", ..)` follows with the plan or result |
| `Progress(s op, d percent, x eta_seconds, s text)` | signal, only to the caller, while a create or real restore runs: at most one per 500 ms (the `100%` one always), all sent before `Finished`; `-1` = unknown. The first has no numbers (progress will come). From rsync's `--info=progress2` |
| `Finished(s op, b ok, s message)` | signal, sent only to the caller that started the operation; `message` is the error, or for a restore the plan/result text |
| errors | `...Helper1.Error.{NotAuthorized,Busy,InvalidInput,Failed,DeviceNotFound,Changed}` |

`helper::encode_error` keeps an error's kind across the bus (a missing disk, a refusal
`refused: `, a failed restore `restore failed: `); anything else is its text.

Each call, in order:
1. Input checked again (`validate_comment`, snapshot name pattern, the config); the applet
   isn't trusted.
2. Refused with `Busy` if another operation runs, before any password dialog.
3. polkit `CheckAuthorization` with subject `system-bus-name` = the caller's unique bus name (not
   a PID, so a reused PID can't inherit an answer); `AllowUserInteraction` for everything but
   `list`. No answer from polkit counts as a no.
4. The single-operation lock is taken; a second call gets `Busy`, never waits in a queue. Every
   use of the backup mount point goes through it.
5. Tools run with a fixed argv, found on a fixed `PATH`, with a cleared environment
   (`HOME=/root`, `USER`/`LOGNAME=root`, `LC_ALL=C.UTF-8`) and stdin null.
6. The backup device from the config is mounted at `/run/apsis/backup` by UUID for the call:
   `ro,nosuid,nodev,noexec` for list, browse and restore, `rw,nosuid,nodev` for create and
   delete; unmounted when the call ends. Encrypted devices and anything that isn't a Linux
   filesystem are refused.
7. Create, delete and restore return as soon as they start. When they end, the lock is
   released and `Finished` is sent to the caller.
8. Every call and its result is logged to the journal (`journalctl -u apsis-helper`); comments
   are cut to 40 characters. A delete logs `delete "<name>" for :1.42: started`, the path it
   deleted, and `done` or the reason.

Long operations don't depend on any D-Bus call timeout: the only long wait inside a method
call is the password dialog, and zbus sets no call timeout by default. The applet waits for
`Finished`, or for the helper to leave the bus without sending one, which it reports as an error.

The helper exits after 60 s with no call open and nothing running. It never exits while an
operation runs or a call (including one waiting for the password dialog) is open, and it waits
until each `Finished` is sent. The next call starts it again.

### File-level restore (phase 6a)

`apsis_core::restore` has the rules and runs rsync; it needs no root, so the tests run it on
folders of their own and on the ext4 image. The helper (`apsis-helper/src/restore.rs`) adds the
real places: the backup device from the config (`native::backup_device`, unencrypted) mounted
`ro,nosuid,nodev,noexec` at `/run/apsis/backup` for one call, the running system at `/`, the
caller's uid from the bus (`GetConnectionUnixUser`) with home and gid from `/etc/passwd`, names
from `/etc/passwd` and `/etc/group`.

- Both methods take the single-operation lock and are refused (`Busy`) while another operation
  runs.
- Paths: `SnapPath` (absolute, no NUL, no empty/`.`/`..` component) resolved under
  `timeshift/snapshots/<name>/localhost/` with `lstat` one component at a time; a symlink
  before the last component is refused, a last-component symlink is copied as a link unless
  its relative target climbs above the snapshot's root. `timeshift/`, `snapshots/`, `<name>/`
  and `localhost/` must be real folders.
- `Browse`: at most 10,000 entries, sorted by name. Each entry is compared with the same path
  under `/` (`lstat`, never following): `missing`, `same` (type, and size + mtime seconds for a
  file, target for a link), `changed`, `present` (folders, not compared recursively).
- Folder mode: `<home>/Apsis-restored/<snapshot>[-n]/`, made through `openat(O_NOFOLLOW |
  O_DIRECTORY)` from `/`, held open, 0700 root while rsync writes to `/proc/self/fd/<n>/`
  (the descriptor inherited by rsync), then `fchown`/`fchmod 0755` on the descriptor. rsync:
  `--relative --ignore-existing --no-devices --no-specials --chmod=ug-s '--filter=-x
  security.*' --chown=<uid>:<gid>`.
- Original mode: one rsync per path into its live parent, `--backup
  --suffix=.apsis-before-<snapshot>`. Refused under `/proc /sys /dev /run /tmp /boot`, for
  `/`, when the live parent is missing or reached through a symlink, when the path is on the
  backup device's filesystem (`st_dev`), and when a backup name it would take already exists
  (found by a dry run first, always). `/etc` and `/usr` are warned about in the plan.
- rsync as argv through `RsyncRunner` (fixed `PATH`, `LC_ALL=C.UTF-8`, stdin null, stdout kept
  for the plan). Exit 0 only; 23/24 are "finished with errors".
- Journal: `restore "folder" dry run "2026-09-25_03-00-01" ["/etc/hosts", +2 more] for :1.42:
  started`, then the summary with the uid and `done`, or the error. Browse logs the folder and
  the entry count.

## Theming

- libcosmic widgets pick up the computed COSMIC theme automatically and update live.
- Use theme tokens only (`cosmic::theme::active()` palette / container styles). No hex literals.
- Monospace everywhere in the popup via libcosmic's monospace font.

## Files installed (by `just install` or the .deb)

- `/usr/bin/apsis`
- `/usr/share/applications/<app-id>.desktop` (with `X-CosmicApplet=true` as the template sets)
  and `<app-id>.Window.desktop` (the launcher entry, `apsis --window`)
- `/usr/share/icons/hicolor/{scalable,symbolic}/apps/<app-id>{,-symbolic}.svg`
- `/usr/share/metainfo/<app-id>.metainfo.xml`
- `/usr/share/man/man1/apsis.1.gz` - `docs/apsis.1`, gzipped with `-9n`
- `/usr/libexec/apsis-helper` (0755)
- `/usr/share/dbus-1/system-services/io.github.atraxsrc.Apsis.Helper.service` - D-Bus activation
- `/usr/share/dbus-1/system.d/io.github.atraxsrc.Apsis.Helper.conf` - bus policy
- `/usr/lib/systemd/system/apsis-helper.service` - `Type=dbus`, no `[Install]`, not sandboxed
  (a snapshot reads the whole filesystem, and the helper mounts devices)
- `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy` - the seven actions

Made at run time: `/etc/apsis/config.toml` (and `.bak`), by the helper; the .deb's postrm
removes `/etc/apsis/` on purge.

The activation file and unit come from `resources/helper/*.in` with `@libexecdir@` filled in.
Afterwards `just install` runs `systemctl daemon-reload` and the bus's `ReloadConfig` (dbus-broker
doesn't watch its config directories); both are skipped when `rootdir` is set for packaging.
`just uninstall` stops `apsis-helper.service` first.
