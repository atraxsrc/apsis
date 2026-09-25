// SPDX-License-Identifier: GPL-3.0-only

//! What a restore did or would do, read from rsync's itemized output.

use std::fmt::{self, Write as _};

/// rsync's `--out-format`: a marker, the itemized changes (`%i`), the length (`%l`), the name
/// (`%n`) and, for links, ` -> target` / ` => target` (`%L`). The marker tells these lines from
/// rsync's other messages on stdout.
pub const OUT_FORMAT: &str = "--out-format=APSIS %i %l %n%L";
const MARKER: &str = "APSIS ";

/// Most item lines a plan's text shows; the totals count them all.
pub const MAX_PLAN_ITEMS: usize = 1000;

/// Where the files go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// `~/Apsis-restored/<snapshot>/<original path>` of the caller, never overwriting.
    Folder,
    /// Back where they were, the current ones kept as `<name>.apsis-before-<snapshot>`.
    Original,
}

impl Destination {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Destination::Folder => "folder",
            Destination::Original => "original",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "folder" => Some(Destination::Folder),
            "original" => Some(Destination::Original),
            _ => None,
        }
    }
}

/// What rsync does with one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A new folder.
    CreateDir,
    /// A new file (or special file, in original mode).
    Copy,
    /// A new symlink.
    Link,
    /// A new hard link to an item copied in the same run.
    HardLink,
    /// An existing item's content or link target replaced (backed up first in original mode).
    Replace,
    /// Only the owner, mode, times, ACLs or xattrs of an existing item change.
    Attrs,
}

impl Action {
    fn word(self) -> &'static str {
        match self {
            Action::CreateDir => "create",
            Action::Copy => "copy",
            Action::Link => "link",
            Action::HardLink => "hardlink",
            Action::Replace => "replace",
            Action::Attrs => "attrs",
        }
    }
}

/// One line of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub action: Action,
    /// rsync's file type letter: `f` file, `d` folder, `L` symlink, `D` device, `S` special.
    pub kind: char,
    /// Absolute, as on the system (`/etc/hosts`); in folder mode, below the target folder.
    pub path: String,
    pub size: u64,
    /// ` -> target` for a symlink, ` => other` for a hard link, else empty.
    pub link: String,
    /// Original mode: where the current item goes before it's replaced.
    pub backup: Option<String>,
}

/// The items in rsync's stdout (lines from [`OUT_FORMAT`]), with `prefix` (the folder rsync
/// wrote into, as shown: `""` for `/`, else `/etc`) before each name. `backup_suffix`: original
/// mode's, for [`Item::backup`]. Other stdout lines are returned as notes (`skipping
/// non-regular file ...`), except rsync's `created directory`.
#[must_use]
pub fn parse_itemized(
    stdout: &str,
    prefix: &str,
    backup_suffix: Option<&str>,
) -> (Vec<Item>, Vec<String>) {
    let mut items = Vec::new();
    let mut notes = Vec::new();
    for line in stdout.lines() {
        match line
            .strip_prefix(MARKER)
            .and_then(|rest| item(rest, prefix))
        {
            Some(mut item) => {
                if item.action == Action::Replace {
                    item.backup = backup_suffix.map(|suffix| format!("{}{suffix}", item.path));
                }
                items.push(item);
            }
            None if line.trim().is_empty() || line.starts_with("created directory ") => {}
            None => notes.push(unescape(line.trim())),
        }
    }
    (items, notes)
}

/// `>f+++++++++ 10 etc/hosts` (after the marker).
fn item(rest: &str, prefix: &str) -> Option<Item> {
    let (flags, rest) = rest.split_once(' ')?;
    let (size, name) = match rest.split_once(' ') {
        Some((length, name)) if length.parse::<u64>().is_ok() => (length.parse().ok(), name),
        _ => (None, rest),
    };
    let flags: Vec<char> = flags.chars().collect();
    if flags.len() != 11 {
        return None;
    }
    let (update, kind) = (flags[0], flags[1]);
    let new = flags[2..].iter().all(|&c| c == '+');
    let (name, link) = match (update, kind) {
        ('h', _) => split_link(name, " => "),
        (_, 'L') => split_link(name, " -> "),
        _ => (name, ""),
    };
    let action = match (update, kind) {
        ('*', _) => return None,
        ('h', _) => Action::HardLink,
        (_, 'd') if new => Action::CreateDir,
        (_, 'd') => Action::Attrs,
        (_, 'L') if new => Action::Link,
        ('c', 'L') => Action::Replace,
        ('>' | 'c', _) if new => Action::Copy,
        ('>' | 'c', _) => Action::Replace,
        _ => Action::Attrs,
    };
    let name = unescape(name.trim_end_matches('/'));
    Some(Item {
        action,
        kind,
        path: format!("{prefix}/{name}"),
        size: size.unwrap_or(0),
        link: link.to_owned(),
        backup: None,
    })
}

fn split_link<'a>(name: &'a str, arrow: &'static str) -> (&'a str, &'a str) {
    match name.find(arrow) {
        Some(at) => (&name[..at], &name[at..]),
        None => (name, ""),
    }
}

/// rsync writes unprintable bytes in names as `\#ooo` (octal).
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 2..i + 5)
            .filter(|_| bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'#'))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        if let Some(byte) = octal {
            out.push(byte);
            i += 5;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A restore's plan (dry run) or result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub snapshot: String,
    pub destination: Destination,
    pub dry_run: bool,
    /// Folder mode: the folder the files go into. Original mode: empty.
    pub target: String,
    pub items: Vec<Item>,
    /// `/etc` and `/usr` in original mode.
    pub warnings: Vec<String>,
    /// rsync's other messages (`skipping non-regular file "..."`).
    pub notes: Vec<String>,
    /// Each rsync argv, in order.
    pub commands: Vec<Vec<String>>,
}

impl Plan {
    /// Items that are replaced (and, in original mode, backed up).
    #[must_use]
    pub fn replaced(&self) -> usize {
        self.count(|item| item.action == Action::Replace)
    }

    fn count(&self, pick: impl Fn(&Item) -> bool) -> usize {
        self.items.iter().filter(|item| pick(item)).count()
    }

    /// `3 files, 1 folder, 1 link, 1.5 KiB; 1 replaced, 1 backed up`
    #[must_use]
    pub fn summary(&self) -> String {
        let files =
            self.count(|i| matches!(i.action, Action::Copy | Action::Replace) && i.kind == 'f');
        let dirs = self.count(|i| i.action == Action::CreateDir);
        let links = self.count(|i| matches!(i.action, Action::Link | Action::HardLink));
        let bytes: u64 = self
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::Copy | Action::Replace))
            .map(|i| i.size)
            .sum();
        let mut text = format!(
            "{files} {}, {dirs} {}, {links} {}, {}",
            plural(files, "file", "files"),
            plural(dirs, "folder", "folders"),
            plural(links, "link", "links"),
            size(bytes)
        );
        let replaced = self.replaced();
        let attrs = self.count(|i| i.action == Action::Attrs);
        // Writing to a String can't fail.
        if replaced > 0 {
            let _ = write!(text, "; {replaced} replaced");
            if self.destination == Destination::Original {
                let _ = write!(text, ", {replaced} backed up");
            }
        }
        if attrs > 0 {
            let _ = write!(text, "; {attrs} with other attributes only");
        }
        text
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = if self.dry_run {
            "restore plan (dry run, nothing written)"
        } else {
            "restored"
        };
        writeln!(
            f,
            "{what}: {} mode, from {}",
            self.destination.word(),
            self.snapshot
        )?;
        match self.destination {
            Destination::Folder => writeln!(f, "to {}", self.target)?,
            Destination::Original => writeln!(f, "to the original places")?,
        }
        for warning in &self.warnings {
            writeln!(f, "warning: {warning}")?;
        }
        if self.items.is_empty() {
            writeln!(f, "  nothing to copy: everything is already the same")?;
        }
        for item in self.items.iter().take(MAX_PLAN_ITEMS) {
            write!(f, "  {:<8} {}{}", item.action.word(), item.path, item.link)?;
            if item.kind == 'f' && matches!(item.action, Action::Copy | Action::Replace) {
                write!(f, "   {}", size(item.size))?;
            }
            if let Some(backup) = &item.backup {
                write!(f, "   backup {backup}")?;
            }
            writeln!(f)?;
        }
        if let Some(more) = self
            .items
            .len()
            .checked_sub(MAX_PLAN_ITEMS)
            .filter(|&n| n > 0)
        {
            writeln!(f, "  ... and {more} more")?;
        }
        writeln!(f, "{}", self.summary())?;
        for note in &self.notes {
            writeln!(f, "rsync: {note}")?;
        }
        for command in &self.commands {
            writeln!(f, "$ {}", command.join(" "))?;
        }
        Ok(())
    }
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

/// `980 B`, `1.2 KiB`, `3.4 GiB`.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    #[allow(clippy::cast_precision_loss, reason = "shown with one decimal")]
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real rsync 3.2.7 output (folder mode, `--relative`, then original mode).
    const FOLDER: &str = "created directory /run/apsis/restore-dry-run
skipping non-regular file \"etc/nm/fifo\"
APSIS cd+++++++++ 4096 etc/
APSIS >f+++++++++ 10 etc/hosts
APSIS cd+++++++++ 4096 etc/nm/
APSIS >f+++++++++ 2 etc/nm/hard
APSIS hf+++++++++ 2 etc/nm/a.conf => etc/nm/hard
APSIS cL+++++++++ 11 etc/nm/vi -> /usr/bin/vi
";
    const ORIGINAL: &str = "APSIS >f.s.p..... 10 hosts
APSIS .f...p..... 3 motd
APSIS .d..t...... 4096 nm/
APSIS cL.c....... 5 nm/link -> other
APSIS >f+++++++++ 7 new\\#012line
";

    #[test]
    fn folder_mode_items_and_notes() {
        let (items, notes) = parse_itemized(FOLDER, "", None);
        let actions: Vec<_> = items.iter().map(|i| (i.action, i.path.as_str())).collect();
        assert_eq!(
            actions,
            [
                (Action::CreateDir, "/etc"),
                (Action::Copy, "/etc/hosts"),
                (Action::CreateDir, "/etc/nm"),
                (Action::Copy, "/etc/nm/hard"),
                (Action::HardLink, "/etc/nm/a.conf"),
                (Action::Link, "/etc/nm/vi"),
            ]
        );
        assert_eq!(items[5].link, " -> /usr/bin/vi");
        assert_eq!(items[4].link, " => etc/nm/hard");
        assert_eq!(items[1].size, 10);
        // Without a length (not what rsync prints, but not a reason to lose the line).
        let (odd, _) = parse_itemized("APSIS >f+++++++++ name with spaces", "", None);
        assert_eq!(odd[0].path, "/name with spaces");
        assert_eq!(notes, ["skipping non-regular file \"etc/nm/fifo\""]);
        assert!(items.iter().all(|i| i.backup.is_none()));
    }

    #[test]
    fn original_mode_replaces_are_backed_up() {
        let (items, _) = parse_itemized(ORIGINAL, "/etc", Some(".apsis-before-S"));
        let got: Vec<_> = items
            .iter()
            .map(|i| (i.action, i.path.as_str(), i.backup.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                (
                    Action::Replace,
                    "/etc/hosts",
                    Some("/etc/hosts.apsis-before-S")
                ),
                (Action::Attrs, "/etc/motd", None),
                (Action::Attrs, "/etc/nm", None),
                (
                    Action::Replace,
                    "/etc/nm/link",
                    Some("/etc/nm/link.apsis-before-S")
                ),
                (Action::Copy, "/etc/new\nline", None),
            ]
        );
    }

    #[test]
    fn plan_text_shows_items_totals_and_commands() {
        let (items, notes) = parse_itemized(ORIGINAL, "/etc", Some(".apsis-before-S"));
        let plan = Plan {
            snapshot: "2026-09-25_03-00-01".to_owned(),
            destination: Destination::Original,
            dry_run: true,
            target: String::new(),
            items,
            warnings: vec!["/etc is live system configuration".to_owned()],
            notes,
            commands: vec![vec!["rsync".to_owned(), "-aHAX".to_owned()]],
        };
        let text = plan.to_string();
        assert!(
            text.starts_with(
                "restore plan (dry run, nothing written): original mode, from 2026-09-25_03-00-01\n"
            ),
            "{text}"
        );
        assert!(text.contains("warning: /etc is live system configuration"));
        assert!(
            text.contains("  replace  /etc/hosts   10 B   backup /etc/hosts.apsis-before-S"),
            "{text}"
        );
        assert!(text.contains("2 replaced, 2 backed up"), "{text}");
        assert!(text.contains("$ rsync -aHAX"));
        assert_eq!(plan.replaced(), 2);
    }

    #[test]
    fn long_plans_are_cut_but_counted() {
        let stdout: String = (0..MAX_PLAN_ITEMS + 5)
            .map(|n| format!("APSIS >f+++++++++ 1 f{n}\n"))
            .collect();
        let (items, _) = parse_itemized(&stdout, "", None);
        let plan = Plan {
            snapshot: String::new(),
            destination: Destination::Folder,
            dry_run: true,
            target: "/home/u/Apsis-restored/x".to_owned(),
            items,
            warnings: vec![],
            notes: vec![],
            commands: vec![],
        };
        let text = plan.to_string();
        assert!(text.contains("  ... and 5 more"));
        assert!(text.contains(&format!("{} files", MAX_PLAN_ITEMS + 5)));
    }

    #[test]
    fn sizes_read_like_ls() {
        assert_eq!(size(980), "980 B");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn destinations_round_trip() {
        for d in [Destination::Folder, Destination::Original] {
            assert_eq!(Destination::from_word(d.word()), Some(d));
        }
        assert_eq!(Destination::from_word("both"), None);
    }
}
