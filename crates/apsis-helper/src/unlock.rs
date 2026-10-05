// SPDX-License-Identifier: GPL-3.0-only

//! Whether a refreshed ESP can still unlock the system disk: the end of the apply's boot
//! refresh on a system whose `/` isn't on a plain partition (core's
//! `refusal::is_restorable_root` lets an encrypted Pop!_OS install through).
//!
//! The refreshed ESP must find and unlock `/` the way the ESP that booted this system did.
//! What fails it fails the boot refresh, so the apply puts the boot files back (PLAN 6b.6
//! step 6).
//!
//! The same installation only. `/etc/fstab` and `/etc/crypttab` aren't copied (the filter's
//! `DISKS`), a snapshot of another root UUID or with another crypttab is refused before
//! arming, and no initrd is rebuilt: the snapshot's was built on this system.

use std::fmt;

/// What an initrd holds to unlock an encrypted root and to find a root on LVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pieces {
    /// `sbin/cryptsetup`.
    pub cryptsetup: bool,
    /// `sbin/lvm`.
    pub lvm: bool,
    /// The size in bytes of `cryptroot/crypttab`, the entries unlocked in the initramfs
    /// (0 on a system without an encrypted root). `None`: the file isn't there.
    pub crypttab: Option<u64>,
}

/// For the journal: `cryptsetup, lvm, cryptroot/crypttab of 62 bytes`, or what's missing.
impl fmt::Display for Pieces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let has = |there: bool, name: &str| format!("{}{name}", if there { "" } else { "no " });
        let crypttab = match self.crypttab {
            Some(size) => format!("cryptroot/crypttab of {size} bytes"),
            None => "no cryptroot/crypttab".to_owned(),
        };
        write!(
            f,
            "{}, {}, {crypttab}",
            has(self.cryptsetup, "cryptsetup"),
            has(self.lvm, "lvm")
        )
    }
}

/// The pieces in `lsinitramfs -l`'s output: `cpio -tv` lines (mode, links, owner, group,
/// size, three fields of date, name). Only regular files count.
#[must_use]
pub fn pieces(listing: &str) -> Pieces {
    let mut found = Pieces::default();
    for line in listing.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(mode), Some(size), Some(name)) = (fields.first(), fields.get(4), fields.get(8))
        else {
            continue;
        };
        if !mode.starts_with('-') {
            continue;
        }
        let is = |path: &str| *name == path || name.ends_with(&format!("/{path}"));
        if is("sbin/cryptsetup") {
            found.cryptsetup = true;
        } else if is("sbin/lvm") {
            found.lvm = true;
        } else if is("cryptroot/crypttab") {
            found.crypttab = Some(size.parse().unwrap_or(0));
        }
    }
    found
}

/// The `root=` option of a loader entry (`loader/entries/Pop_OS-current.conf`).
#[must_use]
pub fn root_option(entry: &str) -> Option<&str> {
    entry
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("options"))
        .flat_map(str::split_whitespace)
        .find(|option| option.starts_with("root="))
}

/// The boot entry and the initrd of an ESP: before the refresh (the backup) or after it.
#[derive(Debug, Clone, Copy)]
pub struct Boot<'a> {
    /// The text of `Pop_OS-current.conf`.
    pub entry: &'a str,
    /// Of `initrd.img`.
    pub pieces: Pieces,
}

/// Whether the refreshed ESP still finds and unlocks `/` as the one before did: the same
/// `root=` option, and every piece the initrd from before had. A system on a plain partition
/// had none of the pieces, so only `root=` is compared there.
///
/// # Errors
///
/// What changed or is missing, for the journal and the result's message.
pub fn check(before: &Boot<'_>, after: &Boot<'_>) -> Result<(), String> {
    let (root_before, root_after) = (root_option(before.entry), root_option(after.entry));
    if root_before != root_after {
        // No UUIDs in the message: it reaches result.json and the window.
        return Err(
            "the new boot entry's root= option isn't the one this system booted with".to_owned(),
        );
    }
    let mut missing = Vec::new();
    if before.pieces.cryptsetup && !after.pieces.cryptsetup {
        missing.push("cryptsetup");
    }
    if before.pieces.lvm && !after.pieces.lvm {
        missing.push("lvm");
    }
    let has_entries = |pieces: Pieces| pieces.crypttab.is_some_and(|size| size > 0);
    if has_entries(before.pieces) && !has_entries(after.pieces) {
        missing.push("the entries of cryptroot/crypttab");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the new initrd lacks what this system's disk needs at boot: {}",
            missing.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lsinitramfs -l` of an initrd built on LUKS with LVM, in short: the early (microcode)
    /// part's lines, folders, a link, the three pieces and their neighbours.
    const ENCRYPTED: &str = "\
drwxr-xr-x   2 root     root            0 Oct  1 10:00 kernel
-rw-r--r--   1 root     root        12345 Oct  1 10:00 kernel/x86/microcode/GenuineIntel.bin
drwxr-xr-x   2 root     root            0 Oct  1 10:00 cryptroot
-rw-r--r--   1 root     root           66 Oct  1 10:00 cryptroot/crypttab
lrwxrwxrwx   1 root     root            8 Apr 22  2024 sbin -> usr/sbin
-rwxr-xr-x   1 root     root       163944 Apr  8  2024 usr/sbin/cryptsetup
-rwxr-xr-x   1 root     root      3021000 Apr  8  2024 usr/sbin/lvm
lrwxrwxrwx   1 root     root            3 Apr  8  2024 usr/sbin/vgchange -> lvm
-rwxr-xr-x   1 root     root         1200 Apr  8  2024 scripts/local-top/cryptroot
-rwxr-xr-x   1 root     root          900 Apr  8  2024 scripts/local-block/lvm2
";
    /// The same of a system on a plain partition: an empty `cryptroot/crypttab`, no tools.
    const PLAIN: &str = "\
drwxr-xr-x   2 root     root            0 Oct  1 10:00 cryptroot
-rw-r--r--   1 root     root            0 Oct  1 10:00 cryptroot/crypttab
-rwxr-xr-x   1 root     root        50000 Apr  8  2024 usr/sbin/blkid
";
    const ENTRY: &str = "\
title Pop!_OS
linux /EFI/Pop_OS-11111111-1111-1111-1111-111111111111/vmlinuz.efi
initrd /EFI/Pop_OS-11111111-1111-1111-1111-111111111111/initrd.img
options root=UUID=11111111-1111-1111-1111-111111111111 ro quiet loglevel=0 splash
";

    fn boot<'a>(entry: &'a str, listing: &str) -> Boot<'a> {
        Boot {
            entry,
            pieces: pieces(listing),
        }
    }

    #[test]
    fn the_pieces_are_read_from_the_listing() {
        assert_eq!(
            pieces(ENCRYPTED),
            Pieces {
                cryptsetup: true,
                lvm: true,
                crypttab: Some(66),
            }
        );
        assert_eq!(
            pieces(PLAIN),
            Pieces {
                cryptsetup: false,
                lvm: false,
                crypttab: Some(0),
            }
        );
        assert_eq!(pieces(""), Pieces::default());
        assert_eq!(
            pieces(ENCRYPTED).to_string(),
            "cryptsetup, lvm, cryptroot/crypttab of 66 bytes"
        );
        assert_eq!(
            Pieces::default().to_string(),
            "no cryptsetup, no lvm, no cryptroot/crypttab"
        );
        // A link or a folder of the name isn't the program, and nor is a longer name.
        let lookalikes = "\
lrwxrwxrwx   1 root     root            3 Apr  8  2024 usr/sbin/lvm -> foo
drwxr-xr-x   2 root     root            0 Apr  8  2024 usr/sbin/cryptsetup
-rwxr-xr-x   1 root     root          100 Apr  8  2024 usr/sbin/lvmdump
-rwxr-xr-x   1 root     root          100 Apr  8  2024 usr/lib/notsbin/lvm
";
        assert_eq!(pieces(lookalikes), Pieces::default());
    }

    #[test]
    fn the_root_option_is_the_options_lines() {
        assert_eq!(
            root_option(ENTRY),
            Some("root=UUID=11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(root_option("title Pop!_OS\noptions ro quiet\n"), None);
        assert_eq!(root_option("linux /EFI/root=x/vmlinuz.efi\n"), None);
    }

    #[test]
    fn the_same_boot_passes_on_an_encrypted_and_on_a_plain_system() {
        assert_eq!(
            check(&boot(ENTRY, ENCRYPTED), &boot(ENTRY, ENCRYPTED)),
            Ok(())
        );
        assert_eq!(check(&boot(ENTRY, PLAIN), &boot(ENTRY, PLAIN)), Ok(()));
        // More than before is fine: an initrd with the tools on a plain system.
        assert_eq!(check(&boot(ENTRY, PLAIN), &boot(ENTRY, ENCRYPTED)), Ok(()));
    }

    #[test]
    fn an_initrd_without_a_piece_from_before_fails() {
        let error = check(&boot(ENTRY, ENCRYPTED), &boot(ENTRY, PLAIN)).unwrap_err();
        assert_eq!(
            error,
            "the new initrd lacks what this system's disk needs at boot: cryptsetup, lvm, the \
             entries of cryptroot/crypttab"
        );
        let no_lvm = ENCRYPTED.replace("usr/sbin/lvm\n", "usr/sbin/other\n");
        let error = check(&boot(ENTRY, ENCRYPTED), &boot(ENTRY, &no_lvm)).unwrap_err();
        assert!(error.ends_with("at boot: lvm"), "{error}");
        let empty_crypttab =
            ENCRYPTED.replace("66 Oct  1 10:00 cryptroot", "0 Oct  1 10:00 cryptroot");
        let error = check(&boot(ENTRY, ENCRYPTED), &boot(ENTRY, &empty_crypttab)).unwrap_err();
        assert!(
            error.ends_with("the entries of cryptroot/crypttab"),
            "{error}"
        );
    }

    #[test]
    fn another_root_option_fails_and_the_message_has_no_uuid() {
        let other = ENTRY.replace("root=UUID=1111", "root=UUID=2222");
        let error = check(&boot(ENTRY, ENCRYPTED), &boot(&other, ENCRYPTED)).unwrap_err();
        assert!(
            error.contains("root= option") && !error.contains("1111"),
            "{error}"
        );
        let none = ENTRY.replace("root=UUID=11111111-1111-1111-1111-111111111111 ", "");
        assert!(check(&boot(ENTRY, PLAIN), &boot(&none, PLAIN)).is_err());
    }
}
