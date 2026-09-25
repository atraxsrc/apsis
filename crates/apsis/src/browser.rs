// SPDX-License-Identifier: GPL-3.0-only

//! The snapshot browser's model (Phase 6a): one folder of a snapshot at a time, the selected
//! entry, and the entries marked for restore. Drawing it is in `app.rs`.

use std::collections::BTreeSet;

use apsis_core::restore::{Entry, Kind, Listing, Live, SnapPath};

/// The shown folder's entries, as far as they've arrived.
#[derive(Debug, Clone)]
pub enum Load {
    Loading,
    Failed(String),
    Ready(Listing),
}

#[derive(Debug, Clone)]
pub struct Browser {
    pub snapshot: String,
    /// The folder shown.
    pub path: SnapPath,
    pub load: Load,
    /// Index into the entries.
    pub cursor: usize,
    /// Marked for restore, from any folder.
    pub marked: BTreeSet<SnapPath>,
    /// After going up: the folder we came from, selected once the parent arrives.
    reselect: Option<String>,
}

impl Browser {
    /// At the snapshot's `/`, waiting for its entries.
    pub fn new(snapshot: String) -> Self {
        Self {
            snapshot,
            path: SnapPath::root(),
            load: Load::Loading,
            cursor: 0,
            marked: BTreeSet::new(),
            reselect: None,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        match &self.load {
            Load::Ready(listing) => &listing.entries,
            Load::Loading | Load::Failed(_) => &[],
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.load, Load::Loading)
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries().get(self.cursor)
    }

    /// The selected entry's snapshot path.
    pub fn current_path(&self) -> Option<SnapPath> {
        self.path.join(&self.current()?.name).ok()
    }

    pub fn select(&mut self, index: usize) {
        self.cursor = index.min(self.entries().len().saturating_sub(1));
    }

    /// Into the selected folder. Returns the folder to fetch, or `None` if the selection isn't
    /// a folder.
    pub fn enter(&mut self) -> Option<SnapPath> {
        if self.current()?.kind != Kind::Dir {
            return None;
        }
        let path = self.current_path()?;
        self.go(path.clone(), None);
        Some(path)
    }

    /// Up one folder, selecting the one we were in. `None` at `/`.
    pub fn up(&mut self) -> Option<SnapPath> {
        let parent = self.path.parent()?;
        let came_from = self.path.name().map(str::to_owned);
        self.go(parent.clone(), came_from);
        Some(parent)
    }

    /// The same folder again (the running system may have changed).
    pub fn reload(&mut self) -> SnapPath {
        let name = self.current().map(|e| e.name.clone());
        self.go(self.path.clone(), name);
        self.path.clone()
    }

    fn go(&mut self, path: SnapPath, reselect: Option<String>) {
        self.path = path;
        self.load = Load::Loading;
        self.cursor = 0;
        self.reselect = reselect;
    }

    /// A browse finished. Kept only if it's for the folder shown now (not one left since).
    /// Folders first, then by name.
    pub fn loaded(&mut self, path: &SnapPath, result: Result<Listing, String>) {
        if *path != self.path {
            return;
        }
        self.load = match result {
            Ok(mut listing) => {
                listing.entries.sort_by(|a, b| {
                    (a.kind != Kind::Dir, &a.name).cmp(&(b.kind != Kind::Dir, &b.name))
                });
                Load::Ready(listing)
            }
            Err(reason) => Load::Failed(reason),
        };
        let reselect = self.reselect.take();
        self.cursor = reselect
            .and_then(|name| self.entries().iter().position(|e| e.name == name))
            .unwrap_or(0);
    }

    /// Marks or unmarks the selected entry. The cursor stays on it, so the details pane shows
    /// what was marked.
    pub fn toggle_mark(&mut self) {
        let Some(path) = self.current_path() else {
            return;
        };
        if !self.marked.remove(&path) {
            self.marked.insert(path);
        }
    }

    /// `J`: marks or unmarks the selected entry, then moves down, for marking a run of rows.
    pub fn toggle_mark_and_move(&mut self) {
        self.toggle_mark();
        self.select(self.cursor + 1);
    }

    pub fn is_marked(&self, entry: &Entry) -> bool {
        self.path
            .join(&entry.name)
            .is_ok_and(|path| self.marked.contains(&path))
    }

    /// What `R` restores: the marked entries, or the selected one if none are marked.
    pub fn targets(&self) -> Vec<String> {
        if self.marked.is_empty() {
            return self
                .current_path()
                .map(|p| p.to_string())
                .into_iter()
                .collect();
        }
        self.marked.iter().map(ToString::to_string).collect()
    }

    /// `2026-09-25_03-00-01:/etc/NetworkManager`, cut from the left to `max` characters.
    pub fn breadcrumb(&self, max: usize) -> String {
        let full = format!("{}:{}", self.snapshot, self.path);
        let count = full.chars().count();
        if count <= max {
            return full;
        }
        let keep: String = full.chars().skip(count - max.saturating_sub(1)).collect();
        format!("…{keep}")
    }
}

/// `ls -l` style: `-rwxr-xr-x`, `drwxr-xr-x`, `lrwxrwxrwx`, with `s`/`S` and `t`/`T`.
pub fn mode_string(kind: Kind, mode: u32) -> String {
    let file_type = match kind {
        Kind::File => '-',
        Kind::Dir => 'd',
        Kind::Link => 'l',
        Kind::Other => match mode & 0o170_000 {
            0o010_000 => 'p',
            0o140_000 => 's',
            0o060_000 => 'b',
            0o020_000 => 'c',
            _ => '?',
        },
    };
    let bit = |mask: u32, c: char| if mode & mask != 0 { c } else { '-' };
    let special = |exec: u32, flag: u32, on: char| match (mode & exec != 0, mode & flag != 0) {
        (true, true) => on,
        (false, true) => on.to_ascii_uppercase(),
        (true, false) => 'x',
        (false, false) => '-',
    };
    [
        file_type,
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        special(0o100, 0o4000, 's'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        special(0o010, 0o2000, 's'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        special(0o001, 0o1000, 't'),
    ]
    .iter()
    .collect()
}

/// The marker a row shows for how the running system compares.
pub fn live_marker(live: Live) -> char {
    match live {
        Live::Missing => '+',
        Live::Changed => '~',
        Live::Same | Live::Present => '=',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            size: 1,
            mtime: 0,
            mode: 0o100_644,
            uid: 0,
            gid: 0,
            owner: "root:root".to_owned(),
            target: String::new(),
            live: Live::Missing,
            live_size: 0,
            live_mtime: 0,
        }
    }

    fn listing(entries: &[(&str, Kind)]) -> Result<Listing, String> {
        Ok(Listing {
            entries: entries.iter().map(|(n, k)| entry(n, *k)).collect(),
            truncated: false,
        })
    }

    fn at(path: &str) -> SnapPath {
        SnapPath::parse(path).unwrap()
    }

    #[test]
    fn folders_come_first_and_up_reselects_where_we_were() {
        let mut browser = Browser::new("2026-09-25_03-00-01".to_owned());
        browser.loaded(
            &SnapPath::root(),
            listing(&[
                ("vmlinuz", Kind::Link),
                ("etc", Kind::Dir),
                ("usr", Kind::Dir),
            ]),
        );
        let names: Vec<_> = browser.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["etc", "usr", "vmlinuz"]);
        browser.select(1);
        assert_eq!(browser.enter(), Some(at("/usr")));
        assert!(browser.is_loading());
        browser.loaded(&at("/usr"), listing(&[("bin", Kind::Dir)]));
        assert_eq!(browser.up(), Some(SnapPath::root()));
        browser.loaded(
            &SnapPath::root(),
            listing(&[("etc", Kind::Dir), ("usr", Kind::Dir)]),
        );
        assert_eq!(browser.current().unwrap().name, "usr");
        assert_eq!(browser.up(), None);
        // A file isn't entered.
        browser.loaded(&SnapPath::root(), listing(&[("hosts", Kind::File)]));
        assert_eq!(browser.enter(), None);
    }

    #[test]
    fn late_answers_for_another_folder_are_dropped() {
        let mut browser = Browser::new("s".to_owned());
        browser.loaded(&SnapPath::root(), listing(&[("etc", Kind::Dir)]));
        browser.enter();
        browser.loaded(&SnapPath::root(), listing(&[("stale", Kind::File)]));
        assert!(browser.is_loading());
        browser.loaded(&at("/etc"), Err("refused".to_owned()));
        assert!(matches!(&browser.load, Load::Failed(r) if r == "refused"));
    }

    #[test]
    fn marks_span_folders_and_default_to_the_selection() {
        let mut browser = Browser::new("s".to_owned());
        browser.loaded(
            &SnapPath::root(),
            listing(&[("etc", Kind::Dir), ("opt", Kind::Dir)]),
        );
        assert_eq!(browser.targets(), ["/etc"]);
        browser.toggle_mark();
        assert_eq!(browser.cursor, 0, "space stays on the marked row");
        assert!(browser.is_marked(&browser.entries()[0].clone()));
        browser.toggle_mark_and_move();
        assert_eq!(browser.cursor, 1, "J moves down");
        assert!(browser.marked.is_empty(), "J toggles too");
        browser.select(0);
        browser.toggle_mark();
        browser.select(1);
        browser.enter();
        browser.loaded(
            &at("/opt"),
            listing(&[("a", Kind::File), ("b", Kind::File)]),
        );
        browser.select(1);
        browser.toggle_mark();
        assert_eq!(browser.targets(), ["/etc", "/opt/b"]);
        assert!(browser.is_marked(&browser.entries()[1].clone()));
        browser.select(1);
        browser.toggle_mark();
        assert_eq!(browser.targets(), ["/etc"]);
    }

    #[test]
    fn breadcrumbs_are_cut_from_the_left() {
        let mut browser = Browser::new("2026-09-25_03-00-01".to_owned());
        browser.path = at("/etc/NetworkManager/system-connections");
        assert_eq!(
            browser.breadcrumb(100),
            "2026-09-25_03-00-01:/etc/NetworkManager/system-connections"
        );
        let cut = browser.breadcrumb(20);
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.starts_with('…') && cut.ends_with("system-connections"));
    }

    #[test]
    fn modes_read_like_ls() {
        assert_eq!(mode_string(Kind::File, 0o100_644), "-rw-r--r--");
        assert_eq!(mode_string(Kind::File, 0o104_755), "-rwsr-xr-x");
        assert_eq!(mode_string(Kind::File, 0o104_644), "-rwSr--r--");
        assert_eq!(mode_string(Kind::Dir, 0o041_777), "drwxrwxrwt");
        assert_eq!(mode_string(Kind::Link, 0o120_777), "lrwxrwxrwx");
        assert_eq!(mode_string(Kind::Other, 0o010_644), "prw-r--r--");
        assert_eq!(live_marker(Live::Missing), '+');
        assert_eq!(live_marker(Live::Changed), '~');
        assert_eq!(live_marker(Live::Present), '=');
    }
}
