// SPDX-License-Identifier: GPL-3.0-only

//! Arming the next boot and undoing it (PLAN 6b.5, 6b.6): the unit, its wants link, the
//! drop-in, the helper copy and `state.json` first, `sync`, then `/system-update` last; a
//! disarm removes the link first. Leftovers without the link arm nothing and are cleaned.
//! The disarm timer: a transient systemd timer in the current boot that runs
//! `apsis-helper --disarm` after ten minutes without a restart.

use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use apsis_core::restore::refusal::UpdateLink;
use apsis_core::restore::state::{STATE_FILE, State as RestoreState};
use apsis_core::restore::{file, plan, unit};

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

/// Arms the next boot, in PLAN 6b.5's order: the unit, its wants link, the drop-in, the
/// helper copy (`helper_exe`, the packaged helper, copied so a restored snapshot can't take
/// it away mid-restore), a fresh `state.json`, then everything synced to disk, then the link
/// last. Nothing before the link arms anything; a failure part-way leaves leftovers for
/// [`clean_leftovers`].
///
/// # Errors
///
/// A file couldn't be written; the link isn't made then.
pub fn arm(paths: &Paths, helper_exe: &Path) -> io::Result<()> {
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

/// Undoes an arm: Apsis's link first (the commit point; another tool's link is left exactly
/// where it is, and then so is everything else), then the unit, its wants link, the drop-in,
/// the helper copy, `state.json` and the plan. What was removed, by name, for the journal.
///
/// # Errors
///
/// Something that was there couldn't be removed. The link going first means a disarm cut
/// short leaves only leftovers.
pub fn disarm(paths: &Paths) -> io::Result<Vec<&'static str>> {
    let mut removed = Vec::new();
    match link_state(&paths.link) {
        UpdateLink::Nothing => {}
        _ if is_armed(paths) => {
            fs::remove_file(&paths.link)?;
            removed.push("/system-update");
        }
        // Another tool's update: not Apsis's to touch (PLAN 6b.6 step 1).
        _ => return Ok(removed),
    }
    removed.extend(remove_arm_files(paths)?);
    if remove_if_there(&paths.state_dir.join(plan::FILE))? {
        removed.push("request.json");
    }
    Ok(removed)
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
/// whole. Without it, the arm files are leftovers, and so is a plan (`request.json`): the
/// helper that held it ready is gone, and a window still at its prompt gets "the preparation
/// is gone" from `RestartToRestore`.
///
/// # Errors
///
/// A leftover couldn't be removed.
pub fn clean_at_start(paths: &Paths) -> io::Result<Vec<&'static str>> {
    if is_armed(paths) {
        return Ok(Vec::new());
    }
    let mut removed = remove_arm_files(paths)?;
    if remove_if_there(&paths.state_dir.join(plan::FILE))? {
        removed.push("request.json");
    }
    Ok(removed)
}

/// The unit, its wants link, the drop-in, the helper copy and `state.json`. The apply calls
/// it once Apsis's link is gone, or when the link was never Apsis's.
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
    // stays if another drop-in is in it.
    if let Some(folder) = paths.drop_in.parent() {
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

/// `apsis-helper --disarm`, run by the timer: disarms if the arm is Apsis's. No D-Bus.
pub fn disarm_from_timer() -> io::Result<()> {
    let paths = Paths::system();
    let removed = disarm(&paths)?;
    if removed.is_empty() {
        eprintln!("apsis-helper: disarm timer: nothing armed");
    } else {
        eprintln!(
            "apsis-helper: disarm timer: no restart in 10 minutes; removed {}",
            removed.join(", ")
        );
    }
    Ok(())
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

    /// A root with the state folder, a plan in it, and a helper binary to copy.
    fn lab() -> (PathBuf, Paths, PathBuf) {
        let root = temp("root");
        let paths = Paths::under(&root);
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(paths.state_dir.join("request.json"), "{}").unwrap();
        let exe = root.join("usr/libexec/apsis-helper");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "#!/bin/sh\n").unwrap();
        (root, paths, exe)
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
        assert_eq!(cleaned.len(), 6, "{cleaned:?}");
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
