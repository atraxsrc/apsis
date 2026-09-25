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
| 0.1.x | yes |
| older | no |

## What runs as root

The applet (`apsis`) always runs as your user. It is never setuid and never run as root.

**`apsis-helper`** (`/usr/libexec/apsis-helper`) is the only part that runs as root. It is a
D-Bus service on the system bus (`io.github.atraxsrc.Apsis.Helper`), started on demand by the
bus through systemd (`apsis-helper.service`, no `[Install]`, so nothing enables it at boot) and
exits after 60 seconds idle. `sudo just install` installs it; without it, the applet falls back
to `pkexec timeshift ...`, which asks for a password on every call.

The helper:

- checks every input again (snapshot names must match `YYYY-MM-DD_HH-MM-SS`, comments are
  length-limited, restore paths are validated), since the applet isn't trusted;
- asks polkit for every call, with the caller's unique bus name as the subject (not a PID);
- runs one operation at a time;
- runs `timeshift`, `rsync`, `mount` and `lsblk` with a fixed argv (no shell), a fixed `PATH`,
  a cleared environment and stdin null;
- mounts the backup device at `/run/apsis/backup` only for the length of one call
  (`nosuid,nodev`, read-only unless a native snapshot is being created, and `noexec` too when
  read-only);
- logs every call and its result to the journal (`journalctl -u apsis-helper`).

It is not sandboxed by systemd: Timeshift and the native backend read and write the whole
filesystem and mount devices, which hardening options would break.

File-level restore is where root writes into places a user controls. See
[docs/PLAN.md](docs/PLAN.md) (Phase 6a) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for
how the helper avoids following symlinks, never overwrites, and drops setuid bits, device nodes
and `security.*` xattrs from files it hands to a user.

## polkit actions

Defined in `/usr/share/polkit-1/actions/io.github.atraxsrc.Apsis.policy`. "Active" is a user at
a local, active session; everyone else (remote, inactive) gets `auth_admin` for every action.

| Action | What it allows | Active session |
|---|---|---|
| `io.github.atraxsrc.Apsis.list` | List snapshots, read Timeshift's settings, native list and dry run | `yes` (no password) |
| `io.github.atraxsrc.Apsis.create` | Create a snapshot (Timeshift or native) | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.delete` | Delete a snapshot | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.configure` | Write `/etc/timeshift/timeshift.json` | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.browse` | Browse a snapshot's files, restore dry runs | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.restore` | Copy files from a snapshot into `~/Apsis-restored/` | `auth_admin_keep` |
| `io.github.atraxsrc.Apsis.restore-original` | Put files back over the live system | `auth_admin` (every time) |

`auth_admin_keep` means an administrator's password, remembered by polkit for a few minutes.
An administrator can tighten any of these with a polkit rule in `/etc/polkit-1/rules.d/`.
