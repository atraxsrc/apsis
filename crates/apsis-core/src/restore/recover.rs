// SPDX-License-Identifier: GPL-3.0-only

//! `timeshift/apsis-restore-RECOVER.txt` on the backup disk: the README's "If a
//! restore goes wrong" steps with this machine's UUIDs and the snapshots' names filled in,
//! written while preparing. Two complete commands, one to restore the same snapshot again
//! and one to go back to the safety snapshot, so nobody edits a line in the recovery. It
//! holds only UUIDs, snapshot names and, on an encrypted system disk, the name its mapping
//! has in the crypttab. Every line fits an 80-column console.

use std::fs::File;
use std::io;
use std::path::Path;

use super::file::{self, FileError};
use super::refusal::Unlocking;

/// The note's name, in the backup disk's `timeshift/` folder next to `snapshots/`.
pub const FILE: &str = "apsis-restore-RECOVER.txt";

/// The note's working copy in the state folder, beside the filter and with its lifetime:
/// the same text as on the backup disk.
pub const NOTE_FILE: &str = "restore.note";

/// The copy of [`NOTE_FILE`] that an arm keeps: the note of the last restore that was armed.
pub const LAST_NOTE_FILE: &str = "last-restore.note";

/// Writes the note in both places, each in one step (a temporary file, flushed, renamed,
/// then the folder flushed): `state_dir/restore.note` for root alone, as the state folder's
/// other files, then `timeshift_dir/apsis-restore-RECOVER.txt` on the backup disk, readable
/// by anyone. A write that fails leaves the file it was writing as it was and no temporary
/// file; the second isn't tried after the first failed.
///
/// # Errors
///
/// [`FileError::Io`] if a write fails.
pub fn write(state_dir: &Path, timeshift_dir: &Path, text: &str) -> Result<(), FileError> {
    write_with(state_dir, timeshift_dir, text, file::write_all)
}

/// [`write`] with the call that puts the bytes into each temporary file.
fn write_with(
    state_dir: &Path,
    timeshift_dir: &Path,
    text: &str,
    mut write: impl FnMut(&mut File, &[u8]) -> io::Result<()>,
) -> Result<(), FileError> {
    file::save_with(state_dir, NOTE_FILE, text, 0o600, &mut write)?;
    file::save_with(timeshift_dir, FILE, text, file::READABLE, &mut write)
}

/// The rsync line that restores `snapshot` over the root mounted at `/mnt`, with the filter
/// the last arm kept. `old_format`: made without ACLs and extended attributes, so no `-A -X`.
fn rsync(snapshot: &str, old_format: bool) -> String {
    let acls = if old_format { "" } else { " -A -X" };
    format!(
        "   sudo rsync -a{acls} --numeric-ids --delete --force --sparse \\
     --exclude-from=/mnt/var/lib/apsis/restore/last-restore.filter \\
     /media/backup/timeshift/snapshots/{snapshot}/localhost/ /mnt/"
    )
}

/// The lines that unlock an encrypted system disk and bring up its LVM volume, before the
/// root can be mounted by its UUID. The name is the crypttab's: `update-initramfs` in the
/// chroot looks the root's mapping up there by name, so a disk opened under another name
/// (a file manager's `luks-<uuid>`) would get an initrd that can't unlock it. A name too
/// long for the line goes on a line of its own.
fn unlock_lines(unlocking: Unlocking<'_>) -> String {
    let Unlocking { name, luks_uuid } = unlocking;
    let device = format!("     /dev/disk/by-uuid/{luks_uuid}");
    let open = if device.len() + 1 + name.len() <= 80 {
        format!("{device} {name}")
    } else {
        format!("{device} \\\n     {name}")
    };
    // No `\` after the opening quote: it would eat the first line's indent.
    format!(
        "   The system disk is encrypted. Unlock it first, under exactly the name in
   the line below (update-initramfs looks that name up in the crypttab), then
   bring up its LVM volume. cryptsetup asks for the disk's passphrase.

   sudo cryptsetup luksOpen \\
{open}
   sudo vgchange -ay

   If luksOpen says the device is in use, the live system opened it under
   another name (lsblk shows it below the partition). Close that one first:
   vgchange -an, then cryptsetup close with that name, both with sudo. Then
   run the two lines again.

"
    )
}

/// The note's text for a restore of `snapshot` on the machine whose root, ESP and backup
/// disk have these filesystem UUIDs. `unlocking`: how an encrypted root is opened
/// ([`super::refusal::System::unlocking`]); `None` on a plain partition, whose note has no
/// such lines. `old_format`: the snapshot was made without ACLs and extended attributes, so
/// its line has no `-A -X`. `safety_snapshot`: the one this preparation took, always the new
/// format; with none, one line says so in place of its command.
#[must_use]
pub fn text(
    root_uuid: &str,
    unlocking: Option<Unlocking<'_>>,
    esp_uuid: &str,
    backup_uuid: &str,
    snapshot: &str,
    old_format: bool,
    safety_snapshot: Option<&str>,
) -> String {
    let same = rsync(snapshot, old_format);
    let back = safety_snapshot.map_or_else(
        || "   (b) No safety snapshot was taken for this restore.".to_owned(),
        |safety| {
            format!(
                "   (b) Go back to the safety snapshot, {safety}:\n\n{}",
                rsync(safety, false)
            )
        },
    );
    let unlock = unlocking.map(unlock_lines).unwrap_or_default();
    let changed_kernel = if safety_snapshot.is_some() {
        "restore the\nsafety snapshot from the window instead."
    } else {
        "restore\nanother snapshot from the window instead."
    };
    let (names, first) = if unlocking.is_some() {
        (
            "It holds only this machine's disk UUIDs, its encrypted disk's mapping name\n\
             and snapshot names.",
            "first mount line",
        )
    } else {
        (
            "It holds only this machine's disk UUIDs and snapshot names.",
            "first line",
        )
    };
    format!(
        "\
Apsis restore: if the computer doesn't start afterwards

Written by Apsis while preparing to restore the snapshot {snapshot}.
{names}
If this note and last-restore.note (step 2) differ, follow last-restore.note.

A restore never touches the recovery partition or the Pop_OS-oldkern entry;
step 2 rebuilds that entry's initrd and rewrites its files on the ESP.
If the restore changed the kernel and the system still starts, {changed_kernel}

1. At power-on, hold Space for the systemd-boot menu and pick Pop!_OS Recovery,
   or boot a Pop!_OS live USB of the same version.
   An installer window may open: don't click its install choices (a reinstall).

2. In a terminal, one line at a time. lsblk -f shows the same UUIDs as below.
   If a line prints error, failed or E:, stop there: the lines after it count
   on it. update-initramfs and kernelstub print a lot; that alone is fine.
   After the {first} this note can also be read, with sudo, as
   /mnt/var/lib/apsis/restore/last-restore.note

{unlock}   sudo mount /dev/disk/by-uuid/{root_uuid} /mnt
   sudo mount /dev/disk/by-uuid/{esp_uuid} /mnt/boot/efi
   sudo mkdir -p /media/backup
   sudo mount -o ro /dev/disk/by-uuid/{backup_uuid} \\
     /media/backup

   Then one of these:

   (a) Restore the same snapshot again, {snapshot}:

{same}

{back}

   If rsync can't open the filter, stop: never run these lines without it.

   Then:

   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done
   sudo chroot /mnt update-initramfs -u -k all
   sudo chroot /mnt kernelstub --verbose
   sudo rm -f /mnt/system-update

3. Restart.

The filter's rules are anchored at the transfer root, so they work unchanged
against /mnt/. --numeric-ids matters here: the live system's user database
isn't the installed one.
See also \"If a restore goes wrong\" in Apsis's README.
"
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    use super::*;
    use crate::restore::file::tests::temp_dir;

    // Fake UUIDs and names only.
    const ROOT: &str = "11111111-1111-1111-1111-111111111111";
    const ESP: &str = "AAAA-0001";
    const BACKUP: &str = "00000000-0000-0000-0000-000000000000";
    const NAME: &str = "2026-09-25_11-28-53";
    const SAFETY: &str = "2026-10-04_09-15-07";

    const LUKS: &str = "22222222-2222-2222-2222-222222222222";
    /// An encrypted Pop!_OS install's root: the mapping `cryptdata` of the LUKS partition.
    const UNLOCKING: Unlocking<'static> = Unlocking {
        name: "cryptdata",
        luks_uuid: LUKS,
    };
    const GOLDEN_ENCRYPTED: &str = include_str!("../../tests/fixtures/recover/encrypted.txt");

    const GOLDEN_SAFETY: &str = include_str!("../../tests/fixtures/recover/safety-snapshot.txt");
    const GOLDEN_NO_SAFETY: &str =
        include_str!("../../tests/fixtures/recover/no-safety-snapshot.txt");

    const FILTER: &str = "--exclude-from=/mnt/var/lib/apsis/restore/last-restore.filter";

    /// The commands as a person types them: the lines indented by three spaces that start
    /// with `sudo ` or `for `, each with the lines it continues onto (a trailing `\`).
    fn commands(note: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut lines = note.lines();
        while let Some(line) = lines.next() {
            if line.starts_with("   sudo ") || line.starts_with("   for ") {
                let mut command = line.to_owned();
                while command.ends_with('\\') {
                    command.push('\n');
                    command.push_str(lines.next().expect("the line a `\\` continues onto"));
                }
                found.push(command);
            }
        }
        found
    }

    fn sh_n(command: &str) -> bool {
        let mut sh = Command::new("sh")
            .arg("-n")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh");
        sh.stdin
            .take()
            .unwrap()
            .write_all(format!("{command}\n").as_bytes())
            .unwrap();
        sh.wait().unwrap().success()
    }

    fn rsync_lines(note: &str) -> Vec<String> {
        commands(note)
            .into_iter()
            .filter(|c| c.trim_start().starts_with("sudo rsync "))
            .collect()
    }

    #[test]
    fn with_a_safety_snapshot_the_note_is_the_golden_file_byte_for_byte() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY));
        assert_eq!(note, GOLDEN_SAFETY);
    }

    #[test]
    fn without_a_safety_snapshot_the_note_is_the_golden_file_byte_for_byte() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, None);
        assert_eq!(note, GOLDEN_NO_SAFETY);
    }

    #[test]
    fn on_an_encrypted_disk_the_note_is_the_golden_file_byte_for_byte() {
        let note = text(
            ROOT,
            Some(UNLOCKING),
            ESP,
            BACKUP,
            NAME,
            false,
            Some(SAFETY),
        );
        assert_eq!(note, GOLDEN_ENCRYPTED);
    }

    /// The disk is unlocked under the crypttab's name and its volume brought up before the
    /// root is mounted; the rest of the note is the plain one's, line for line.
    #[test]
    fn an_encrypted_disk_is_unlocked_by_name_before_the_root_is_mounted() {
        let note = text(
            ROOT,
            Some(UNLOCKING),
            ESP,
            BACKUP,
            NAME,
            false,
            Some(SAFETY),
        );
        let commands = commands(&note);
        assert_eq!(
            commands[..3],
            [
                format!("   sudo cryptsetup luksOpen \\\n     /dev/disk/by-uuid/{LUKS} cryptdata"),
                "   sudo vgchange -ay".to_owned(),
                format!("   sudo mount /dev/disk/by-uuid/{ROOT} /mnt"),
            ]
        );
        assert_eq!(commands.len(), 12);
        for command in &commands {
            assert!(sh_n(command), "sh -n refused:\n{command}");
        }
        let plain = text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY));
        assert_eq!(commands[2..], self::commands(&plain)[..]);
        let only_here: Vec<&str> = note
            .lines()
            .filter(|line| !plain.lines().any(|p| p == *line))
            .collect();
        assert!(
            only_here.iter().all(|line| !line.contains("rsync")),
            "{only_here:#?}"
        );
        assert!(!plain.contains("cryptsetup") && !plain.contains("vgchange"));
        assert!(note.is_ascii() && !note.contains('@'));
    }

    /// A long mapping name goes on a line of its own, and no line wraps.
    #[test]
    fn a_long_mapping_name_keeps_every_line_in_80_columns() {
        for length in [1, 20, 21, 60] {
            let name = "n".repeat(length);
            let unlocking = Unlocking {
                name: &name,
                luks_uuid: LUKS,
            };
            let note = text(ROOT, Some(unlocking), ESP, BACKUP, NAME, false, None);
            for line in note.lines() {
                assert!(line.chars().count() <= 80, "{length}: {line}");
            }
            let open = &commands(&note)[0];
            assert!(open.ends_with(&name) && sh_n(open), "{open}");
            assert_eq!(open.lines().count(), if length <= 20 { 2 } else { 3 });
        }
    }

    #[test]
    fn without_a_safety_snapshot_one_line_takes_the_place_of_the_go_back() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, None);
        assert_eq!(rsync_lines(&note).len(), 1);
        let said: Vec<_> = note
            .lines()
            .filter(|l| l.contains("safety snapshot"))
            .collect();
        assert_eq!(
            said,
            ["   (b) No safety snapshot was taken for this restore."]
        );
    }

    #[test]
    fn every_command_pulled_out_of_the_note_passes_sh_n() {
        // The extraction and `sh -n` can fail: a broken line is caught.
        assert!(!sh_n("   for d in dev; do sudo mount --rbind /$d /mnt/$d;"));
        assert!(!sh_n("   sudo rsync -a \\\n     '--exclude-from=x"));
        for (note, count) in [
            (text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY)), 10),
            (text(ROOT, None, ESP, BACKUP, NAME, false, None), 9),
            (text(ROOT, None, ESP, BACKUP, NAME, true, Some(SAFETY)), 10),
        ] {
            let commands = commands(&note);
            assert_eq!(commands.len(), count, "{commands:#?}");
            for command in commands {
                assert!(sh_n(&command), "sh -n refused:\n{command}");
            }
        }
    }

    #[test]
    fn both_commands_name_the_kept_filter_and_their_snapshot_in_full() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY));
        let [same, back] = <[String; 2]>::try_from(rsync_lines(&note)).unwrap();
        for (command, name) in [(&same, NAME), (&back, SAFETY)] {
            assert!(command.contains(FILTER), "{command}");
            assert!(
                command.contains(&format!(
                    "/media/backup/timeshift/snapshots/{name}/localhost/ /mnt/"
                )),
                "{command}"
            );
            assert!(command.contains("--numeric-ids --delete --force --sparse"));
        }
        assert!(back.contains("sudo rsync -a -A -X "), "{back}");
        // Each is labelled, by its snapshot's full name.
        assert!(note.contains(&format!("(a) Restore the same snapshot again, {NAME}:")));
        assert!(note.contains(&format!("(b) Go back to the safety snapshot, {SAFETY}:")));
        assert!(
            !note.contains("restore/restore.filter"),
            "the working filter"
        );
    }

    /// The go-back's flags are its own. A safety snapshot is always the new format.
    #[test]
    fn after_an_old_format_snapshot_only_the_same_restore_drops_acls_and_attributes() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, true, Some(SAFETY));
        let [same, back] = <[String; 2]>::try_from(rsync_lines(&note)).unwrap();
        assert!(same.contains("sudo rsync -a --numeric-ids "), "{same}");
        assert!(!same.contains("-A -X"), "{same}");
        assert!(
            back.contains("sudo rsync -a -A -X --numeric-ids "),
            "{back}"
        );
        assert!(back.contains(SAFETY));
    }

    #[test]
    fn no_line_wraps_on_an_80_column_console() {
        // ROOT and BACKUP are as long as a filesystem UUID gets (36); the ESP is vfat, whose
        // UUID is always nine characters.
        for note in [
            text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY)),
            text(ROOT, None, ESP, BACKUP, NAME, true, None),
        ] {
            for line in note.lines() {
                assert!(line.chars().count() <= 80, "{} > 80: {line}", line.len());
            }
        }
    }

    #[test]
    fn the_note_keeps_what_check_8_typed_and_holds_no_names() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY));
        for needle in [
            &format!("   sudo mount /dev/disk/by-uuid/{ROOT} /mnt\n"),
            &format!("   sudo mount /dev/disk/by-uuid/{ESP} /mnt/boot/efi\n"),
            "   sudo mkdir -p /media/backup\n",
            &format!("   sudo mount -o ro /dev/disk/by-uuid/{BACKUP} \\\n     /media/backup\n"),
            "   for d in dev proc sys run; do sudo mount --rbind /$d /mnt/$d; done\n",
            "   sudo chroot /mnt update-initramfs -u -k all\n",
            "   sudo chroot /mnt kernelstub --verbose\n",
            "   sudo rm -f /mnt/system-update\n",
            "lsblk -f",
            "Pop!_OS Recovery",
        ] {
            assert!(note.contains(needle), "missing {needle:?}\n{note}");
        }
        assert!(!note.contains('@'), "no names, no emails");
        assert!(note.is_ascii());
        assert!(note.ends_with('\n'));
        assert_eq!(FILE, "apsis-restore-RECOVER.txt");
    }

    /// What lands in both places is `text`, byte for byte.
    #[test]
    fn what_is_written_is_the_text_byte_for_byte_in_both_places() {
        let (state, backup) = (temp_dir("recover-state"), temp_dir("recover-backup"));
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY));
        write(&state, &backup, &note).unwrap();
        assert_eq!(fs::read(state.join(NOTE_FILE)).unwrap(), note.as_bytes());
        assert_eq!(fs::read(backup.join(FILE)).unwrap(), note.as_bytes());
        // The state folder's copy is root's alone, as its other files; the backup disk's is
        // readable by anyone, as the plain write made it (it holds only UUIDs and names).
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&state.join(NOTE_FILE)), 0o600);
        assert_eq!(mode(&backup.join(FILE)), 0o644);
        let left = |d: &Path| fs::read_dir(d).unwrap().count();
        assert_eq!((left(&state), left(&backup)), (1, 1), "no temporary file");
    }

    /// The write cut midway, by a stand-in writer that writes half and fails: the earlier
    /// note stays whole and no partial file is left, under either name.
    #[test]
    fn a_write_cut_midway_leaves_the_earlier_note_whole_and_no_partial_file() {
        let note = text(ROOT, None, ESP, BACKUP, NAME, false, Some(SAFETY));
        let half = |file: &mut fs::File, bytes: &[u8]| {
            file.write_all(&bytes[..bytes.len() / 2])?;
            Err(io::Error::other("cut"))
        };
        // The backup disk's write is the one cut; the state folder's went through.
        let (state, backup) = (
            temp_dir("recover-cut-state"),
            temp_dir("recover-cut-backup"),
        );
        fs::write(backup.join(FILE), "the earlier note\n").unwrap();
        let calls = Cell::new(0);
        let second_is_cut = |file: &mut fs::File, bytes: &[u8]| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                half(file, bytes)
            } else {
                file.write_all(bytes)
            }
        };
        let FileError::Io(error) = write_with(&state, &backup, &note, second_is_cut).unwrap_err()
        else {
            panic!("an I/O error")
        };
        assert_eq!(error.to_string(), "cut");
        assert_eq!(
            fs::read_to_string(backup.join(FILE)).unwrap(),
            "the earlier note\n"
        );
        assert_eq!(
            fs::read_dir(&backup).unwrap().count(),
            1,
            "no temporary file"
        );
        assert_eq!(fs::read(state.join(NOTE_FILE)).unwrap(), note.as_bytes());

        // The first write cut: nothing is written in either place, nothing is left.
        let (state, backup) = (
            temp_dir("recover-cut1-state"),
            temp_dir("recover-cut1-backup"),
        );
        assert!(write_with(&state, &backup, &note, half).is_err());
        assert_eq!(fs::read_dir(&state).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&backup).unwrap().count(), 0);
    }
}
