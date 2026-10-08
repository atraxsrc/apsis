# Changelog

All notable changes to Apsis are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.6.3] - 2026-10-08

### Security

- **A copied disk label can't stand in for the backup disk.** Apsis finds the backup disk
  by its filesystem UUID, which anyone can copy onto a USB stick. When two devices carry
  it, Apsis now uses neither: lists, snapshots, deletes and restores refuse and name both
  devices. A restore waiting for its restart ends "didn't start" with nothing changed.

## [0.6.2] - 2026-10-08

### Fixed

- **Snapshot progress only goes forward.** The percent no longer jumps back and forth,
  and a snapshot no longer ends at a few percent. rsync now lists every file before it
  copies; until then the line says "scanning files". The percent keeps moving through
  large unchanged parts of the system too, instead of standing still and then jumping.
  The time left shows only once it means something.

### Changed

- **The popup is simpler.** One status line ("Last snapshot 1m ago", "Creating snapshot
  · 42%") over the same bar the window has: the backup disk's use, or the snapshot's
  progress while one runs. Refresh moved to the right-click menu only.

## [0.6.1] - 2026-10-07

### Changed

- **The panel popup is a ring.** The ring shows how full the backup disk is, with the
  age of the last snapshot in its centre. While a snapshot or a restore's preparation
  runs, the ring shows its progress. The exact disk figures are in the ring's tooltip.
- The age turns the warning colour when a snapshot is overdue, and the ring when the
  disk is nearly full, so the two are told apart at a glance.
- The popup no longer lists snapshots or shows "Snapshot created"-style messages; the
  window has both. Errors still show until the next action.
- Nothing in the popup wraps or clips with a larger or monospace system font.

## [0.6.0] - 2026-10-06

Restore now works on an encrypted Pop!_OS install: an LVM volume inside one LUKS
partition, the layout Pop!_OS's installer makes with "Encrypt drive". Restore stays
experimental, for Pop!_OS 24.04 with systemd-boot.

### Added

- **Restore on an encrypted system disk.** The disk's passphrase is asked twice: at the
  restart that runs the restore, and at the start after it. As before, only a snapshot
  of this installation can be restored, and the live `/etc/fstab` and `/etc/crypttab`
  are kept.
- **A check that the new boot files can still unlock the disk.** If they can't, the
  computer keeps the boot files it had and starts the kernel from before the restore.
- **The recovery note unlocks the disk first.** On an encrypted system,
  `timeshift/apsis-restore-RECOVER.txt` on the backup disk starts with the lines that
  unlock it, with this computer's details filled in.

### Changed

- A system disk set up another way is refused with a clearer message: restore works on
  a plain partition and on Pop!_OS's standard encrypted install, and snapshots still
  work either way.
- The Restore dialog's note about snapshots holding an older Apsis no longer names a
  version.

### Known limits

- Encrypted restore works in the "Encrypt drive" layout only. LUKS without LVM, and LVM
  without LUKS, are refused before anything happens.
- A snapshot made while Apsis 0.5.x was installed puts 0.5.x back. On an encrypted
  system, install 0.6.0 again after such a restore.
- The backup disk itself can't be encrypted; the system disk can.
- Restore is new: a report of how it went on your hardware, good or bad, helps.

## [0.5.0] - 2026-10-04

Restoring the whole system to a snapshot, at the next start. Experimental; Pop!_OS 24.04
with systemd-boot only.

### Added

- **Restore** in the window's toolbar: Apsis checks the snapshot fits this computer (the
  same installation, UEFI, Pop!_OS with systemd-boot, a plain ext4 system disk, the
  snapshot's kernel files, no update waiting, the same `crypttab`), offers to keep or
  restore the home folders and to take a safety snapshot first (on by default), prepares
  (the space is measured with dry runs, the safety snapshot taken, the plan written), and
  asks once more at "Ready to restore". "Restart now" arms the next start and restarts;
  "Cancel restore", Esc or closing the window drops the plan and keeps the safety snapshot.
  While a restore waits for its restart, a second one is refused and leaves the first as
  it is, and no snapshot can be deleted.
- The restore itself runs at the next start with the desktop stopped (systemd's
  offline-update mode, with plymouth), keeps the live `fstab` and `crypttab`, the boot and
  recovery partitions, other mounts and the kernel the computer started with, refreshes the
  boot files with kernelstub as Pop!_OS's own hooks do, checks them byte for byte, and
  keeps the old ones if the refresh or the check fails. A copy that breaks is tried again at
  the next start, three times at most. After login, the window's status line says how it
  went.
- A recovery note with this machine's UUIDs on the backup disk
  (`timeshift/apsis-restore-RECOVER.txt`), and "If a restore goes wrong" in the README.
- Each snapshot's row says in its tooltip when it was made in the older format (without
  ACLs and extended attributes), and the restore dialog says what that means.
- `/root` is restored with the system only when the snapshot has it with content; otherwise
  it's kept as it is.

### Changed

- `apsis-helper`'s D-Bus interface is `io.github.atraxsrc.Apsis.Helper3`: `List` carries
  each snapshot's format, and `CheckRestore`, `Restore`, `RestartToRestore`,
  `CancelRestore` and `RestoreResult` are new. A panel applet still running from 0.4.x shows
  an error until it's re-added (log out and in).
- A sixth polkit action, `io.github.atraxsrc.Apsis.restore` (`auth_admin`, asked every
  time).
- `apsis-helper --apply-restore` and `--disarm` are the offline entry points; the .deb's
  purge removes the restore's state folder and anything an unfinished restore left.
- While a restore plan waits at its prompt, creating, deleting and saving the settings are
  refused as busy in every window, and lists go on.
- Removing the package (`apt remove`) now also removes what an armed restore wrote (Apsis's
  own `/system-update` link, the restore unit, its wants link and the drop-in) and stops the
  disarm timer; the config and `/var/lib/apsis` still go only on purge.

### Fixed

- Upgrading, reinstalling or removing the package no longer stops a running job part-way
  (a reinstall during a delete had left a half-removed snapshot). The helper holds a lock on
  `/run/apsis/job.lock` while a job runs, and the package's `prerm` refuses while it's held:
  apt stops with `apsis: an Apsis job is running; try again when it has finished`, the
  package stays as it was and the job goes on. This takes effect once 0.5.0 is the installed
  version: the upgrade from 0.4.x still runs the old script.
- A package operation while a restore waits for its restart cancels the restore first
  (`apsis: the restore that was waiting for a restart is cancelled`). Before, a remove left
  the restore armed with nothing left to disarm it: the next restart, on any later day,
  would have restored. If the restore can't be cancelled, apt stops and prints the two
  commands to run by hand.
- In the seconds between "Restart now" and the next start being armed, a second restore, a
  delete, a snapshot or a settings save is now refused as busy; before, it could get in
  between and take the plan's files or a snapshot away. The same holds while a cancelled
  plan's files are removed.
- A delete cut off part-way (power loss, or the helper stopped during a reinstall) no longer
  leaves a half-removed snapshot that Apsis could only warn about: a delete first moves the
  folder into `timeshift/apsis-staging/`, so what's left is a dimmed row that **Delete**
  removes. A folder an earlier version left half-deleted in `timeshift/snapshots/` (an
  `info.json`, no `exclude.list`) is shown as such a row too. These rows now read "Unfinished
  snapshot or delete · Delete removes it".
- An error the helper reports now reaches the window's status line in its own words, without
  `apsis-helper:` in front, for example `Restore failed: No space left on device (os error
  28)`. 0.4.2 took it off only for errors made in the window; the helper's journal keeps it.
- Settings' **Cancel** is always clickable and leaves the page, dropping any change. Before,
  it was greyed out until something changed, so with nothing changed it did nothing.

## [0.4.2] - 2026-10-01

Deleting several snapshots works again with more than one display, and lists never get in
each other's way.

### Fixed

- A window no longer reports its own failed operation as another window's: the helper's end
  announcement could reach the window after its own result and was shown as "A delete
  started elsewhere failed" over the real line. The helper now puts the end on the bus
  before the result, and the window knows its own job's end in either order.
- The helper's reasons are shown as it said them: the status line, the tooltip and the
  dialogs no longer carry an `apsis-helper:` prefix (the journal keeps it).
- Deleting several snapshots at once: the second one was refused as busy on a machine with
  two monitors (each display runs its own panel applet, and every applet listed the moment
  the first delete ended, holding the helper's lock). Now the window sends the whole
  selection as one job (`DeleteMany` on the helper's interface), the password is asked once,
  and it stops at the first failure saying which were deleted and which were not.
- Lists share: the helper mounts the backup disk read-only once for all the lists running at
  the same time, and a list is no longer a job (it isn't announced, and never shows as
  "busy" to another list). A snapshot or delete waits up to 15 s for lists to finish instead
  of being refused; a list that arrives while one runs or waits is refused as before.
- The helper releases its lock before it announces a job's end, so the refresh that follows
  is never refused.
- Each Apsis process (one per display, plus the window) keeps one connection to the helper
  and lists once per snapshot or delete, so the journal shows one `list` per process after a
  job, none refused.
- The applet's desktop entry no longer passes `%F` to `apsis` (it takes no files; the panel
  passed it literally and it was ignored).

### Changed

- `apsis-helper`: `DeleteMany(as names)` added to `io.github.atraxsrc.Apsis.Helper2`; `Job()`
  and `JobChanged` can say `delete-many` and no longer say `list`.

## [0.4.1] - 2026-10-01

Snapshots now hold everything a full restore (0.5.0) needs.

### Changed

- Snapshots keep POSIX ACLs and extended attributes, including file capabilities (for example
  on `ping`). rsync gets `-A -X --numeric-ids`. Hard links between files (`-H`) are still not
  kept, as in Timeshift.
- `info.json` gets one more entry at the end, `"apsis-rsync-flags"`. Timeshift ignores it.
  Snapshots without it (Timeshift's, and Apsis 0.4.0 and older) keep working; they are the
  older format, made without ACLs and extended attributes.
- The first snapshot after the upgrade copies again the few files that have ACLs or extended
  attributes, instead of linking them to the previous snapshot. Later snapshots link as before.

## [0.4.0] - 2026-09-30

Apsis does four things: snapshot the system (with `/root` and `/home` as choices), a filter
list, delete snapshots, and (in 0.5.0) restore the whole system. The window is now laid out
like Timeshift's, in standard COSMIC widgets.

### Upgrading

- Open the settings once and click **Save**. Settings from 0.3 (version 1 of
  `/etc/apsis/config.toml`) are shown converted: a home folder set to "everything" for every
  user becomes the `/home` choice, "hidden files only" becomes a `+ <home>/.**` filter, and
  plain excludes get a `-`. The converted settings take the same files. One exception: a `+`
  filter that a later filter used to hide now works, because the folders above a kept path are
  let in.
- Going back to 0.3: `sudo cp /etc/apsis/config.toml.bak /etc/apsis/config.toml` (0.3 can't read
  version 2).
- File restore is gone. Anything in `~/Apsis-restored/`, and any `*.apsis-before-*` file next to
  an original, is yours; delete it by hand when you don't need it.
- The helper's D-Bus interface is now `io.github.atraxsrc.Apsis.Helper2`. After upgrading, log
  out and in (or re-add the applet) so the panel runs the new version.

### Added

- A window like Timeshift's: a toolbar (Create, Delete, Settings), the snapshot list (date and
  comment; Ctrl-click and Shift-click select several), and a status area with the last
  snapshot, the backup disk and its bar, and a running snapshot's progress.
- Settings as tabs: **Location** (the backup disk), **Include** (`/root`, on by default; `/home`,
  off by default), **Filters** (`+`/`-` list with Add Folder, Add File, Add Pattern, Remove,
  Move Up, Move Down; new lines go on top), **Misc** (reminder, panel label).
- **Stop** a snapshot while it's being made: rsync's process group is ended and what it copied
  removed. The user who started it isn't asked for a password (new polkit action `stop` for
  anyone else).
- Every window and the panel popup show a snapshot being made or deleted elsewhere, and list
  again when it ends.
- What an interrupted snapshot leaves behind is shown in the list, removed by the next
  snapshot, and can be deleted.
- A backup disk pulled out during a snapshot or delete is reported as such by the helper.
- Keyboard shortcuts: Ctrl+N, Delete, Ctrl+R / F5, Ctrl+,, Ctrl+A, arrows, Esc (man page and
  README).

### Changed

- No terminal look: no prompt line, no key-hint footer, no rooms or dock.
- The panel popup uses standard widgets: last snapshot, backup disk, a running job, the newest
  snapshots, Open Apsis and Refresh.
- A failed create's staging folder is removed with the delete's safe walk.
- rsync gets `--delete-excluded` once (Timeshift passes it twice).
- Summary: "System snapshot and restore".

### Removed

- Restoring single files (the browser, both restore modes, `Apsis-restored`, `.apsis-before`
  backups) and the polkit actions `browse`, `restore` and `restore-original`.
- Keep last N snapshots, and its preview.
- Per-user home folder modes (excluded / hidden files only / everything).

## [0.3.1] - 2026-09-30

Fixes for a fresh install.

### Fixed

- One window: "open apsis" in the panel and the app launcher bring the open window forward
  instead of opening another (libcosmic's single-instance support; the panel asks the
  compositor for an activation token first). `--settings` and `--about` switch the open window
  to that view when nothing is in progress there.
- A window opened while a snapshot job runs (started by another window, or by one that was
  closed) no longer looks idle: the activity pane says `busy · a job is running in the
  background`, and Apsis lists again every few seconds until the job is over, then goes back to
  idle.
- A create that fails because the backup disk was unplugged says `create failed · backup disk
  removed`; rsync's raw error ("Input/output error (os error 5)") goes to the log room only.
- The helper's systemd unit no longer says it runs Timeshift: `Apsis snapshot helper`.

### Added

- The create room shows whether your home folder goes into the snapshot: `home  included`,
  `home  hidden files only` or `home  excluded` (muted; changed in settings).
- The file browser says `home not included in this snapshot` for an empty home folder, instead
  of `empty folder`.
- After a folder-mode restore, the activity pane says where the files went (`restored to
  ~/Apsis-restored/<snapshot>/...`), and `o` (or `[o]pen folder`) opens that folder.
- The backup disk is watched without `r`: every 5 seconds and when the popup or window opens,
  Apsis looks for it in `/dev/disk/by-uuid` (no root, no mount). When it's gone, the strip and
  the tooltip say `disk  not connected` and creating waits; when it's back, Apsis lists again.

## [0.3.0] - 2026-09-29

A status you can read at a glance, and a window that does the work. The panel popup is now a
read-only overview; creating, deleting, settings and restore are in the window.

### Added

- A status model behind the panel: the tooltip shows `last`, `next` (manual only, for now) and
  `disk`; the icon takes the warning colour when a snapshot is overdue or the disk is low, and
  the destructive colour when the disk is nearly full.
- An optional panel label beside the icon: the age of the newest snapshot and the share of the
  backup disk in use (horizontal panels only). Off by default; a setting.
- The `apsis` strip in the window: last and next on the left, the backup disk and its bar on the
  right.
- Disk warning colours: the bar turns to the theme's warning and destructive colours as free
  space runs low.
- The activity pane shows progress and the time left while a snapshot is created or restored.
- A four-room window: snapshots, create (beside the list), schedule (keep and remind) and log,
  switched with `1`-`4` or by clicking the dock.

### Changed

- The panel popup is a read-only overview: the strip, the newest snapshots, `open apsis`,
  refresh and close. Nothing is created, deleted or restored from it.
- Theme: every colour comes from the COSMIC theme (accent, divider, background, warning,
  destructive); the only derived colours are faded versions of those.
- A leftover from an interrupted snapshot is reported as one plain line ("leftover from an
  interrupted snapshot, safe to delete") instead of the raw staging path and timestamp; the path
  goes to the journal.
- New screenshots in the README and the metainfo: the window, the create and schedule rooms,
  and the panel popup.

### Known issues

- With a theme whose background is translucent, the accent border of the active pane tints the
  whole pane. Changing the theme avoids it; a proper fix waits for libcosmic.

## [0.2.0] - 2026-09-29

Apsis is standalone now: it takes rsync snapshots itself and no longer needs or runs
Timeshift. Snapshots keep Timeshift's on-disk layout, so the ones Timeshift made keep working
(listed, browsed, restored from, deleted), and an installed Timeshift still reads Apsis's.
Manual only: there is no schedule, and a snapshot is only deleted when you delete it.

**Upgrading:** open the settings (`s`) once. They show what was taken from Timeshift's
settings (backup disk, home folders, filters); press `w` to save them as Apsis's own
(`/etc/apsis/config.toml`). Your snapshots stay where they are.

### Added

- Apsis's own settings file, `/etc/apsis/config.toml` (backup disk and filters), written only
  by `apsis-helper`, atomically, with a `.bak` of the previous one. The first time, it's
  imported from Timeshift's `/etc/timeshift/timeshift.json`, which is only read.
- Keep the last N snapshots (optional): `p` previews which older snapshots would go, and
  deletes them only after `y`; offered after each create. Commented snapshots always stay and
  don't count.
- Reminder: the panel icon turns the warning colour, and the tooltip says so, when the last
  snapshot is older than 7 days (configurable, or off).
- A disk usage line under the panes: the backup disk's used and free space as a bar.
- Progress with the time left for creates and restores.
- rsync runs at idle I/O priority and nice 19, so the desktop stays responsive.
- `just deb-install` builds the .deb and installs it.

### Changed

- Creating, listing and deleting snapshots is done by Apsis (rsync), no longer by Timeshift.
  Deleting as root refuses anything that isn't plainly one snapshot folder: a snapshot name,
  no symlink on the way, an `info.json`, nothing mounted inside; it never follows a symlink or
  leaves the filesystem, and removes only that snapshot's tag links.
- The settings view has the backup disk, home folders and filters, plus Apsis's own keep and
  remind.
- `apsis-helper` is required (the .deb installs it); there's no `pkexec` fallback any more.
- The .deb no longer recommends `timeshift`; `apt purge` also removes `/etc/apsis`.

### Removed

- Timeshift as the backend, and everything that ran `timeshift`.
- Timeshift's settings in Apsis: schedules and retention counts, rsync/btrfs mode, `@home`.
  Apsis doesn't schedule or delete snapshots by itself.
- The native backend's dry-run setting (the native backend is the only one now).

### Not supported (yet)

- Encrypted (LUKS) backup disks: Timeshift unlocks them itself; Apsis doesn't.
- btrfs snapshots and full-system restore.

## [0.1.2] - 2026-09-27

### Added

- Bulk delete in the snapshot list, like Timeshift: `space` marks or unmarks a snapshot, `J`
  marks and moves down. Marked rows get an accent `*` and a faint accent background, and the
  title shows `· N marked`. `d` asks once, listing them; they're deleted one by one through
  `apsis-helper` with a single password prompt, with progress (`deleting 2/4: …`) in the
  activity pane. It stops at the first failure and says which were deleted and which weren't.
  Esc clears the marks.
- Right-click menu: **Close** (closes the menu, like Esc).
- README: "Settings explained": device, rsync or btrfs mode, schedules and "keep N", home
  folders (excluded / hidden files only / everything), filters with examples, and the Apsis
  backend and dry run settings.
- Manual page `apsis(1)` (`man apsis`), installed by `sudo just install` and the .deb.
- Settings view: home rows explain the three options.

### Changed

- Delete prompts, progress and results name a snapshot by its date and comment
  (`09-27 09:12 "bulk 1"`) instead of the raw name; the details pane and the helper's journal
  keep the full name.
- Right-click menu: "Panel settings…" is now "Remove or move applet…" (it still opens COSMIC
  Settings on the panel page).

### Fixed

- Settings view: the explanations in the details pane are no longer cut off. In the popup the
  pane is sized to the longest one and keeps its height while moving between rows; the notes
  fit the smallest window.
- Timeshift's `Ret=NNN` lines are recognised as warnings like `E:`/`W:`, instead of being
  ignored or reported as `not a snapshot row`.

## [0.1.1] - 2026-09-26

### Added

- `.deb` package for Pop!_OS / Ubuntu 24.04 or newer (amd64): `just deb` builds it with
  cargo-deb, with the same files as `sudo just install`. Pushing a `v*` tag builds it in CI and
  attaches it to the GitHub release.
- README: a "How to use" section (install, first run, everyday use, restoring files, settings,
  troubleshooting).

### Fixed

- `just vendor` now writes `vendor.tar` (it used to delete its output), with versioned vendor
  folders so `atspi-common` builds, and `just build-vendored` builds offline (`--frozen`) and
  cleans up the unpacked sources afterwards.

## [0.1.0] - 2026-09-26

First release.

### Added

- COSMIC panel applet with a terminal-style popup that follows the live COSMIC theme: snapshot
  list, details pane, activity pane, `>` input line and key hints. Keyboard (vim-style keys) and
  mouse.
- List Timeshift snapshots (`timeshift --list`), with tags, comments, and the age of the last
  snapshot in the panel button's tooltip.
- Create snapshots with a comment, and delete them after a `[y/N]` confirmation.
- Settings view for Timeshift: backup device, rsync or btrfs mode, `@home`, schedules and their
  counts, home folders per user, and filters, written to `/etc/timeshift/timeshift.json` with a
  backup of the old file.
- Window mode: `apsis --window`, and an app launcher entry that opens it; resizable, opens
  floating.
- Right-click menu on the panel button (refresh, settings, about, panel settings).
- `apsis-helper`: a root D-Bus service, started on demand and guarded by polkit, so creating or
  deleting asks for a password once per few minutes instead of every time. Falls back to
  `pkexec` when it isn't installed. See `SECURITY.md`.
- **Experimental:** native rsync backend. Creates Timeshift-compatible rsync snapshots without
  running `timeshift`, with a dry run (on by default) that shows the whole plan. Opt-in from
  the settings view.
- **Experimental:** file-level restore. Browse a snapshot's files with a comparison to the
  running system, mark entries (space marks in place, `J` marks and moves down), and restore
  them to `~/Apsis-restored/<snapshot>/` or back to their original place, after a dry run.
  Original mode keeps each replaced file as `<name>.apsis-before-<snapshot>` and asks for the
  password every time.
- App and symbolic icons, AppStream metainfo, desktop entries, `just install` / `just uninstall`.

[0.6.3]: https://github.com/atraxsrc/apsis/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/atraxsrc/apsis/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/atraxsrc/apsis/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/atraxsrc/apsis/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/atraxsrc/apsis/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/atraxsrc/apsis/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/atraxsrc/apsis/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/atraxsrc/apsis/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/atraxsrc/apsis/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/atraxsrc/apsis/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/atraxsrc/apsis/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/atraxsrc/apsis/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/atraxsrc/apsis/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/atraxsrc/apsis/releases/tag/v0.1.0
