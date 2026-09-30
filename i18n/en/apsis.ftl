app-title = Apsis
app-comment = System snapshot and restore
app-keywords =

# Panel tooltip
tooltip-last = last  { $ago }
tooltip-last-stale = last  { $ago }  (over { $days } days)
tooltip-last-none = last  none yet
tooltip-last-none-stale = last  none yet  (over { $days } days)
tooltip-next-manual = next  manual only
tooltip-disk = disk  { $used } / { $total }  { $free } free
tooltip-disk-low = disk  { $free } free  (under { $pct }%)
tooltip-disk-not-connected = disk  not connected
tooltip-disk-unknown = disk  unknown

# Right-click menu on the panel button
menu-open = Open Apsis
menu-refresh = Refresh
menu-settings = Settings…
menu-about = About Apsis
menu-remove-or-move = Remove or move applet…
menu-close = Close

# Status: last snapshot and backup disk (window status area, popup)
last-snapshot = Last snapshot
backup-disk = Backup disk
strip-last-none = none yet
strip-over = (over { $days } days)
strip-under = (under { $pct }%)
strip-used-free = { $pct }% used · { $free } free
strip-disk-not-connected = not connected
strip-disk-unknown = unknown
strip-next-manual = manual only

# Popup
open-apsis = Open Apsis
refresh = Refresh
overview-more = { $count } older

# Toolbar and list
create = Create
delete = Delete
settings = Settings
column-snapshot = Snapshot
column-comment = Comment
leftover-row = Interrupted snapshot, removed by the next one
leftover-label = interrupted snapshot from { $when }
loading = Reading snapshots…
empty = No snapshots yet. Click Create to make one.
no-device = No backup disk chosen.
choose-disk = Choose one in Settings
retry = Try again
need-helper = Apsis needs apsis-helper, which the .deb installs.
failed-auth = Not allowed, or the password dialog was closed.
busy-background = Busy: another snapshot job is running.
disk-missing = Backup disk not connected ({ $id }). Plug it in.
disk-removed = Backup disk removed

# Jobs
progress-creating = Creating snapshot
progress-percent = { $label } · { $percent }
progress-percent-left = { $label } · { $percent } · { $time } left
progress-working = { $label } · { $elapsed }
deleting = Deleting { $name }…
deleting-many = Deleting { $step } of { $count }: { $name }…
stop = Stop
stopping = Stopping…
keep-going = Keep Going

# Results
created = Snapshot created
create-stopped = Create stopped
create-failed = Create failed: { $reason }
create-failed-disk-removed = Create failed: backup disk removed
create-failed-elsewhere = A snapshot started elsewhere failed
deleted = Deleted { $name }
deleted-many = Deleted { $count } snapshots
deleted-elsewhere = A snapshot was deleted elsewhere
delete-failed = Delete failed: { $reason }
delete-failed-disk-removed = Delete failed: backup disk removed
delete-failed-elsewhere = A delete started elsewhere failed
delete-many-stopped = Delete stopped at { $name }: { $reason }
delete-many-deleted = Deleted ({ $count }): { $names }
delete-many-kept = Not deleted ({ $count }): { $names }
delete-none = none
stop-failed = Couldn't stop: { $reason }

# Dialogs
cancel = Cancel
create-title = Create snapshot
comment = Comment (optional)
comment-placeholder = What changed, or why
delete-title-one = Delete this snapshot?
delete-title-many = Delete { $count } snapshots?
delete-body = This can't be undone.
stop-title = Stop the snapshot?
stop-body = What was copied so far is deleted.
add = Add
pattern = Pattern
pattern-placeholder = /var/lib/libvirt/***
pattern-help = Start with "+ " to include it. Anything else is excluded.
unsaved-title = Save the settings?
unsaved-body = Your changes aren't saved yet.
save = Save
discard = Discard

# Settings
tab-location = Location
tab-include = Include
tab-filters = Filters
tab-misc = Misc
settings-reading = Reading the settings…
settings-read-failed = Couldn't read the settings:
settings-saving = Saving the settings…
settings-saved = Settings saved
settings-saved-note = Settings saved, but: { $note }
settings-failed = Settings not saved: { $reason }
backend-save-failed = Couldn't save the setting: { $reason }
location-title = Backup disk
location-help = Choose the disk where snapshots are saved.
location-note = Snapshots go in timeshift/snapshots on this disk, where Timeshift finds them too.
device-away = Not connected ({ $id })
device-away-description = The disk in the settings. Plug it in, or choose another.
device-unusable = { $path } ({ $fstype }) can't hold snapshots: it's encrypted or a container.
include-help = The system is always included.
include-root = The root user's home folder.
include-home = Every user's home folder: documents and settings.
include-note = A full restore puts included home folders back as they were in the snapshot.
filters-none = No filters. Add one below.
filter-keep = Keep
add-folder = Add Folder
add-file = Add File
add-pattern = Add Pattern
remove = Remove
move-up = Move Up
move-down = Move Down
filters-help = Checked (+) keeps a path, unchecked (-) leaves it out. The first line that matches wins, top to bottom. Built-in excludes (/proc, /dev, caches) come before these.
pick-folder = Choose a folder
pick-file = Choose a file
remind-title = Remind me
remind-description = When the newest snapshot is older than this, the panel icon turns the warning colour. 0 turns it off.
remind-days = { $days } days
remind-off = Off
label-title = Panel label
label-description = Shows the last snapshot's age and the disk use next to the panel icon.

# About
about-source = Source code
