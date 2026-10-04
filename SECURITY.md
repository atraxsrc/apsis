# Security

Apsis installs a helper that runs as root, so security reports are welcome and taken seriously.

## Reporting a vulnerability

Please report privately, not in a public issue:

1. Open <https://github.com/atraxsrc/apsis/security/advisories/new> (the repository's
   **Security** tab → **Report a vulnerability**). This uses GitHub's private vulnerability
   reporting; only you and the maintainer see the report.
2. Say what you found, the version or commit, and how to reproduce it. A proof of concept that
   stays on your own machine is fine; please don't test against systems that aren't yours.

I'll reply as soon as I can. Once a fix is released, the advisory is published with
credit to you, unless you'd rather not be named.

## Supported versions

| Version | Supported |
|---|---|
| 0.4.x | yes |
| 0.3.x | no |
| 0.2.x | no |
| 0.1.x and older | no |

## What runs as root

The applet (`apsis`) always runs as your user. It is never setuid and never run as root.

**`apsis-helper`** (`/usr/libexec/apsis-helper`) is the only part that runs as root. It is a
D-Bus service on the system bus (`io.github.atraxsrc.Apsis.Helper`), started on demand by the
bus through systemd (`apsis-helper.service`, no `[Install]`, so nothing enables it at boot) and
exits after 60 seconds idle. The .deb and `sudo just install` install it; the applet does
nothing without it. Apsis doesn't run `timeshift`.

The helper:

- checks every input again (snapshot names must match `YYYY-MM-DD_HH-MM-SS`, comments are
  length-limited, a config is checked like the applet checks it), since the applet isn't
  trusted;
- asks polkit for every call, with the caller's unique bus name as the subject (not a PID);
- runs one operation at a time, and holds a lock on `/run/apsis/job.lock` meanwhile (an
  exclusive `flock`; the file is root's, 0600, opened `O_NOFOLLOW` and `O_CLOEXEC`, so a
  symlink at its name is refused and no program the helper starts inherits it). The kernel
  drops the lock with the process; the file's being there means nothing. The file must be
  root's and 0600 because `flock` works on a read-only descriptor: a lock file others could
  open would let any local user hold the lock and block every job and every package
  operation until a restart. So the `prerm` sets `umask 077` before it makes the file, and
  the helper refuses a file that isn't root's and sets any other mode to 0600 before it
  asks for the lock;
- runs `rsync`, `mount`, `umount`, `lsblk` and `findmnt` with a fixed argv (no shell), a fixed
  `PATH`, a cleared environment and stdin null; a snapshot's rsync runs at idle I/O priority
  and nice 19;
- mounts the backup device by UUID at `/run/apsis/backup` only for the length of one call
  (`nosuid,nodev`; read-only, and `noexec`, except while a snapshot is created or deleted);
  encrypted (LUKS) devices, and anything that isn't a plain Linux filesystem, are refused;
- logs every call and its result to the journal (`journalctl -u apsis-helper`).

It is not sandboxed by systemd: a snapshot reads the whole filesystem and the helper mounts
devices, which hardening options would break.

**Deleting a snapshot** is a recursive delete as root, so it refuses anything that isn't plainly
one snapshot folder, before deleting anything (what a stopped or interrupted snapshot or
delete leaves under `timeshift/apsis-staging/` is removed by the same rules):

- the name must match `YYYY-MM-DD_HH-MM-SS` (so it can't be `snapshots/` itself, `..` or a
  path), and the fresh list must have it;
- `timeshift/`, `timeshift/snapshots/` and the snapshot folder are each opened with
  `openat(O_NOFOLLOW | O_DIRECTORY)` from the backup mount: a symlink anywhere on the way is
  refused, never followed;
- the folder must contain a regular `info.json`;
- nothing may be mounted at or below it (`/proc/self/mountinfo`: a bind mount has the same
  device number, so the walk alone couldn't tell);
- the folder is then moved into `timeshift/apsis-staging/` (opened with `O_NOFOLLOW`, so a
  symlink there is refused) with `RENAME_NOREPLACE`, before anything is removed: a name already
  there, or a filesystem without that flag, refuses the delete with nothing moved, and there
  is no plain rename to fall back to. A delete cut off after the move leaves a leftover there,
  never a half-removed snapshot in `timeshift/snapshots/`;
- then only that snapshot's links in `timeshift/snapshots-<tag>/` are removed;
- then the walk that deletes opens every folder with `O_NOFOLLOW` relative to the one above,
  removes symlinks as links, and stops at any folder on another filesystem.

**The config** `/etc/apsis/config.toml` (backup device UUID, `/root` and `/home` includes, and
rsync filters) is written only
by the helper, after polkit `configure`: a temporary file, `fsync`, `rename` over the old one,
mode 0644, owned by root, with the previous file kept as `config.toml.bak`. A write is refused
if the file changed since the applet read it, or if a newly chosen device isn't connected and a
plain, unencrypted Linux filesystem. Timeshift's `/etc/timeshift/timeshift.json` is only read,
once, to import its settings while there is no `config.toml`.

**The .deb's maintainer scripts** run as root, from dpkg. Before a remove, an upgrade or a
deconfigure, the `prerm` takes the job lock without waiting and, while a job holds it, fails
with one line (exit 75), so a package operation never stops a snapshot, a delete or a
restore's preparation part-way; dpkg then leaves the package as it was. It keeps the lock
until it exits, so no job begins before the helper is stopped. The helper takes the same
lock from "Restart now" until the next start is armed, and while a cancelled plan's files are
removed. If a restore is armed, the `prerm` runs `apsis-helper --disarm`, only when
`/system-update` is Apsis's own link (it points at `/var/lib/apsis/restore`), and looks at
the link again afterwards, whatever the helper's exit status: still Apsis's, the script
fails and prints the two commands to run by hand; gone, it goes on. Another tool's link is
never touched and the helper isn't run for it. `postrm`, on remove and purge, stops the
disarm timer and removes Apsis's link (the same check), the restore unit, its wants link and
the drop-in; only purge removes `/etc/apsis` and `/var/lib/apsis`. Each script names its
paths once, and the tests run copies of the scripts against a temp folder, with a fake
`systemctl`, `busctl` and helper. Knowingly not covered: the upgrade from 0.4.2 runs
0.4.2's `prerm`, which doesn't wait; a job begun after the `prerm` has exited, while an
upgrade unpacks, runs the old binary to its end.

**Stopping a snapshot** sends `SIGTERM` to the rsync process group the helper started (only
that group, and only while its leader is known not to be reaped, so a reused pid is never
signalled), then `SIGKILL` after 10 seconds; the unfinished copy is removed as above. The user
who started a snapshot may stop it without a password; anyone else needs polkit `stop`. The
rename that puts a finished snapshot in place can't be interrupted by a stop.

**Job status** (`Job`, and the `JobChanged` signal every process on the system bus can
receive) carries the kind of job, its state, the snapshot's name (a time), when it started and
its progress: no comment, caller, error text or path. The error text goes only to the caller
that started the job.

## polkit actions

Defined in `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy`. "Active" is a user at
a local, active session; everyone else (remote, inactive) gets `auth_admin` for every action.

| Action | What it allows | Active session |
|---|---|---|
| `io.github.atraxsrc.Apsis.list` | List snapshots (with the disk's usage), read the config, ask what the helper is doing | `yes` (no password) |
| `io.github.atraxsrc.Apsis.create` | Create a snapshot | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.delete` | Delete a snapshot or an unfinished copy (with the checks above) | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.stop` | Stop a snapshot another user started (the starter isn't asked) | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.configure` | Write `/etc/apsis/config.toml` | `auth_admin_keep` |

`auth_admin_keep` means an administrator's password, remembered by polkit for a few minutes.
An administrator can tighten any of these with a polkit rule in `/etc/polkit-1/rules.d/`.
