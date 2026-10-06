# Troubleshooting

What the messages mean, what to do about them, upgrading from older versions and uninstalling.

| you see | what to do |
|---|---|
| Backup disk not connected | plug it in; Apsis notices and lists again |
| the list fails with a `mount` error that says the disk is already mounted (on `/media/...`) | the desktop opened the backup disk by itself: unmount it in Files (unmount, don't eject), then Refresh |
| can't hold snapshots: it's encrypted or a container | encrypted backup disks aren't supported yet; pick a plain one |
| No backup disk chosen | Settings → Location |
| Apsis needs apsis-helper | reinstall the .deb, or `sudo just install` |
| apt stops with "apsis: an Apsis job is running; try again when it has finished" | a snapshot, a delete, a restore's preparation or a settings save is running: let it finish (or stop it), then run apt again. The package is as it was |
| apt stops with "apsis: a restore is armed and couldn't be disarmed" | a restore waits for its restart and couldn't be cancelled; run the two commands apt printed, then apt again |
| the applet doesn't show after installing | log out and back in, or re-add it in *Configure panel applets* |
| Can't restore this snapshot | the dialog says why and what to do; see "Known limitations of restore" |
| Not deleted: a restart to restore is waiting | "Restart now" was clicked and the restore waits for the restart; restart the computer, or wait ten minutes for it to time out, then delete |
| The preparation is too old / is gone | the "Ready to restore" prompt waited more than 30 minutes, or the helper was restarted; start the restore again |
| Restore incomplete · system partly restored | the copy broke; reconnect the backup disk and **Restore again**, or restore the safety snapshot; see "If a restore goes wrong" |
| Restore finished · some files were not restored | rsync couldn't write or delete some files; the rest of the system is the snapshot's. Which ones: `sudo less /var/lib/apsis/restore/rsync-log` |
| Restore finished · a cleanup step failed | the system is restored; a step after the copy failed (for example, the kernel from before the restore couldn't be removed); the tooltip says what |
| Restore finished · the computer may not start next time | the boot files on the ESP aren't in order: before you restart, read "If a restore goes wrong" and have the recovery or a live USB ready; the tooltip says what went wrong |
| System restored · still boots the previous kernel | the boot files couldn't be refreshed, or failed the check, and the old ones were kept; the next kernel update should set it right |
| the computer keeps restarting into the restore | boot the recovery and `rm /mnt/system-update` as in "If a restore goes wrong" |
| anything else | the helper's log: `journalctl -u apsis-helper -e` |

## Still stuck?

- **The logs.** `journalctl -u apsis-helper -e` is the helper's log. After a restore,
  `journalctl -b -1 -u apsis-restore` is the log of the restore's own boot.
- **The manual page**: `man apsis`.
- **After a restore**: [If a restore goes wrong](RECOVERY.md).
- **Ask, or report a bug**: [open an issue](https://github.com/atraxsrc/apsis/issues). Say
  which version (the window's About page shows it), what you did, what you expected and
  what the status line or the dialog said, with the log lines around it. Logs hold disk
  UUIDs, your user name and your computer's name: take out what you don't want public.
- **A security problem** doesn't go in an issue: see [Security](../SECURITY.md).

## Upgrading

**Upgrading, reinstalling or removing while Apsis is busy.** From 0.5.0 the package waits
its turn. While a snapshot, a delete, a restore's preparation or a settings save runs, apt
stops with `apsis: an Apsis job is running; try again when it has finished`: the package
stays as it was and the job goes on. Run the same command again once the job has finished.
At the "Ready to restore" prompt the package operation goes on, and the window then says the
preparation is gone. After "Restart now", while the restore waits for its restart, the
package operation cancels the restore and says `apsis: the restore that was waiting for a
restart is cancelled`; the safety snapshot stays, and the restore can be started again
afterwards.

**The upgrade from 0.4.x doesn't wait.** It runs the installed 0.4.x package's script, which
stops the helper even in the middle of a snapshot or a delete. Upgrade from 0.4.x while
nothing runs.

**Upgrading from 0.3?** Open the settings once. Your old settings are shown as they will be
saved (a note at the top says what changed: a home folder set to "everything" becomes the
`/home` choice, "hidden files only" becomes a `+` filter); snapshots keep holding the same
files. Click Save. File restore is gone in 0.4: anything already in `~/Apsis-restored/`, and
any `*.apsis-before-*` file next to an original, is yours and can be deleted by hand.

## Uninstall

Your snapshots stay on the backup disk. `apt purge` also removes `/var/lib/apsis`, the
restore's state folder, including the kept recovery pair (`last-restore.filter` and
`last-restore.note`). `apt remove` is refused while a job runs, and cancels a restore that
waits for its restart (see "Upgrading, reinstalling or removing while Apsis is busy").
`just uninstall` runs none of the package's checks: it stops the helper even in the middle
of a job and doesn't cancel a restore that waits for its restart, so run it only while
nothing runs and no restore waits.
