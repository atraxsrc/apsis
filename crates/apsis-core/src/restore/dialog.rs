// SPDX-License-Identifier: GPL-3.0-only

//! What the Restore dialog needs to know about a snapshot (`CheckRestore`):
//! the first refusal, if any, in the order of the checks, and the lines that aren't
//! refusals (home files, `/root`, the old format, which Apsis the snapshot holds).
//!
//! Pure: the helper reads the files and runs the checks that need paths, and hands their
//! results in. [`to_wire`] and [`from_wire`] are the D-Bus shape, `(bsbbbs)`.

use super::apsis::{InSnapshot, in_snapshot};
use super::filter::{has_home, has_root};
use super::refusal::{Refusal, check_pending, crypttab_differs};
use crate::error::{Error, Result};
use crate::native::info::is_old_format;

/// The dialog on the bus, D-Bus type `(bsbbbs)`: `(ok, refusal, has_home, has_root,
/// old_format, apsis_note)`. `ok` is "no refusal"; `refusal` is [`Refusal::to_wire`]'s word
/// or `""`; `apsis_note` is [`InSnapshot::to_wire`]'s.
pub type WireCheckRestore = (bool, String, bool, bool, bool, String);

/// What `CheckRestore` found: the Restore dialog's content, or the "Can't restore" one's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    /// The first refusal in the checks' order; `None` means the snapshot can be restored.
    pub refusal: Option<Refusal>,
    /// The snapshot holds home files, so the dialog offers the choice.
    pub has_home: bool,
    /// The snapshot holds `/root` with content, so it's restored with the system (not a
    /// choice); otherwise the filter keeps the live `/root` whole.
    pub has_root: bool,
    /// Made without ACLs and extended attributes (0.4.1).
    pub old_format: bool,
    /// Which Apsis the snapshot holds.
    pub apsis: InSnapshot,
}

/// What the helper read and checked, for [`build`].
#[derive(Debug, Clone)]
pub struct Inputs<'a> {
    /// [`super::refusal::check`]'s result: the system and the snapshot.
    pub system: Result<(), Refusal>,
    /// [`super::refusal::pop_upgrade_found`]'s.
    pub pop_upgrade_found: &'a [&'a str],
    /// The snapshot's `etc/crypttab`, if it has one.
    pub snapshot_crypttab: Option<&'a str>,
    /// The live `/etc/crypttab`, empty if there's none.
    pub live_crypttab: &'a str,
    /// [`super::esp::check_before_arming`]'s result, on the live system.
    pub boot_files: Result<(), Refusal>,
    /// The snapshot's `exclude.list`, if it has one.
    pub snapshot_excludes: Option<&'a str>,
    /// The snapshot's `localhost/root` is a folder with something in it (owner, 2026-10-02):
    /// a list that lets `/root` in isn't `has_root` without it, or the restore's `--delete`
    /// would empty `/root` against an empty source.
    pub root_has_content: bool,
    /// The raw `apsis-rsync-flags` string from its `info.json`, `""` when missing.
    pub rsync_flags: &'a str,
    /// The snapshot's `var/lib/dpkg/status`, if it has one.
    pub dpkg_status: Option<&'a str>,
}

/// The dialog: the first refusal in the checks' order (the system and snapshot checks, the Pop!_OS
/// upgrade, the crypttab, the boot files), and the lines that aren't refusals.
#[must_use]
pub fn build(inputs: &Inputs<'_>) -> Dialog {
    let refusal = inputs
        .system
        .clone()
        .and_then(|()| check_pending(inputs.pop_upgrade_found))
        .and_then(|()| {
            if crypttab_differs(inputs.snapshot_crypttab, inputs.live_crypttab) {
                Err(Refusal::CrypttabDiffers)
            } else {
                Ok(())
            }
        })
        .and_then(|()| inputs.boot_files.clone())
        .err();
    let excludes = inputs.snapshot_excludes.unwrap_or_default();
    Dialog {
        refusal,
        has_home: has_home(excludes),
        has_root: has_root(excludes) && inputs.root_has_content,
        old_format: is_old_format(inputs.rsync_flags),
        apsis: in_snapshot(inputs.dpkg_status),
    }
}

/// The dialog as the helper sends it.
#[must_use]
pub fn to_wire(dialog: &Dialog) -> WireCheckRestore {
    (
        dialog.refusal.is_none(),
        dialog
            .refusal
            .as_ref()
            .map(Refusal::to_wire)
            .unwrap_or_default(),
        dialog.has_home,
        dialog.has_root,
        dialog.old_format,
        dialog.apsis.to_wire(),
    )
}

/// The dialog the helper sent, checked.
///
/// # Errors
///
/// [`Error::Helper`]: `ok` and `refusal` disagree, or a word this version doesn't know.
pub fn from_wire(wire: WireCheckRestore) -> Result<Dialog> {
    let (ok, refusal, has_home, has_root, old_format, apsis) = wire;
    let bad = |what: &str| Error::Helper(format!("CheckRestore reply: {what}"));
    let refusal = match (ok, refusal.as_str()) {
        (true, "") => None,
        (true, _) => return Err(bad("ok with a refusal")),
        (false, word) => Some(Refusal::from_wire(word).ok_or_else(|| bad("unknown refusal"))?),
    };
    let apsis = InSnapshot::from_wire(&apsis).ok_or_else(|| bad("unknown Apsis note"))?;
    Ok(Dialog {
        refusal,
        has_home,
        has_root,
        old_format,
        apsis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::restore::esp::CheckFailure;

    const EXCLUDES: &str = "/dev/*\n+ /home/**\n+ /root/**\n/home/*/**\n";
    const DPKG_0_5: &str = "Package: apsis\nStatus: install ok installed\nVersion: 0.5.0\n\n";

    fn good() -> Inputs<'static> {
        Inputs {
            system: Ok(()),
            pop_upgrade_found: &[],
            snapshot_crypttab: Some("# comment\n"),
            live_crypttab: "",
            boot_files: Ok(()),
            snapshot_excludes: Some(EXCLUDES),
            root_has_content: true,
            rsync_flags: "-aAX --numeric-ids",
            dpkg_status: Some(DPKG_0_5),
        }
    }

    #[test]
    fn a_restorable_snapshot_fills_the_dialog() {
        assert_eq!(
            build(&good()),
            Dialog {
                refusal: None,
                has_home: true,
                has_root: true,
                old_format: false,
                apsis: InSnapshot::Current,
            }
        );
    }

    /// The owner's rule (2026-10-02): a list that lets `/root` in isn't enough; the
    /// snapshot's `/root` must have content, or the restore would wipe `/root` against an
    /// empty source. Then `has_root` is false and the filter keeps `/root`.
    #[test]
    fn root_counts_only_when_the_snapshot_has_something_there() {
        let mut inputs = good();
        assert!(build(&inputs).has_root);
        inputs.root_has_content = false;
        assert!(!build(&inputs).has_root);
        // Content without the list letting it in: still false (rsync wouldn't copy it).
        inputs.root_has_content = true;
        inputs.snapshot_excludes = Some("/dev/*\n/root/**\n");
        assert!(!build(&inputs).has_root);
    }

    /// The checks' order: the system and snapshot checks, the Pop!_OS upgrade, the crypttab, the
    /// boot files. The first that fails is the refusal; the rest aren't consulted.
    #[test]
    fn the_first_refusal_in_the_tables_order_wins() {
        let mut inputs = good();
        inputs.boot_files = Err(Refusal::BootFiles(CheckFailure::NoEntry));
        assert_eq!(
            build(&inputs).refusal,
            Some(Refusal::BootFiles(CheckFailure::NoEntry))
        );
        inputs.snapshot_crypttab = Some("swap /dev/sdX2 /dev/urandom swap\n");
        assert_eq!(build(&inputs).refusal, Some(Refusal::CrypttabDiffers));
        inputs.pop_upgrade_found = &["/pop-upgrade"];
        assert_eq!(build(&inputs).refusal, Some(Refusal::PopUpgradePending));
        inputs.system = Err(Refusal::NotUefi);
        assert_eq!(build(&inputs).refusal, Some(Refusal::NotUefi));
    }

    #[test]
    fn the_lines_that_are_not_refusals() {
        let mut inputs = good();
        inputs.snapshot_excludes = Some("/dev/*\n/root/**\n/home/*/**\n");
        inputs.rsync_flags = "";
        inputs.dpkg_status = None;
        let dialog = build(&inputs);
        assert!(!dialog.has_home && !dialog.has_root && dialog.old_format);
        assert_eq!(dialog.apsis, InSnapshot::NotInstalled);
        // An unreadable snapshot has no excludes to ask: no home, no root.
        inputs.snapshot_excludes = None;
        inputs.system = Err(Refusal::Unreadable(
            crate::restore::refusal::Unreadable::NoExcludeList,
        ));
        let dialog = build(&inputs);
        assert!(!dialog.has_home && !dialog.has_root);
    }

    #[test]
    fn the_dialog_survives_the_bus() {
        let dialogs = [
            build(&good()),
            Dialog {
                refusal: Some(Refusal::SystemSpace { needs: 9, free: 1 }),
                has_home: false,
                has_root: true,
                old_format: true,
                apsis: InSnapshot::NoRestore {
                    version: "0.4.2".to_owned(),
                },
            },
            Dialog {
                refusal: None,
                has_home: true,
                has_root: false,
                old_format: false,
                apsis: InSnapshot::OldSettings {
                    version: "0.3.1".to_owned(),
                },
            },
        ];
        for dialog in dialogs {
            let wire = to_wire(&dialog);
            assert_eq!(wire.0, dialog.refusal.is_none(), "ok is no refusal");
            assert_eq!(from_wire(wire).unwrap(), dialog);
        }
        assert_eq!(
            to_wire(&build(&good())),
            (true, String::new(), true, true, false, "current".to_owned())
        );
        // A refused dialog with a word this version doesn't know, or ok with a word: bad.
        assert!(
            from_wire((
                false,
                "moon-phase".to_owned(),
                false,
                false,
                false,
                "current".to_owned()
            ))
            .is_err()
        );
        assert!(
            from_wire((
                true,
                "not-uefi".to_owned(),
                false,
                false,
                false,
                "current".to_owned()
            ))
            .is_err()
        );
        assert!(
            from_wire((
                true,
                String::new(),
                false,
                false,
                false,
                "apsis-9".to_owned()
            ))
            .is_err()
        );
    }

    #[test]
    fn which_apsis_survives_the_bus() {
        for apsis in [
            InSnapshot::Current,
            InSnapshot::NotInstalled,
            InSnapshot::NoRestore {
                version: "0.4.2".to_owned(),
            },
            InSnapshot::OldSettings {
                version: "0.3.1".to_owned(),
            },
        ] {
            assert_eq!(InSnapshot::from_wire(&apsis.to_wire()), Some(apsis));
        }
        assert_eq!(InSnapshot::NotInstalled.to_wire(), "not-installed");
        assert_eq!(
            InSnapshot::NoRestore {
                version: "0.4.2".to_owned()
            }
            .to_wire(),
            "no-restore:0.4.2"
        );
        assert_eq!(InSnapshot::from_wire("no-restore"), None);
        assert_eq!(InSnapshot::from_wire(""), None);
    }
}
