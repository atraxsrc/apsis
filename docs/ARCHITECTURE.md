# Architecture

Apsis is standalone: it takes rsync snapshots itself and never runs `timeshift`. It keeps
Timeshift's on-disk layout, so Timeshift-made snapshots keep working. Manual only: no schedule,
no automatic deletion. Since 0.4.0 it does four things: snapshot the system (with `/root` and
`/home` as choices), a filter list, delete snapshots, and (0.5.0) restore the whole system.
(Up to 0.1.x Apsis drove the `timeshift` command line; file-level restore existed from 0.1 to
0.3. Both are in git history and DECISIONS.md.)

## Crates

| crate | kind | depends on | job |
|---|---|---|---|
| `apsis-core` | lib | no UI crates | Snapshot model, `Backend` trait, the native rsync backend (with stop and leftovers), Apsis's config (v2, converting v1 and Timeshift's settings), job status, the helper's wire types and client |
| `apsis` | bin (applet) | libcosmic, apsis-core | Panel button, read-only popup, and the window (toolbar, list, status area, settings tabs) |
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
    pub warnings: Vec<String>,    // incomplete folders, odd things in the staging folder
    pub leftovers: Vec<String>,   // interrupted creates' folders in apsis-staging/
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
(`native::exclude::for_backup`: Timeshift's defaults, then the config's filters with the
parent folders of each `+` path let in, then `+ /root/**` / `+ /home/**` if included, then the
built-in `/root/**` and `/home/*/**`).

- `list`: reads `timeshift/snapshots/*/info.json`; folders Timeshift would count as incomplete
  are warnings; snapshot-named folders in `timeshift/apsis-staging/` are `leftovers` (anything
  else there is a warning and is never touched).
- `create`: `plan` works out the name (local time), the `--link-dest` snapshot (newest valid
  one with this `sys-uuid`), `exclude.list`, the rsync argv and `info.json` (tag `ondemand`).
  Since 0.4.1 the argv has `-A -X --numeric-ids` (ACLs, extended attributes, owners by number;
  no `-H`), and `info.json` ends with `"apsis-rsync-flags" : "-aAX --numeric-ids"`. A snapshot
  without that key (Timeshift's, Apsis 0.4.0 and older) is the old format.
  It's built in `timeshift/apsis-staging/<name>/` (rsync through `QuietRunner`: fixed `PATH`,
  `ionice -c 3 nice -n <to 19>`, `--info=progress2` for the progress), checked like Timeshift
  checks it (a total size in `rsync-log`), renamed into `snapshots/`, and the
  `snapshots-<tag>/` links rebuilt. Leftovers are removed first. A failure or a stop removes
  the staging folder with `remove_staging` (the delete's rules: `O_NOFOLLOW` from the mount, a
  snapshot name, nothing mounted inside, `prune::remove_at`).
- stop (`native::Cancel`): rsync runs in its own process group; `request()` sends `SIGTERM` to
  it and `SIGKILL` after 10 s; the runner waits with `WNOWAIT` before reaping so a signal never
  reaches a reused pid. The create checks the `Cancel` before and after rsync and `commit()`s
  right before the rename; a stop after that is refused (`TooLate`).
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
- `delete` of a name that isn't a snapshot but is a leftover removes the staging folder
  (`remove_staging`); anything else goes through `delete_snapshot`.
- Refuses to write through a symlinked `timeshift/`, `snapshots/` or staging folder.

The applet calls the helper on a background task (libcosmic `Task`) and never blocks the UI
thread. `helper::HelperClient` is the applet's client: async (zbus on the applet's tokio),
since create and delete wait for a signal. One per process (cosmic-panel runs one applet
process per display, plus the window): made at start, kept in the model, re-made when the bus
drops, so all of a process's calls come from one bus name. `job()` asks `NameHasOwner` first
so it never starts the helper just to hear it's idle; `job_changes()` streams `JobChanged` and
the helper leaving the bus, without starting it. A process lists once per create or delete end
it hears of; a selection of several is one `DeleteMany` call.

## Config

`/etc/apsis/config.toml` (`apsis_core::config`), written only by the helper:

```toml
# Written by apsis-helper; change it in Apsis's settings.
# filters: first match wins, top to bottom; "- " leaves out, "+ " keeps.
# Built-in excludes (/proc, /dev, caches, ...) always come first.
version = 2
backup_device_uuid = "8cecb045-975d-49d0-bd57-1ec9f6eb77b5"
include_root = true
include_home = false
filters = [
    "+ /home/user1/Videos/keep/***",
    "- /home/*/Videos/***",
]
```

- `Config::read` gives `Stored::Current` (version 2) or `Stored::Legacy` (version 1, Apsis 0.2
  and 0.3: one filter list in Timeshift's format with per-user home patterns). Fresh config:
  `/root` in, `/home` out.
- **Conversion** (`config::convert`): v1 and Timeshift's `timeshift.json` go through it. It
  simplifies only when the result provably takes the same files (a home that was "everything"
  for every user, nothing else in `/home`, no later filter shadowed), else it keeps explicit
  rows; "hidden files only" becomes a `+ <home>/.**` row. It needs the users and the names in
  `/home`, which the helper reads. The result is used at once and written as v2 on Save (the v1
  file becomes `.bak`). A test runs real rsync with both lists and compares what they copy.
- **Import**: while there's no `config.toml`, `config::effective` imports Timeshift's
  `/etc/timeshift/timeshift.json` (only read), converted, with notes. No `timeshift.json`: the
  defaults. A `config.toml` that can't be read is an error, not an import.
- `config::validate`: a device that differs from the saved one must be connected and a plain,
  unencrypted Linux filesystem (`Device::selectable`); filters signed (`+ `/`- `), not blank, no
  control characters, not repeated.

## Privilege model

- The applet runs as the user and is never setuid and never run as root (Wayland GUIs must not
  run as root).
- `apsis-helper` is a system service, D-Bus activated. Each method checks a polkit action:
  - `<app-id>.list` → `allow_active=yes` (list, read the config, ask the job)
  - `<app-id>.create`, `<app-id>.delete` → `auth_admin_keep`
  - `<app-id>.stop` → `auth_admin_keep`, not asked of the uid that started the create
  - `<app-id>.configure` (write `/etc/apsis/config.toml`) → `auth_admin_keep`
  - `<app-id>.restore` (0.5.0: prepare a restore) → `auth_admin`, asked every time; "Restart
    now" and "Cancel restore" aren't asked of the uid that prepared the plan
- Without the helper the applet does nothing but say so; there's no `pkexec` fallback.
- The restore's offline part runs from `apsis-restore.service` in `system-update.target`, as
  root, with no D-Bus: `apsis-helper --apply-restore` from a copy of the helper in the state
  folder (`apsis_core::restore::apply` is the state machine, the helper its `Runner`).
  `apsis-helper --disarm` is the ten-minute timer's.
- Input validation in the helper: snapshot names must match `YYYY-MM-DD_HH-MM-SS`; comments are
  length-limited (they go into `info.json`); configs are checked again.

## apsis-helper

All names (bus name, path, interface, methods, signals, error names, polkit action IDs, unit)
are constants in `apsis_core::helper::names`; the helper's tests check its interface and the
files in `resources/helper/` against them.

| | |
|---|---|
| bus name | `io.github.atraxsrc.Apsis.Helper` (system bus, owned by root only) |
| object / interface | `/io/github/atraxsrc/Apsis/Helper`, `io.github.atraxsrc.Apsis.Helper3` |
| `List() -> ((sssa(ssss)asas)a{st})` | `(device, uuid, mode, [(name, tags, comment, rsync_flags)], warnings, leftovers)` and the backup device's usage in bytes (`total`, `used`, `free`, all or none, `statvfs` while mounted). `rsync_flags` (0.5.0, `Helper3`) is the raw `apsis-rsync-flags` string from the snapshot's `info.json`, `""` when missing (an old-format snapshot; core's `native::info::is_old_format` reads it). polkit `list`, not interactive. A read, not a job (0.4.2): lists share one read-only mount and run at the same time; never announced |
| `Create(s comment)` | a snapshot (leftovers removed first); polkit `create`, interactive; returns once started, `Finished("create", ..)` follows |
| `Delete(s name)` | one snapshot or leftover (see Backend above), only a name the fresh list has; polkit `delete`, interactive; returns once started, `Finished("delete", ..)` follows |
| `DeleteMany(as names)` | (0.4.2) two or more distinct snapshot names, as one job: each in order, `Job`'s `snapshot` the one being deleted and `percent` done of total, stopping at the first failure; polkit `delete` once, interactive; returns once started, `Finished("delete-many", ..)` follows, its failure message encoding what was deleted, what failed and what's left (`Error::DeleteManyStopped`) |
| `Stop(s snapshot)` | stops the running create if it makes `snapshot`; no prompt for the starter's uid, else polkit `stop`; refused once the snapshot is being put in place |
| `Job() -> (sssxdx)` | `(kind, state, snapshot, started, percent, eta_seconds)`, idle `("", "", "", 0, -1, -1)`; kinds `create`, `delete`, `delete-many`, `configure`, `restore` (never `list`; a ready restore plan is a running `restore` at 100%); polkit `list`, not interactive |
| `CheckRestore(s snapshot) -> (bsbbbs)` | (0.5.0) `(ok, refusal, has_home, has_root, old_format, apsis_note)` for the Restore dialog: the first refusal of PLAN 6b.7 as a stable word (`restore::refusal::Refusal::to_wire`, `""` when ok), whether the snapshot holds `/home` and `/root`, the old format, and which Apsis it holds (`current`, `not-installed`, `no-restore:<v>`, `old-settings:<v>`). `snapshot` must be in the fresh list. polkit `list`, not interactive. A read like `List`: shares the read-only mount, never a job, `Busy` while a write runs or waits |
| `Restore(s snapshot, b restore_home, b safety_snapshot)` | (0.5.0) prepares a full-system restore (PLAN 6b.4): the 6b.7 checks again, the restore's dry run (and a second one for a separate `/home` being restored), the space checks, the safety snapshot (with `/home` when home is restored), `request.json` and `restore.filter` in `/var/lib/apsis/restore/`, the recovery note on the backup disk. polkit `restore`, `auth_admin` every time. Returns once started; a `restore` job, stoppable with `Stop(snapshot)` until ready; `Finished("restore", true, "")` means the plan is ready and the helper refuses writes until `RestartToRestore` or `CancelRestore`. A refusal is `Finished(.., false, "restore refused: <word>")` |
| `RestartToRestore(s snapshot)` | (0.5.0) "Restart now" for the ready plan (PLAN 6b.5): re-checks its age (30 min), the space on each destination, the ESP's boot files and space, both update-link names and a Pop!_OS upgrade; arms the next boot (unit, wants link, drop-in, helper copy, `state.json`, sync, `/system-update` last), starts the ten-minute disarm timer (`systemd-run`, `Conflicts=shutdown.target`, runs `apsis-helper --disarm`), ends the job `done` and asks logind `Reboot(false)`. No password for the uid that prepared the plan, else polkit `restore`. Any refusal or failure removes the plan: `InvalidInput` ("the preparation is too old" / "the preparation is gone"), `Failed` with `restore refused: <word>`, or the text |
| `CancelRestore()` | (0.5.0) "Cancel restore" at the ready prompt (PLAN 6b.5): removes the plan's files (the last `result.json` stays; a finished safety snapshot stays), the job ends `stopped`. No password for the uid that prepared the plan, else polkit `restore`. The plan also goes when the connection that prepared it leaves the bus, and a plan found at the helper's start with no `/system-update` link is a leftover and goes. No plan: `InvalidInput` ("the preparation is gone") |
| `RestoreResult() -> (sssxss)` | (0.5.0) `(state, snapshot, message, when, home, safety_snapshot)`: `ready` while a plan waits at the prompt, else the last `result.json`'s outcome (`done`, `problems`, `boot-kept`, `boot-broken`, `not-started`, `failed`), its snapshot, message and time (`""` and `0` for none), its `home` (`keep` or `restore`) and its `safety_snapshot` (`""` for none), else `""`. polkit `list`, not interactive; a read of one file, never a job |
| `ReadConfig() -> (s(sbbas)sas)` | `(config.toml text or empty, the config in effect, lsblk JSON, notes)`; notes only while converted or imported; polkit `list`, not interactive |
| `WriteConfig(s expected, (sbbas) config) -> s` | writes `/etc/apsis/config.toml` if it still reads `expected` (empty: none yet); polkit `configure`, interactive; returns once done |
| `JobChanged((sssxdx) job)` | signal to everyone: a job started, got further (at most one per 500 ms, the end always), is stopping, or ended (`done`, `failed`, `stopped`, once). No comment, caller, error text or path |
| `Finished(s op, b ok, s message)` | signal, sent only to the caller that started the operation; `message` is the error |
| errors | `...Helper3.Error.{NotAuthorized,Busy,InvalidInput,Failed,DeviceNotFound,Changed}` |

`helper::encode_error` keeps an error's kind across the bus (a missing disk, a disk removed
during the job `backup disk removed: <uuid>: <reason>`, a refusal `refused: `, `stopped`);
anything else is its text. After a failed create or delete the helper checks the device's
`/dev/disk/by-uuid` link to tell a removed disk from other failures, and a failed unmount tries
`umount --lazy`.

Each call, in order:
1. Input checked again (`validate_comment`, snapshot name pattern, the config); the applet
   isn't trusted.
2. A write (`Create`, `Delete`, `DeleteMany`, `WriteConfig`) is refused with `Busy` if another
   write runs or waits, before any password dialog. A read (`List`) is refused with `Busy`
   while a write runs or waits (writer priority); reads never refuse each other.
3. polkit `CheckAuthorization` with subject `system-bus-name` = the caller's unique bus name (not
   a PID, so a reused PID can't inherit an answer); `AllowUserInteraction` for everything but
   `list`. No answer from polkit counts as a no.
4. A write takes the one write lock; a second write gets `Busy`, never waits in a queue. Readers
   already in are waited for, up to 15 s (a list is a second or two), then the write is refused
   `Busy`. Every use of the backup mount point goes through the lock or the readers' share.
5. Tools run with a fixed argv, found on a fixed `PATH`, with a cleared environment
   (`HOME=/root`, `USER`/`LOGNAME=root`, `LC_ALL=C.UTF-8`) and stdin null.
6. The backup device from the config is mounted at `/run/apsis/backup` by UUID:
   `ro,nosuid,nodev,noexec` for lists, one mount shared by the lists running at the same time
   (the first mounts, the last out unmounts); `rw,nosuid,nodev` for a create or delete, for
   that one job, after the readers are gone. Encrypted devices and anything that isn't a Linux
   filesystem are refused.
7. Create and delete return as soon as they start. The job (kind, snapshot name, progress,
   state) is kept beside the lock for `Job` and announced with `JobChanged`. When it ends, the
   lock is released **first**, then the end is announced, then `Finished` is sent to the
   caller: the refresh either one sets off is never refused by the job it refreshes for (the
   rule the restore builds on). `Finished` waits until the end is on the bus (the announcing
   task says so; 2 s at most), so the caller, a listener too, sees its job end before it
   hears the result. The applet copes with either order anyway (`OwnEnd` in `app.rs`).
8. Every call and its result is logged to the journal (`journalctl -u apsis-helper`); comments
   are cut to 40 characters. A delete logs `delete "<name>" for :1.42: started`, the path it
   deleted, and `done` or the reason.

Long operations don't depend on any D-Bus call timeout: the only long wait inside a method
call is the password dialog, and zbus sets no call timeout by default. The applet waits for
`Finished`, or for the helper to leave the bus without sending one, which it reports as an error.

The helper exits after 60 s with no call open and nothing running. It never exits while an
operation runs or a call (including one waiting for the password dialog) is open, and it waits
until each `Finished` is sent. The next call starts it again.

## Theming

- libcosmic widgets pick up the computed COSMIC theme automatically and update live.
- Use theme tokens only (`cosmic::theme::active()` palette / container styles). No hex literals.
- Standard libcosmic widgets (buttons, list rows, dialogs, tab bar, settings items, progress
  bars); see `docs/APSIS-UI-PROMPT.md`.

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
- `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy` - the six actions

Made at run time: `/etc/apsis/config.toml` (and `.bak`), by the helper; the .deb's postrm
removes `/etc/apsis/` on purge.

**The restore's files** (0.5.0; PLAN 6b.5, 6b.6, 6b.9), all made by the helper, none packaged:

- `/var/lib/apsis/restore/` (root, 0700): `request.json` (the plan: snapshot, its creation
  time, the backup disk's UUID, the home choice, the format, the safety snapshot, the root
  UUID, the running kernel, the space each destination needs, a separate `/home`'s UUID, the
  starter's uid, when it was prepared), `restore.filter` (rsync's `--exclude-from`),
  `rsync-log`, `state.json` (attempts, written, step, problems, boots), `esp-backup/` (the
  boot partition's file set and its manifest), `result.json` (the outcome the window shows
  after login; kept until the next apply or purge), and while armed `apsis-helper` (the copy
  the unit runs). The texts are versioned (`version: 1`) and refused whole when invalid
  (`apsis_core::restore::{plan, state, file}`).
- Written on arm, removed when the restore ends or is disarmed, and by `postrm purge`:
  `/etc/systemd/system/apsis-restore.service`, its link in
  `system-update.target.wants/`, `/etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf`
  (keeps Pop!_OS's release upgrade from running in the restore's boot), and last
  `/system-update -> /var/lib/apsis/restore` (the single commit point: with it the next boot
  restores; without it nothing does).
- On the backup disk: `timeshift/apsis-restore-RECOVER.txt` (the recovery steps with the
  UUIDs filled in), written while preparing; the safety snapshot is an ordinary snapshot.
- Transient, this boot only: `apsis-disarm.timer` and `.service` (`systemd-run`,
  `Conflicts=shutdown.target`), which disarm ten minutes after "Restart now" if no restart
  followed.

The activation file and unit come from `resources/helper/*.in` with `@libexecdir@` filled in.
Afterwards `just install` runs `systemctl daemon-reload` and the bus's `ReloadConfig` (dbus-broker
doesn't watch its config directories); both are skipped when `rootdir` is set for packaging.
`just uninstall` stops `apsis-helper.service` first.
