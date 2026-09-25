app-title = Apsis
app-comment = Timeshift-style system snapshots for the COSMIC™ desktop
app-keywords =

tooltip-last = Apsis - last snapshot { $ago }
tooltip-none = Apsis - no snapshots

waiting = waiting for timeshift (authenticate if asked)
not-loaded = press [r] to list snapshots
empty = no snapshots yet - press [c] to create one
no-device = no snapshot device selected in Timeshift
no-device-hint = set one up in Timeshift, then press [r]

error-label = error:
not-installed = timeshift not found - install it or wait for the native backend
failed-code = timeshift exited with code { $code }
failed-signal = timeshift was killed by a signal
failed-auth = not authorised, or the password dialog was dismissed
failed-other = { $message }
disk-missing = backup disk not connected ({ $id }): plug it in and press r

pane-snapshots = snapshots
pane-details = details
pane-activity = activity
pane-help = help
pane-about = about
pane-settings = settings
pane-settings-unsaved = settings (unsaved)

details-name = name
details-created = created
details-age = age
details-tags = tags
details-comment = comment
details-none = no snapshot selected

activity-idle = idle
activity-list-warning = list: { $warning }

help-move = move selection
help-ends = first / last snapshot
help-details = focus details, or back to list
help-create = create a snapshot, with a comment
help-delete = delete selected (confirm with y)
help-refresh = refresh the list
help-settings = Timeshift settings
help-help = show or hide this help
help-escape = cancel, go back, close the popup
help-settings-change = settings: change the selected row
help-settings-count = settings: keep one more, one fewer, type it
help-settings-filters = settings: add a filter, remove the selected one
help-settings-write = settings: write them to Timeshift (asks for your password)
help-settings-reload = settings: read them again, dropping changes

menu-refresh = Refresh
menu-settings = Settings…
menu-about = About Apsis
menu-panel-settings = Panel settings…

about-license = license
about-source = source

prompt-comment = comment:
prompt-delete = delete { $name }? [y/N]
creating = creating snapshot…
deleting = deleting { $name }…
created = snapshot created
deleted = deleted { $name }
delete-cancelled = delete cancelled
create-failed = create failed: { $reason }
delete-failed = delete failed: { $reason }

prompt-filter = add filter:
prompt-count = keep { $level }:

settings-device = device
settings-mode = mode
settings-schedule = schedule
settings-home = home
settings-filters = filters
settings-device-unset = none selected
settings-device-away = not connected ({ $id })
settings-rsync-only = rsync  (no btrfs here)
settings-btrfs-home = include @home
settings-keep = keep { $count }
settings-home-excluded = excluded
settings-home-hidden = hidden files only
settings-home-all = everything
settings-encrypted = (encrypted)
settings-add-filter = + add filter…

settings-key-path = path
settings-key-type = type
settings-key-size = size
settings-key-label = label
settings-key-uuid = uuid
settings-key-keep = keep
settings-key-user = user
settings-key-home = home
settings-key-backup = backup
settings-key-kind = kind
settings-filter-include = include
settings-filter-exclude = exclude

settings-device-note = Snapshots go to this device. Space picks the next one that can hold them.
settings-device-none = No backup device selected. Space picks one.
settings-device-missing-note = Not connected now. Timeshift keeps it selected; plug it in to use it.
settings-mode-rsync = rsync copies the system to the backup device. Space switches to btrfs, which snapshots @ and @home on the system disk.
settings-mode-rsync-only = rsync copies the system to the backup device. btrfs mode needs a btrfs filesystem, and there is none.
settings-mode-btrfs = btrfs snapshots the @ and @home subvolumes on the system disk (the backup device must be btrfs). Space switches to rsync.
settings-btrfs-home-note = Also snapshot the @home subvolume.
settings-schedule-note = Space turns { $level } snapshots on or off. Timeshift keeps the newest ones up to this count and removes older ones that have no comment.
settings-home-note = Space cycles excluded, hidden files only, everything: the same filter patterns Timeshift's Users tab writes.
settings-home-encrypted = An encrypted home uses other patterns: change it in Timeshift.
settings-filter-note = rsync uses the first filter that matches. + in front includes. x removes this one.
settings-add-note = Enter a pattern like /var/lib/libvirt/** or *.mp3. Start with "+ " to include instead of exclude.

settings-reading = reading Timeshift's settings
settings-read-failed = couldn't read Timeshift's settings:
settings-need-helper = settings need apsis-helper (sudo just install)
settings-saving = writing settings
settings-saved = settings written; Timeshift's schedule updated
settings-saved-note = settings written, but: { $note }
settings-failed = settings not written: { $reason }
settings-unchanged = nothing changed
settings-unsaved = unsaved changes: [w] writes them, Esc again drops them
settings-edit-hint = e types a count on a schedule row
settings-gui-open = Timeshift's window is open: it saves its own settings when it closes, over these
