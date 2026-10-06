# If a restore goes wrong

The recovery steps for a restore that broke the system or left it unable to start.

A restore never touches the recovery partition or the **Pop_OS-oldkern** entry; the steps
below do: they rebuild that entry's initrd and rewrite its files on the ESP. They make no
backup of the ESP first (the restore itself does).

While preparing, Apsis writes `timeshift/apsis-restore-RECOVER.txt` on the backup disk: these
steps with this machine's disk UUIDs and the snapshots' names filled in (and, on an
encrypted system disk, the lines that unlock it), two complete
commands and no line to edit, each line short enough for an 80-column console. It's written
again at every preparation. When the restore is armed ("Restart now"), its copy is kept as
`/var/lib/apsis/restore/last-restore.note` on the system disk (root only), readable once step
2's first mount line has run. If the two differ, follow `last-restore.note`. `RECOVER.txt` is
on the disk that step 2 mounts, so the UUIDs for the mount lines come from `lsblk -f`.

**Known limit:** these steps are untried after a restore that changed the kernel (the
restore's filter keeps out the kernel that ran when it was prepared). When the system still
starts from either boot entry, the way back is the window: restore the safety snapshot there.

1. At power-on, hold Space for the systemd-boot menu and pick **Pop!_OS Recovery**, or boot
   a Pop!_OS live USB of the same version. The recovery opens an installer window: don't
   click its install choices (they reinstall the system).
2. In a terminal, one line at a time. If a line prints error, failed or E:, stop there: the
   lines after it count on it. `update-initramfs` and `kernelstub` print a lot; that alone
   is fine. `lsblk -f` shows the UUIDs, and Tab completes the paths.

   If the live system mounted the backup disk by itself, unmount it first:
   `sudo umount /dev/disk/by-uuid/<backup-uuid>`.

   On an encrypted system disk (Pop!_OS's "Encrypt drive"), unlock it first and bring up its
   LVM volume. Use the name the installed system's `/etc/crypttab` has for it (`cryptdata`
   on a default install; `RECOVER.txt` has the line with the name and the UUID filled in):
   `update-initramfs` below looks that name up, and with another name it builds an initrd
   that can't unlock the disk. `cryptsetup` asks for the disk's passphrase.

   ```sh
   sudo cryptsetup luksOpen /dev/disk/by-uuid/<luks-partition-uuid> cryptdata
   sudo vgchange -ay
   ```

   If `luksOpen` says the device is in use, the live system opened it under another name,
   which `lsblk` shows below the partition: `sudo vgchange -an`, then `sudo cryptsetup close
   <that name>`, then the two lines again. **Known limit:** the two unlock lines have run
   in a recovery once (2026-10-06, Pop!_OS's standard encrypted install, the same
   installation); the "in use" case has not.

   Mount the system disk, its boot partition (ESP) and the backup disk, read-only:

   ```sh
   sudo mount /dev/disk/by-uuid/<root-uuid> /mnt
   sudo mount /dev/disk/by-uuid/<esp-uuid> /mnt/boot/efi
   sudo mkdir -p /media/backup
   sudo mount -o ro /dev/disk/by-uuid/<backup-uuid> /media/backup
   ```

   The note on the system disk is root's; once the first mount line has run, read it with:

   ```sh
   sudo cat /mnt/var/lib/apsis/restore/last-restore.note
   ```

   Then one of these. Copy the line from `RECOVER.txt` (or the note): it has the right
   flags for the snapshot, and the names filled in.

   (a) Restore the same snapshot again:

   ```sh
   sudo rsync -a -A -X --numeric-ids --delete --force --sparse \
     --exclude-from=/mnt/var/lib/apsis/restore/last-restore.filter \
     /media/backup/timeshift/snapshots/<snapshot>/localhost/ /mnt/
   ```

   (b) Go back to the safety snapshot:

   ```sh
   sudo rsync -a -A -X --numeric-ids --delete --force --sparse \
     --exclude-from=/mnt/var/lib/apsis/restore/last-restore.filter \
     /media/backup/timeshift/snapshots/<safety-snapshot>/localhost/ /mnt/
   ```

   `-A -X` only for a snapshot made by Apsis 0.4.1 or later; a safety snapshot always is.
   Without a safety snapshot, `RECOVER.txt` says so in place of (b).

   If rsync can't open the filter, stop: never run these lines without it. Without
   `last-restore.filter` rsync exits 11 before it copies or deletes anything. After `apt
   purge apsis` the state folder is gone: the note's rsync lines stop with exit 11 and change
   nothing.

   Then the boot files:

   ```sh
   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done
   sudo chroot /mnt update-initramfs -u -k all
   sudo chroot /mnt kernelstub --verbose
   sudo rm -f /mnt/system-update
   ```

   `update-initramfs` rebuilds both initrds and takes about two minutes. The last line
   matters: while `/system-update` exists, the next start would try the restore again.

   Then check that the result boots: the ESP's kernel and initrd must be the ones `/boot`
   links to. Each line prints nothing when they are the same, and `differ` when not:

   ```sh
   sudo cmp /mnt/boot/efi/EFI/Pop_OS-<root-uuid>/vmlinuz.efi /mnt/boot/$(basename $(readlink /mnt/boot/vmlinuz))
   sudo cmp /mnt/boot/efi/EFI/Pop_OS-<root-uuid>/initrd.img /mnt/boot/$(basename $(readlink /mnt/boot/initrd.img))
   ```

   If either prints anything, don't restart: the computer may not start.

   The filter's rules are anchored at the transfer root, so they work unchanged against
   `/mnt/`. `--rbind /sys` brings `efivars` for kernelstub. `--numeric-ids` matters here: the
   live system's user database isn't the installed one.
3. Restart, from the panel. Nothing needs to be unmounted first.

After a restore by hand the window still names the restore before it: its line comes from
`result.json`, which these steps keep as it is.

If the status line after a restore says **incomplete**, the copy broke (most often the backup
disk was disconnected): reconnect it and click **Restore again**, or restore the safety
snapshot. **The restore didn't start** means nothing was changed. If a restore stops with an
error after the safety snapshot was taken, the snapshot stays in the list; delete it if you
don't want it. **Some files were not restored**: rsync's log names them, and it's root's:
`sudo less /var/lib/apsis/restore/rsync-log`. See the helper's log for the details:
`journalctl -b -1 -u apsis-restore` (the restore's own boot) and `journalctl -u apsis-helper`.
