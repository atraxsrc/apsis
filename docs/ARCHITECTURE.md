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
  device (UUID, else path) and adds `--snapshot-device` to later calls. The applet uses
  `PkexecRunner` (`pkexec --disable-internal-agent /abs/path/timeshift ...`, see DECISIONS.md).
- `HelperClient` (phase 4) - same trait over D-Bus.
- `Native` (phase 5) - btrfs / rsync.

The applet calls the backend on a background task (libcosmic `Task`) and never blocks the UI thread.

## Privilege model

- The applet runs as the user and is never setuid and never run as root (Wayland GUIs must not run as root).
- Phases 2–3: `pkexec timeshift …` — COSMIC's polkit agent shows the password prompt.
  Downside: a prompt per call. Acceptable for now.
- Phase 4: `apsis-helper` system service, D-Bus activated. Each method checks a polkit action:
  - `<app-id>.list` → `allow_active=yes`
  - `<app-id>.create`, `<app-id>.delete` → `auth_admin_keep`
  - `<app-id>.restore` (phase 6) → `auth_admin` (no caching)
- Input validation in the helper: snapshot names must match Timeshift's pattern
  (`YYYY-MM-DD_HH-MM-SS`); comments are length-limited and passed as argv, never a shell.

## Theming

- libcosmic widgets pick up the computed COSMIC theme automatically and update live.
- Use theme tokens only (`cosmic::theme::active()` palette / container styles). No hex literals.
- Monospace everywhere in the popup via libcosmic's monospace font.

## Files installed (by `just install`, run by the user)

- `/usr/bin/apsis`
- `/usr/share/applications/<app-id>.desktop` (with `X-CosmicApplet=true` as the template sets)
- `/usr/share/icons/hicolor/{scalable,symbolic}/apps/<app-id>{,-symbolic}.svg`
- `/usr/share/metainfo/<app-id>.metainfo.xml`
- phase 4: helper binary, D-Bus service + policy, polkit policy, systemd unit
