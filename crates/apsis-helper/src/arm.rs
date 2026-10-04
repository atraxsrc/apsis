// SPDX-License-Identifier: GPL-3.0-only

//! Arming the next boot and undoing it (PLAN 6b.5, 6b.6): the plan's filter and note kept
//! as `last-restore.*` and rsync's log cleared, then the unit, its wants link, the drop-in,
//! the helper copy and `state.json`, `sync`, then `/system-update` last; a disarm removes
//! the link first. Leftovers without the link arm nothing and are cleaned.
//! The disarm timer: a transient systemd timer in the current boot that runs
//! `apsis-helper --disarm` after ten minutes without a restart. The package's `prerm` runs
//! it too, before a remove or an upgrade.

use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use apsis_core::restore::refusal::UpdateLink;
use apsis_core::restore::state::{STATE_FILE, State as RestoreState};
use apsis_core::restore::{argv, file, plan, unit};

/// The transient disarm units: `apsis-disarm.timer` and `apsis-disarm.service`.
pub const DISARM_UNIT: &str = "apsis-disarm";

/// Where the arm's files are: under `/` for real, under a temp root in the tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub unit: PathBuf,
    pub wants_link: PathBuf,
    pub drop_in: PathBuf,
    pub helper_copy: PathBuf,
    /// `/var/lib/apsis/restore`: `request.json`, `state.json` and the rest.
    pub state_dir: PathBuf,
    /// `/system-update`, the commit point.
    pub link: PathBuf,
    /// `/etc/system-update`, systemd's other name for it.
    pub etc_link: PathBuf,
}

impl Paths {
    #[must_use]
    pub fn system() -> Self {
        Self::under(Path::new("/"))
    }

    /// The design's paths (`apsis_core::restore::unit`, `file::DIR`) under `root`.
    #[must_use]
    pub fn under(root: &Path) -> Self {
        let at = |absolute: &str| root.join(absolute.trim_start_matches('/'));
        Self {
            unit: at(unit::UNIT_PATH),
            wants_link: at(unit::UNIT_WANTS_LINK),
            drop_in: at(unit::DROP_IN_PATH),
            helper_copy: at(unit::HELPER_COPY),
            state_dir: at(file::DIR),
            link: at("/system-update"),
            etc_link: at("/etc/system-update"),
        }
    }
}

/// What's at a link's name, asked of the name itself (`lstat`), for
/// [`apsis_core::restore::refusal::check_arming`].
#[must_use]
pub fn link_state(path: &Path) -> UpdateLink {
    match fs::symlink_metadata(path) {
        Err(_) => UpdateLink::Nothing,
        Ok(meta) if !meta.file_type().is_symlink() => UpdateLink::Other,
        Ok(_) => {
            if fs::metadata(path).is_ok() {
                UpdateLink::Link
            } else {
                UpdateLink::Dangling
            }
        }
    }
}

/// `/system-update` is Apsis's link: it points at the state folder.
#[must_use]
pub fn is_armed(paths: &Paths) -> bool {
    fs::read_link(&paths.link).is_ok_and(|target| target == paths.state_dir)
}

/// Arms the next boot, in PLAN 6b.5's order: first the plan's filter and note are kept as
/// `last-restore.filter` and `last-restore.note` ([`keep_last`]) and rsync's log of the
/// restore before is cleared; then the unit, its wants link, the drop-in, the helper copy
/// (`helper_exe`, the packaged helper, copied so a restored snapshot can't take it away
/// mid-restore), a fresh `state.json`, then everything synced to disk, then the link last.
/// Nothing before the link arms anything; a failure part-way leaves leftovers for
/// [`clean_leftovers`].
///
/// # Errors
///
/// The pair couldn't be kept (nothing of the arm is written then, and the pair and the log
/// of the arm before are as they were), or a file couldn't be written; the link isn't made
/// either way.
pub fn arm(paths: &Paths, helper_exe: &Path) -> io::Result<()> {
    keep_last(paths)?;
    remove_if_there(&paths.state_dir.join(argv::LOG_FILE))?;
    write_file(&paths.unit, unit::unit_text())?;
    make_link(&paths.wants_link, &paths.unit)?;
    write_file(&paths.drop_in, unit::drop_in_text())?;
    fs::create_dir_all(&paths.state_dir)?;
    fs::copy(helper_exe, &paths.helper_copy)?;
    fs::set_permissions(&paths.helper_copy, fs::Permissions::from_mode(0o755))?;
    RestoreState::default()
        .save(&paths.state_dir)
        .map_err(|error| io::Error::other(error.to_string()))?;
    rustix::fs::sync();
    make_link(&paths.link, &paths.state_dir)?;
    rustix::fs::sync();
    Ok(())
}

/// Copies the plan's working files to the pair of the last arm (PLAN 6b.5):
/// `restore.filter` to `last-restore.filter` and `restore.note` to `last-restore.note`, with
/// the state folder's writer. "Last restore" means "last arm": an arm that's disarmed has
/// replaced the pair too.
///
/// # Errors
///
/// A working file is missing or a copy couldn't be written: both targets are as they were.
fn keep_last(paths: &Paths) -> io::Result<()> {
    file::copy_all(&paths.state_dir, &plan::KEPT).map_err(|error| {
        io::Error::other(format!(
            "the restore's filter and note couldn't be kept: {error}"
        ))
    })
}

/// Removes the plan and its working files (the filter and the note); what was there, by
/// name. Never part of [`remove_arm_files`]: that runs right before an arm, on the plan
/// that's about to be armed.
fn remove_plan_files(paths: &Paths) -> io::Result<Vec<&'static str>> {
    let mut removed = Vec::new();
    for name in [plan::FILE].into_iter().chain(plan::WORKING_FILES) {
        if remove_if_there(&paths.state_dir.join(name))? {
            removed.push(name);
        }
    }
    Ok(removed)
}

/// Undoes an arm: Apsis's link first (the commit point; another tool's link is left exactly
/// where it is, and then so is everything else), then the unit, its wants link, the drop-in,
/// the helper copy, `state.json`, the plan and its working files. What was removed, by name,
/// for the journal. The pair the arm kept and rsync's log stay.
///
/// The link's removal is flushed before anything else goes (its folder, then everything
/// once, as the arm does): a power cut seconds after a disarm must not bring the link back
/// over an arm that's still whole (PLAN 6b.5). The timer and the package script both come
/// through here.
///
/// # Errors
///
/// Something that was there couldn't be removed. The link going first means a disarm cut
/// short leaves only leftovers. Or the flush failed: the rest is removed all the same, and
/// the error says that the disarm may not outlast a power cut, and what was removed.
pub fn disarm(paths: &Paths) -> io::Result<Vec<&'static str>> {
    disarm_with(paths, |folder| fs::File::open(folder)?.sync_all())
}

/// [`disarm`], with the flush of the link's folder as `flush` (the tests make it fail).
fn disarm_with(
    paths: &Paths,
    flush: impl Fn(&Path) -> io::Result<()>,
) -> io::Result<Vec<&'static str>> {
    let mut removed = Vec::new();
    let mut unflushed = None;
    match link_state(&paths.link) {
        UpdateLink::Nothing => {}
        _ if is_armed(paths) => {
            fs::remove_file(&paths.link)?;
            removed.push("/system-update");
            unflushed = paths.link.parent().and_then(|folder| flush(folder).err());
            rustix::fs::sync();
        }
        // Another tool's update: not Apsis's to touch (PLAN 6b.6 step 1).
        _ => return Ok(removed),
    }
    removed.extend(remove_arm_files(paths)?);
    removed.extend(remove_plan_files(paths)?);
    match unflushed {
        None => Ok(removed),
        Some(error) => Err(io::Error::other(format!(
            "the removal of /system-update couldn't be flushed to disk ({error}), so a power \
             cut now could bring the link back; removed: {}",
            removed.join(", ")
        ))),
    }
}

/// Leftovers of an arm that was cut short, or of a disarm that was (PLAN 6b.5): without
/// Apsis's link they arm nothing, and go. With the link, it's an arm and stays whole. The plan
/// (`request.json`) isn't touched: whether it's a leftover is the caller's to judge.
///
/// # Errors
///
/// A leftover couldn't be removed.
pub fn clean_leftovers(paths: &Paths) -> io::Result<Vec<&'static str>> {
    if is_armed(paths) {
        return Ok(Vec::new());
    }
    remove_arm_files(paths)
}

/// The helper's start (PLAN 6b.5, 6b.9): with Apsis's link, an arm about to be applied, kept
/// whole. Without it, the arm files are leftovers, and so is a plan (`request.json`) with its
/// filter and note: the helper that held it ready is gone, and a window still at its prompt
/// gets "the preparation is gone" from `RestartToRestore`.
///
/// # Errors
///
/// A leftover couldn't be removed.
pub fn clean_at_start(paths: &Paths) -> io::Result<Vec<&'static str>> {
    if is_armed(paths) {
        return Ok(Vec::new());
    }
    let mut removed = remove_arm_files(paths)?;
    removed.extend(remove_plan_files(paths)?);
    Ok(removed)
}

/// The unit, its wants link, the drop-in, the helper copy and `state.json`, and the wants
/// folder and the drop-in's folder once they're empty. The apply calls it once Apsis's link
/// is gone, or when the link was never Apsis's. Not the plan, its filter or its note:
/// [`clean_leftovers`] calls this right before an arm.
pub fn remove_arm_files(paths: &Paths) -> io::Result<Vec<&'static str>> {
    let mut removed = Vec::new();
    for (path, name) in [
        (&paths.unit, "the unit"),
        (&paths.wants_link, "the wants link"),
        (&paths.drop_in, "the drop-in"),
        (&paths.helper_copy, "the helper copy"),
        (&paths.state_dir.join(STATE_FILE), "state.json"),
    ] {
        if remove_if_there(path)? {
            removed.push(name);
        }
    }
    // The drop-in's folder is Apsis's too: Pop's unit ships none (check 1, 2026-10-02). It
    // stays if another drop-in is in it. The wants folder the same: the arm made it if it
    // wasn't there, and each restore left it behind empty (N3, check 8). It stays if another
    // unit's link is in it.
    for folder in [paths.drop_in.parent(), paths.wants_link.parent()]
        .into_iter()
        .flatten()
    {
        match fs::remove_dir(folder) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(removed)
}

/// `systemd-run` for the disarm timer (PLAN 6b.5): a transient timer in this boot, ten
/// minutes, whose service runs `helper_exe --disarm` and can't start once a shutdown is
/// queued (`Conflicts=shutdown.target`). The packaged helper, not the copy: the copy goes
/// with the arm.
#[must_use]
pub fn disarm_timer_argv(helper_exe: &Path) -> Vec<String> {
    vec![
        "systemd-run".to_owned(),
        "--quiet".to_owned(),
        "--on-active=10min".to_owned(),
        format!("--unit={DISARM_UNIT}"),
        "--timer-property=AccuracySec=1s".to_owned(),
        "--property=Conflicts=shutdown.target".to_owned(),
        "--property=Before=shutdown.target".to_owned(),
        "--description=Apsis: disarm a restore that no restart followed".to_owned(),
        helper_exe.to_string_lossy().into_owned(),
        "--disarm".to_owned(),
    ]
}

/// Stops a disarm timer from an earlier arm, so it can't fire on this one.
#[must_use]
pub fn stop_disarm_timer_argv() -> [&'static str; 3] {
    ["systemctl", "stop", "apsis-disarm.timer"]
}

/// `apsis-helper --disarm`, run by the disarm timer and by the package's `prerm`: disarms if
/// the arm is Apsis's. No D-Bus, and no job lock: the `prerm` holds it while this runs, and
/// only the arm's files are touched, never the backup disk.
pub fn disarm_system() -> io::Result<()> {
    let paths = Paths::system();
    let removed = disarm(&paths)?;
    eprintln!("{}", disarmed_line(&removed));
    Ok(())
}

/// `--disarm`'s line on stderr, with what it removed. It names no caller: in the journal
/// the unit says who ran it, on apt's output the `prerm`'s own line follows.
fn disarmed_line(removed: &[&str]) -> String {
    if removed.is_empty() {
        "apsis-helper: disarm: nothing armed".to_owned()
    } else {
        format!("apsis-helper: disarm: removed {}", removed.join(", "))
    }
}

fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)?;
    fs::File::open(path)?.sync_all()
}

/// `link -> target`, replacing a link already at the name.
fn make_link(link: &Path, target: &Path) -> io::Result<()> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink()) {
        fs::remove_file(link)?;
    }
    symlink(target, link)
}

/// Removes the file or link at `path`; whether there was one.
fn remove_if_there(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use apsis_core::restore::refusal::UpdateLink;
    use apsis_core::restore::state::State as RestoreState;
    use apsis_core::restore::unit;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apsis-arm-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The preparation's filter and note, as a lab has them.
    const FILTER: &str = "P /system-update\n- /home/***\n+ /***\n";
    const NOTE: &str = "Apsis restore: if the computer doesn't start afterwards\n\n1. ...\n";

    /// A root with the state folder, a plan and its two working files in it, and a helper
    /// binary to copy.
    fn lab() -> (PathBuf, Paths, PathBuf) {
        let root = temp("root");
        let paths = Paths::under(&root);
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(paths.state_dir.join("request.json"), "{}").unwrap();
        fs::write(paths.state_dir.join("restore.filter"), FILTER).unwrap();
        fs::write(paths.state_dir.join("restore.note"), NOTE).unwrap();
        let exe = root.join("usr/libexec/apsis-helper");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        (root, paths, exe)
    }

    /// Q3 (owner, 2026-10-04): `--disarm` is run by the disarm timer and by the package's
    /// `prerm`, so its line names neither. In the journal the unit says who ran it; on
    /// apt's output the `prerm`'s own line follows.
    #[test]
    fn the_disarm_line_names_no_caller() {
        assert_eq!(disarmed_line(&[]), "apsis-helper: disarm: nothing armed");
        assert_eq!(
            disarmed_line(&["/system-update", "the unit", "request.json"]),
            "apsis-helper: disarm: removed /system-update, the unit, request.json"
        );
    }

    #[test]
    fn the_paths_are_the_designs_under_the_root() {
        let paths = Paths::under(Path::new("/lab"));
        assert_eq!(
            paths.unit,
            Path::new("/lab").join(unit::UNIT_PATH.trim_start_matches('/'))
        );
        assert_eq!(
            paths.wants_link,
            Path::new("/lab").join(unit::UNIT_WANTS_LINK.trim_start_matches('/'))
        );
        assert_eq!(
            paths.drop_in,
            Path::new("/lab").join(unit::DROP_IN_PATH.trim_start_matches('/'))
        );
        assert_eq!(
            paths.helper_copy,
            Path::new("/lab").join(unit::HELPER_COPY.trim_start_matches('/'))
        );
        assert_eq!(paths.state_dir, Path::new("/lab/var/lib/apsis/restore"));
        assert_eq!(paths.link, Path::new("/lab/system-update"));
        assert_eq!(paths.etc_link, Path::new("/lab/etc/system-update"));
        let system = Paths::system();
        assert_eq!(system.unit, Path::new(unit::UNIT_PATH));
        assert_eq!(system.link, Path::new("/system-update"));
        assert_eq!(system.state_dir, Path::new(apsis_core::restore::file::DIR));
    }

    #[test]
    fn arming_writes_everything_and_the_link_last() {
        let (root, paths, exe) = lab();
        arm(&paths, &exe).unwrap();
        assert_eq!(fs::read_to_string(&paths.unit).unwrap(), unit::unit_text());
        assert_eq!(fs::read_link(&paths.wants_link).unwrap(), paths.unit);
        assert_eq!(
            fs::read_to_string(&paths.drop_in).unwrap(),
            unit::drop_in_text()
        );
        assert_eq!(
            fs::read(&paths.helper_copy).unwrap(),
            fs::read(&exe).unwrap()
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&paths.helper_copy)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            RestoreState::load(&paths.state_dir).unwrap(),
            RestoreState::default()
        );
        assert_eq!(fs::read_link(&paths.link).unwrap(), paths.state_dir);
        assert!(is_armed(&paths));
        // The link is the commit point: when the helper copy can't be made, nothing is armed.
        let (root2, paths2, _) = lab();
        assert!(arm(&paths2, &root2.join("missing")).is_err());
        assert_eq!(link_state(&paths2.link), UpdateLink::Nothing);
        assert!(!is_armed(&paths2));
        // The unit it wrote before failing is a leftover, cleaned before the next arm.
        assert!(paths2.unit.exists());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&root2).unwrap();
    }

    /// What's in the state folder, by name, sorted.
    fn in_state(paths: &Paths) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&paths.state_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn text(paths: &Paths, name: &str) -> String {
        fs::read_to_string(paths.state_dir.join(name)).unwrap()
    }

    /// Pull 1 (PLAN 6b.5): the arm keeps the filter and the note of the plan it arms as
    /// `last-restore.filter` and `last-restore.note`, byte for byte and for root only, in
    /// place of an earlier arm's, and clears rsync's log. The working files stay: the apply
    /// reads the filter.
    #[test]
    fn arming_keeps_the_filter_and_the_note_and_clears_the_log() {
        let (root, paths, exe) = lab();
        for (name, old) in [
            ("last-restore.filter", "an earlier arm's filter\n"),
            ("last-restore.note", "an earlier arm's note\n"),
            ("rsync-log", "the last restore's log\n"),
            ("result.json", "{}"),
        ] {
            fs::write(paths.state_dir.join(name), old).unwrap();
        }
        arm(&paths, &exe).unwrap();
        assert!(is_armed(&paths));
        assert_eq!(text(&paths, "last-restore.filter"), FILTER);
        assert_eq!(text(&paths, "last-restore.note"), NOTE);
        assert_eq!(text(&paths, "restore.filter"), FILTER);
        assert_eq!(text(&paths, "restore.note"), NOTE);
        use std::os::unix::fs::PermissionsExt;
        for name in ["last-restore.filter", "last-restore.note"] {
            let mode = fs::metadata(paths.state_dir.join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
        // The log is gone, the last result stays, and no temporary file is left.
        assert_eq!(
            in_state(&paths),
            [
                "apsis-helper",
                "last-restore.filter",
                "last-restore.note",
                "request.json",
                "restore.filter",
                "restore.note",
                "result.json",
                "state.json",
            ]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// A copy that fails refuses the arm before anything of it is written: no link, no
    /// unit, the pair of the arm before as it was, and the log not cleared.
    #[test]
    fn an_arm_that_cant_keep_the_pair_is_refused_and_changes_nothing() {
        for missing in ["restore.note", "restore.filter"] {
            let (root, paths, exe) = lab();
            for (name, old) in [
                ("last-restore.filter", "an earlier arm's filter\n"),
                ("last-restore.note", "an earlier arm's note\n"),
                ("rsync-log", "the last restore's log\n"),
            ] {
                fs::write(paths.state_dir.join(name), old).unwrap();
            }
            fs::remove_file(paths.state_dir.join(missing)).unwrap();
            let before = in_state(&paths);
            let error = arm(&paths, &exe).unwrap_err().to_string();
            assert!(error.contains("couldn't be kept"), "{missing}: {error}");
            assert_eq!(link_state(&paths.link), UpdateLink::Nothing, "{missing}");
            assert!(
                !paths.unit.exists() && !paths.helper_copy.exists(),
                "{missing}"
            );
            assert_eq!(
                text(&paths, "last-restore.filter"),
                "an earlier arm's filter\n"
            );
            assert_eq!(text(&paths, "last-restore.note"), "an earlier arm's note\n");
            assert_eq!(text(&paths, "rsync-log"), "the last restore's log\n");
            assert_eq!(in_state(&paths), before, "{missing}: no temporary file");
            fs::remove_dir_all(&root).unwrap();
        }
    }

    /// The working files go with the plan in a disarm and at the helper's start without a
    /// link, never with the arm's files alone (`clean_leftovers` runs right before an arm,
    /// on the plan that is about to be armed). The kept pair and the log stay.
    #[test]
    fn the_working_files_go_with_the_plan_and_never_with_the_arm_files() {
        let (root, paths, exe) = lab();
        arm(&paths, &exe).unwrap();
        fs::write(paths.state_dir.join("rsync-log"), "log\n").unwrap();
        // An arm: kept whole at the helper's start.
        assert_eq!(clean_at_start(&paths).unwrap(), Vec::<&str>::new());
        assert!(paths.state_dir.join("restore.filter").exists());
        // The arm's files alone, the link gone: the working files and the plan stay.
        fs::remove_file(&paths.link).unwrap();
        let cleaned = remove_arm_files(&paths).unwrap();
        assert!(
            !cleaned.iter().any(|name| name.starts_with("restore.")),
            "{cleaned:?}"
        );
        assert_eq!(
            in_state(&paths),
            [
                "last-restore.filter",
                "last-restore.note",
                "request.json",
                "restore.filter",
                "restore.note",
                "rsync-log",
            ]
        );
        assert_eq!(clean_leftovers(&paths).unwrap(), Vec::<&str>::new());
        assert!(paths.state_dir.join("restore.note").exists());
        // The helper's start with no link: they go with the plan.
        let cleaned = clean_at_start(&paths).unwrap();
        assert_eq!(cleaned, ["request.json", "restore.filter", "restore.note"]);
        assert_eq!(
            in_state(&paths),
            ["last-restore.filter", "last-restore.note", "rsync-log"]
        );
        // A disarm: the same, after the link and the arm's files.
        let (root2, paths2, exe2) = lab();
        arm(&paths2, &exe2).unwrap();
        fs::write(paths2.state_dir.join("rsync-log"), "log\n").unwrap();
        let removed = disarm(&paths2).unwrap();
        assert_eq!(removed.first(), Some(&"/system-update"));
        assert_eq!(
            removed[removed.len() - 3..],
            ["request.json", "restore.filter", "restore.note"]
        );
        assert_eq!(
            in_state(&paths2),
            ["last-restore.filter", "last-restore.note", "rsync-log"]
        );
        assert_eq!(text(&paths2, "last-restore.filter"), FILTER);
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&root2).unwrap();
    }

    #[test]
    fn disarming_removes_the_link_first_and_then_the_rest() {
        let (root, paths, exe) = lab();
        arm(&paths, &exe).unwrap();
        let removed = disarm(&paths).unwrap();
        assert_eq!(removed.first(), Some(&"/system-update"));
        assert!(removed.contains(&"request.json"), "{removed:?}");
        for path in [
            &paths.link,
            &paths.unit,
            &paths.wants_link,
            &paths.drop_in,
            &paths.helper_copy,
            &paths.state_dir.join("state.json"),
            &paths.state_dir.join("request.json"),
            // The drop-in's folder too: Pop's unit has none of its own (check 1, 2026-10-02).
            paths.drop_in.parent().unwrap(),
            // And the wants folder, empty once Apsis's link is out of it (N3, check 8).
            paths.wants_link.parent().unwrap(),
        ] {
            assert!(fs::symlink_metadata(path).is_err(), "{}", path.display());
        }
        assert!(!is_armed(&paths));
        // Nothing there: nothing removed, no error.
        assert_eq!(disarm(&paths).unwrap(), Vec::<&str>::new());
        // Another tool's link is left exactly where it is, and nothing else goes either.
        symlink("/var/lib/other-tool", &paths.link).unwrap();
        fs::write(&paths.unit, "x").unwrap();
        assert_eq!(disarm(&paths).unwrap(), Vec::<&str>::new());
        assert_eq!(link_state(&paths.link), UpdateLink::Dangling);
        assert!(paths.unit.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// N3 (check 8): each restore left an empty `system-update.target.wants/`. It goes with
    /// Apsis's link when nothing else is in it, and stays, with what's in it, when another
    /// unit's link is there.
    #[test]
    fn the_wants_folder_goes_when_empty_and_stays_with_another_units_link() {
        let (root, paths, exe) = lab();
        let wants = paths.wants_link.parent().unwrap().to_owned();
        arm(&paths, &exe).unwrap();
        fs::remove_file(&paths.link).unwrap();
        remove_arm_files(&paths).unwrap();
        assert!(fs::symlink_metadata(&wants).is_err(), "the empty folder");
        // Its parent is systemd's, and stays.
        assert!(wants.parent().unwrap().is_dir());
        // Another unit's link in it: the folder and that link stay, Apsis's link goes.
        let (root2, paths2, exe2) = lab();
        let wants2 = paths2.wants_link.parent().unwrap().to_owned();
        arm(&paths2, &exe2).unwrap();
        symlink(
            "../other-update.service",
            wants2.join("other-update.service"),
        )
        .unwrap();
        fs::remove_file(&paths2.link).unwrap();
        remove_arm_files(&paths2).unwrap();
        assert!(fs::symlink_metadata(&paths2.wants_link).is_err());
        assert_eq!(
            fs::read_link(wants2.join("other-update.service")).unwrap(),
            std::path::Path::new("../other-update.service")
        );
        // Nothing there at all: no error.
        assert_eq!(remove_arm_files(&paths).unwrap(), Vec::<&str>::new());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&root2).unwrap();
    }

    /// The link's removal is flushed to disk (PLAN 6b.5): a power cut seconds after a
    /// disarm must not bring the link back. If the flush fails, the rest is still removed
    /// and the error says what may not last and what was removed.
    #[test]
    fn a_disarm_that_cant_flush_still_removes_everything_and_says_so() {
        let (root, paths, exe) = lab();
        arm(&paths, &exe).unwrap();
        let flushed = std::cell::RefCell::new(Vec::new());
        let error = disarm_with(&paths, |folder| {
            flushed.borrow_mut().push(folder.to_owned());
            Err(io::Error::other("Input/output error"))
        })
        .unwrap_err()
        .to_string();
        // The folder the link was in, once, after the link was removed.
        assert_eq!(*flushed.borrow(), [root.as_path()]);
        assert!(
            error.contains("/system-update") && error.contains("Input/output error"),
            "{error}"
        );
        assert!(error.contains("request.json"), "what was removed: {error}");
        for path in [
            &paths.link,
            &paths.unit,
            &paths.wants_link,
            &paths.drop_in,
            &paths.helper_copy,
            &paths.state_dir.join("state.json"),
            &paths.state_dir.join("request.json"),
            &paths.state_dir.join("restore.filter"),
            &paths.state_dir.join("restore.note"),
        ] {
            assert!(fs::symlink_metadata(path).is_err(), "{}", path.display());
        }
        // Nothing armed and no link of another tool's: nothing is flushed.
        let none = std::cell::Cell::new(0);
        let count = |_: &std::path::Path| {
            none.set(none.get() + 1);
            Ok(())
        };
        assert_eq!(disarm_with(&paths, count).unwrap(), Vec::<&str>::new());
        symlink("/var/lib/other-tool", &paths.link).unwrap();
        assert_eq!(disarm_with(&paths, count).unwrap(), Vec::<&str>::new());
        assert_eq!(none.get(), 0);
        // The real flush works on a real folder: the disarm of an arm is `Ok`.
        let (root2, paths2, exe2) = lab();
        arm(&paths2, &exe2).unwrap();
        assert_eq!(disarm(&paths2).unwrap().first(), Some(&"/system-update"));
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&root2).unwrap();
    }

    #[test]
    fn leftovers_without_the_link_are_cleaned_and_an_arm_is_kept() {
        let (root, paths, exe) = lab();
        arm(&paths, &exe).unwrap();
        // With Apsis's link: an arm, kept whole.
        assert_eq!(clean_leftovers(&paths).unwrap(), Vec::<&str>::new());
        assert!(paths.unit.exists() && is_armed(&paths));
        // Without it: the unit, the wants link, the drop-in, the helper copy and state.json
        // go; the plan (request.json) is the caller's to judge.
        fs::remove_file(&paths.link).unwrap();
        let cleaned = clean_leftovers(&paths).unwrap();
        assert_eq!(cleaned.len(), 5, "{cleaned:?}");
        for path in [
            &paths.unit,
            &paths.wants_link,
            &paths.drop_in,
            &paths.helper_copy,
        ] {
            assert!(fs::symlink_metadata(path).is_err(), "{}", path.display());
        }
        assert!(fs::symlink_metadata(paths.state_dir.join("state.json")).is_err());
        assert!(paths.state_dir.join("request.json").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// The helper's start (PLAN 6b.5, 6b.9): a plan with no link is a leftover of a helper
    /// that died while it was ready, and goes with the other leftovers; an arm stays whole.
    #[test]
    fn at_start_a_plan_without_the_link_is_a_leftover() {
        let (root, paths, exe) = lab();
        arm(&paths, &exe).unwrap();
        assert_eq!(clean_at_start(&paths).unwrap(), Vec::<&str>::new());
        assert!(is_armed(&paths) && paths.state_dir.join("request.json").exists());
        fs::remove_file(&paths.link).unwrap();
        let cleaned = clean_at_start(&paths).unwrap();
        assert!(cleaned.contains(&"request.json"), "{cleaned:?}");
        assert_eq!(cleaned.len(), 8, "{cleaned:?}");
        assert!(!paths.state_dir.join("request.json").exists());
        assert!(!paths.unit.exists());
        // Nothing at all: nothing to say.
        assert_eq!(clean_at_start(&paths).unwrap(), Vec::<&str>::new());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn what_is_at_a_link_name_is_told_apart() {
        let dir = temp("link");
        assert_eq!(link_state(&dir.join("none")), UpdateLink::Nothing);
        fs::write(dir.join("file"), "").unwrap();
        assert_eq!(link_state(&dir.join("file")), UpdateLink::Other);
        fs::create_dir(dir.join("folder")).unwrap();
        assert_eq!(link_state(&dir.join("folder")), UpdateLink::Other);
        symlink(dir.join("folder"), dir.join("link")).unwrap();
        assert_eq!(link_state(&dir.join("link")), UpdateLink::Link);
        symlink(dir.join("nowhere"), dir.join("dangling")).unwrap();
        assert_eq!(link_state(&dir.join("dangling")), UpdateLink::Dangling);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The disarm timer (PLAN 6b.5): transient, ten minutes, in this boot only, its service
    /// can't run once a shutdown is queued, and it runs the packaged helper (the copy goes
    /// with the arm).
    #[test]
    fn the_disarm_timer_is_transient_and_yields_to_a_shutdown() {
        let argv = disarm_timer_argv(Path::new("/usr/libexec/apsis-helper"));
        assert_eq!(argv[0], "systemd-run");
        assert!(argv.contains(&"--on-active=10min".to_owned()), "{argv:?}");
        assert!(argv.contains(&format!("--unit={DISARM_UNIT}")), "{argv:?}");
        assert!(
            argv.contains(&"--property=Conflicts=shutdown.target".to_owned()),
            "{argv:?}"
        );
        let tail: Vec<&str> = argv
            .iter()
            .rev()
            .take(2)
            .rev()
            .map(String::as_str)
            .collect();
        assert_eq!(tail, ["/usr/libexec/apsis-helper", "--disarm"]);
        assert!(
            !argv.iter().any(|a| a.contains("/var/lib/apsis")),
            "not the copy"
        );
        assert_eq!(
            stop_disarm_timer_argv(),
            ["systemctl", "stop", "apsis-disarm.timer"]
        );
    }
}
