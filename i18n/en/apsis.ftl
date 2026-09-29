app-title = Apsis
app-comment = Simple system snapshots and file restore for the COSMIC™ desktop
app-keywords =

tooltip-last = last  { $ago }
tooltip-last-stale = last  { $ago }  (over { $days } days)
tooltip-last-none = last  none yet
tooltip-last-none-stale = last  none yet  (over { $days } days)
tooltip-next-manual = next  manual only

waiting = reading snapshots…
not-loaded = press [r] to list snapshots
empty = no snapshots yet - press [c] to create one
no-device = no backup device chosen
no-device-hint = pick one in settings [s], then press [r]

error-label = error:
need-helper = Apsis needs apsis-helper, which the .deb installs
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
pane-restore-plan = restore plan
pane-restore-result = restore result

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
help-browse = browse the selected snapshot's files
help-browse-into = browser: into the folder
help-browse-up = browser: up one folder
help-browse-mark = browser: mark or unmark for restore
help-browse-mark-down = browser: mark or unmark, then move down
help-browse-restore = browser: restore the marked (or the selected) entries
help-browse-reload = browser: read the folder again
help-create = create a snapshot, with a comment
help-mark = mark or unmark for deletion
help-mark-down = mark or unmark, then move down
help-delete = delete the marked (or the selected) snapshots (confirm with y)
help-refresh = refresh the list
help-settings = settings: backup device, home folders, filters
help-rooms = window rooms: snapshots, create, schedule (up/down, space, + -), log
help-help = show or hide this help
help-escape = cancel, go back, clear marks, close the popup
help-settings-change = settings: change the selected row
help-settings-count = settings: one more, one fewer, type it (keep, remind)
help-settings-filters = settings: add a filter, remove the selected one
help-settings-write = settings: write them (asks for your password)
help-settings-reload = settings: read them again, dropping changes

menu-open = Open Apsis
menu-refresh = Refresh
menu-settings = Settings…
menu-about = About Apsis
menu-remove-or-move = Remove or move applet…
menu-close = Close

about-license = license
about-source = source

prompt-comment = comment:
prompt-delete = delete { $name }? [y/N]
prompt-delete-many = delete { $count } snapshots: { $names }? [y/N]
creating = creating snapshot…
deleting = deleting { $name }…
deleting-many = deleting { $step }/{ $count }: { $name }…
created = snapshot created
deleted = deleted { $name }
deleted-many = deleted { $count } snapshots
delete-cancelled = delete cancelled
create-failed = create failed: { $reason }
delete-failed = delete failed: { $reason }
delete-many-stopped = delete stopped at { $name }: { $reason }
delete-many-deleted = deleted ({ $count }): { $names }
delete-many-kept = not deleted ({ $count }): { $names }
delete-none = none

prompt-filter = add filter:

settings-device = device
settings-home = home
settings-filters = filters
settings-apsis = apsis
settings-device-unset = none selected
settings-device-away = not connected ({ $id })
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
settings-key-user = user
settings-key-home = home
settings-key-backup = backup
settings-key-kind = kind
settings-filter-include = include
settings-filter-exclude = exclude
settings-filter-home = home folder (the home rows change it)

settings-device-note = Snapshots go to this device. Space picks the next one that can hold them.
settings-device-none = No backup device selected. Space picks one.
settings-device-missing-note = Not connected now. It stays chosen; plug it in to use it.
settings-home-excluded-note = excluded: none of it.
settings-home-hidden-note = hidden files only: app settings, not documents.
settings-home-all-note = everything: all files; a full restore rolls documents back too.
settings-home-encrypted = An encrypted (ecryptfs) home uses other patterns; Apsis leaves its setting as it is.
settings-filter-note = rsync uses the first filter that matches. + in front includes. x removes this one.
settings-add-note = Enter a pattern like /var/lib/libvirt/** or *.mp3. Start with "+ " to include instead of exclude.

settings-reading = reading settings
settings-read-failed = couldn't read the settings:
settings-need-helper = settings need apsis-helper (install the .deb)
settings-saving = writing settings
settings-saved = settings written to /etc/apsis/config.toml
settings-saved-note = settings written, but: { $note }
settings-failed = settings not written: { $reason }
settings-unchanged = nothing changed
settings-unsaved = unsaved changes: [w] writes them, Esc again drops them
settings-edit-hint = e types a number on keep or remind
backend-save-failed = couldn't save the setting: { $reason }

browse-reading = reading { $path }… (authenticate if asked)
browse-failed = couldn't read this folder:
browse-empty = empty folder
browse-truncated = … more entries not shown (10,000 at most)
browse-marked = { $count } marked
restore-need-helper = browsing and restoring need apsis-helper (install the .deb)
prompt-restore-where = restore { $count } item(s) to [f]older (~/Apsis-restored) or [o]riginal?
prompt-restore-confirm = put { $count } item(s) back over the running system? [y/N]
restore-cancelled = restore cancelled
restore-dry-running = restore dry run…
restoring = restoring…
restore-plan-ready = restore plan: nothing written yet; Enter runs it, Esc goes back
restore-dry-run-failed = restore dry run failed: { $reason }
restore-done = restore done; what was copied is in the left pane and in journalctl -u apsis-helper
restore-failed = restore failed: { $reason }

details-path = path
details-type = type
details-size = size
details-modified = modified
details-mode = mode
details-owner = owner
details-link = link
details-live = live
details-restore = restore
details-marked = marked
details-delete = delete
entry-file = file
entry-folder = folder
entry-link = symlink (restored as a symlink)
entry-special = special file (not restored to a folder)
live-missing = not on the running system
live-same = same size and time as on the running system
live-present = folder exists on the running system (contents not compared)
live-changed = differs from the running system
live-changed-file = differs: size { $size } here, { $live_size } now; modified { $time } here, { $live_time } now

# Disk usage line under the panes, and the panel tooltip's disk line
disk-label = disk
tooltip-disk = disk  { $used } / { $total }  { $free } free
tooltip-disk-low = disk  { $free } free  (under { $pct }%)
tooltip-disk-not-connected = disk  not connected
tooltip-disk-unknown = disk  unknown

# Progress of a create or restore in the activity pane, before the bar:
# `creating snapshot · 58% · 3m 12s left`, or `creating snapshot · working · 1m 08s elapsed`
progress-creating = creating snapshot
progress-restoring = restoring
progress-percent = { $label } · { $percent }
progress-percent-left = { $label } · { $percent } · { $time } left
progress-working = { $label } · working · { $elapsed } elapsed

# Keep last N snapshots (settings, prune preview) and the reminder
settings-apsis-saved = saved
prompt-keep-manual = keep (0 = off):
prompt-remind = remind after days (0 = off):
settings-keep-manual = keep last { $count }
settings-keep-manual-off = keep: all
settings-keep-manual-note = Saved at once. Keeps the newest N snapshots without a comment; [p] previews which older ones go and deletes them after y (also offered after a create). Commented ones are pinned: they stay and don't count.
settings-remind = remind after { $days } days
settings-remind-off = remind: off
settings-panel-label = panel label: on
settings-panel-label-off = panel label: off
settings-remind-note = Saved at once. When the newest snapshot is older than this, the panel icon turns the warning colour and the tooltip says so.
settings-panel-label-note = Saved at once. Shows the age of the newest snapshot and the share of the backup disk in use (12h · 62%) beside the panel icon. Horizontal panels only.
pane-prune = prune preview
prune-delete = delete  { $name }
prune-keep-recent = keep    { $name }  newest { $count }
prune-keep-comment = keep    { $name }  comment
prune-keep-newest = keep    { $name }  newest snapshot
prompt-prune = remove { $count } old snapshots? [y/N]
prune-off = keep is off: set it in settings [s]
prune-nothing = nothing to prune: keeping the last { $keep }
help-prune = old snapshots past "keep": preview, then y

# The apsis pane above the panes in the window: time on the left, backup disk on the right
pane-apsis = apsis
strip-label-last = last
strip-label-next = next
strip-next-manual = manual only
strip-last-none = none yet
strip-over = (over { $days } days)
strip-under = (under { $pct }%)
strip-used-free = { $pct }% used · { $free } free
strip-disk-not-connected = not connected
strip-disk-unknown = unknown
overview-more = +{ $count } older

# The window's rooms (dock cells), their panes, and their text
room-snapshots = snapshots
room-create = create
room-schedule = schedule
room-log = log
pane-create = create
pane-schedule = schedule
pane-log = log
create-label-comment = comment
create-label-tag = tag
create-comment-none = press [c] to write one
create-tag = O on-demand
create-unavailable = No backup disk to create on.
schedule-label-keep = keep
schedule-label-remind = remind
schedule-note = There is no scheduler yet: Apsis creates a snapshot when you ask. Keep and remind are saved at once.
log-empty = nothing yet
log-error = error
