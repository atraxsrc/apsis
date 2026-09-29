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
| 0.3.x | yes |
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
  length-limited, restore paths are validated, a config is checked like the applet checks
  it), since the applet isn't trusted;
- asks polkit for every call, with the caller's unique bus name as the subject (not a PID);
- runs one operation at a time;
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
one snapshot folder, before deleting anything:

- the name must match `YYYY-MM-DD_HH-MM-SS` (so it can't be `snapshots/` itself, `..` or a
  path), and the fresh list must have it;
- `timeshift/`, `timeshift/snapshots/` and the snapshot folder are each opened with
  `openat(O_NOFOLLOW | O_DIRECTORY)` from the backup mount: a symlink anywhere on the way is
  refused, never followed;
- the folder must contain a regular `info.json`;
- nothing may be mounted at or below it (`/proc/self/mountinfo`: a bind mount has the same
  device number, so the walk alone couldn't tell);
- the walk that deletes opens every folder with `O_NOFOLLOW` relative to the one above, removes
  symlinks as links, and stops at any folder on another filesystem;
- afterwards only that snapshot's links in `timeshift/snapshots-<tag>/` are removed.

**The config** `/etc/apsis/config.toml` (backup device UUID and rsync filters) is written only
by the helper, after polkit `configure`: a temporary file, `fsync`, `rename` over the old one,
mode 0644, owned by root, with the previous file kept as `config.toml.bak`. A write is refused
if the file changed since the applet read it, or if a newly chosen device isn't connected and a
plain, unencrypted Linux filesystem. Timeshift's `/etc/timeshift/timeshift.json` is only read,
once, to import its settings while there is no `config.toml`.

File-level restore is where root writes into places a user controls. See
[docs/PLAN.md](docs/PLAN.md) (Phase 6a) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for
how the helper avoids following symlinks, never overwrites, and drops setuid bits, device nodes
and `security.*` xattrs from files it hands to a user.

## polkit actions

Defined in `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy`. "Active" is a user at
a local, active session; everyone else (remote, inactive) gets `auth_admin` for every action.

| Action | What it allows | Active session |
|---|---|---|
| `io.github.atraxsrc.Apsis.list` | List snapshots (with the disk's usage), read the config | `yes` (no password) |
| `io.github.atraxsrc.Apsis.create` | Create a snapshot | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.delete` | Delete a snapshot (with the checks above) | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.configure` | Write `/etc/apsis/config.toml` | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.browse` | Browse a snapshot's files, restore dry runs | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.restore` | Copy files from a snapshot into `~/Apsis-restored/` | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.restore-original` | Put files back over the live system | `auth_admin` (every time) |

`auth_admin_keep` means an administrator's password, remembered by polkit for a few minutes.
An administrator can tighten any of these with a polkit rule in `/etc/polkit-1/rules.d/`.
