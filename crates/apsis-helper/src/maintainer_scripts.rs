// SPDX-License-Identifier: GPL-3.0-only

//! The .deb's maintainer scripts (`resources/deb/`), run and not only read (PLAN 6b.9; fix 1
//! and 1b).
//!
//! Each script names its paths once, as variables at its top. A test runs a copy in which
//! only those lines are changed, to paths under a temp folder, with a `PATH` that holds
//! nothing but a fake `systemctl`, a fake `busctl` and links to the few real tools the
//! scripts use: a command the harness doesn't know isn't found, instead of run.
//!
//! Before any copy runs, the harness refuses ([`Lab::run`]): as root (`postrm purge` removes
//! folders); a copy that still names a path outside the temp folder; and a `systemctl`, a
//! `busctl` or a helper that isn't its own fake.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use apsis_core::restore::unit::{DROP_IN_PATH, UNIT_PATH, UNIT_WANTS_LINK};
use apsis_core::restore::{file, plan};

use crate::arm;
use crate::state::JOB_LOCK;
use crate::state::tests::{Held, flock_exit, hold};

const PRERM: &str = include_str!("../../../resources/deb/prerm");
const POSTINST: &str = include_str!("../../../resources/deb/postinst");
const POSTRM: &str = include_str!("../../../resources/deb/postrm");
/// Where the .deb puts the helper (`[package.metadata.deb]`).
const DEB_MANIFEST: &str = include_str!("../../apsis/Cargo.toml");

/// The real tools the scripts and the fakes may run. Nothing else is on the `PATH`.
const TOOLS: [&str; 6] = ["flock", "mkdir", "readlink", "rm", "rmdir", "true"];

/// How every fake begins: how the harness knows a command is one of its own.
const FAKE: &str = "#!/bin/sh\n# an Apsis test's fake\n";

/// Absolute names a copy may still hold. Nothing is written to either.
const ALLOWED: [&str; 2] = ["/dev/null", "/org/freedesktop/DBus"];

/// What the `prerm` runs for: dpkg's `remove`, `upgrade`, `deconfigure`, and the new
/// package's `failed-upgrade`.
const OPERATIONS: [&str; 4] = ["remove", "upgrade", "deconfigure", "failed-upgrade"];

const REFUSAL: &str = "apsis: an Apsis job is running; try again when it has finished\n";
const CANCELLED: &str = "apsis: the restore that was waiting for a restart is cancelled\n";

/// What the fake helper does with `--disarm`.
#[derive(Clone, Copy)]
enum Disarm {
    /// Removes the link and exits 0.
    Works,
    /// Leaves the link and exits 1.
    Fails,
    /// Removes the link and exits 1: the flush failed.
    Unflushed,
}

const DISARM_FAILED: &str = "apsis-helper: disarm: Permission denied (os error 13)\n";
const DISARM_UNFLUSHED: &str =
    "apsis-helper: disarm: the removal of /system-update couldn't be flushed to disk\n";

/// The file `name` from the tests' own `PATH`.
fn real(name: &str) -> PathBuf {
    let path = std::env::var_os("PATH").expect("a PATH");
    std::env::split_paths(&path)
        .map(|folder| folder.join(name))
        .find(|file| file.is_file())
        .unwrap_or_else(|| panic!("the script tests need {name}"))
}

/// `sh`, and `dash` (Debian's `/bin/sh`) when it's installed.
fn shells() -> Vec<PathBuf> {
    let mut shells = vec![real("sh")];
    let path = std::env::var_os("PATH").expect("a PATH");
    shells.extend(
        std::env::split_paths(&path)
            .map(|folder| folder.join("dash"))
            .find(|file| file.is_file()),
    );
    shells
}

fn refuse_root(is_root: bool) -> Result<(), String> {
    if is_root {
        return Err("not run as root: the scripts remove files and folders".to_owned());
    }
    Ok(())
}

/// `NAME=/an/absolute/path`, alone on its line: a path the script names once.
fn variable(line: &str) -> Option<(&str, &str)> {
    let (name, path) = line.split_once('=')?;
    let named = !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c == '_');
    let plain = path.starts_with('/')
        && !path.contains(|c: char| c.is_whitespace() || "\"'$`\\;".contains(c));
    (named && plain).then_some((name, path))
}

/// `line` without its comment.
fn code_of(line: &str) -> &str {
    if line.trim_start().starts_with('#') {
        return "";
    }
    line.split_once(" #").map_or(line, |(code, _)| code)
}

/// Every absolute path `code` names: a `/` at the start of a word, up to the word's end.
fn absolute_names(code: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut before = ' ';
    let mut skip_to = 0;
    for (at, c) in code.char_indices() {
        if at >= skip_to && c == '/' && (before.is_whitespace() || "=\"'><(:".contains(before)) {
            let rest = &code[at..];
            let end = rest
                .find(|c: char| c.is_whitespace() || "\"');".contains(c))
                .unwrap_or(rest.len());
            names.push(&rest[..end]);
            skip_to = at + end;
        }
        before = c;
    }
    names
}

/// The path guard: outside its comments, `copy` names no absolute path but ones under
/// `root` (and [`ALLOWED`]), and none that climbs out with `..`.
fn only_under(copy: &str, root: &Path) -> Result<(), String> {
    for line in copy.lines() {
        for name in absolute_names(code_of(line)) {
            let path = Path::new(name);
            let inside = path.starts_with(root)
                && !path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir);
            if !inside && !ALLOWED.contains(&name) {
                return Err(format!(
                    "the copy names {name}, outside the test's folder, in: {line}"
                ));
            }
        }
    }
    Ok(())
}

/// The value of `NAME=` in `script`.
fn assigned<'a>(script: &'a str, name: &str) -> &'a str {
    script
        .lines()
        .filter_map(variable)
        .find_map(|(found, path)| (found == name).then_some(path))
        .unwrap_or_else(|| panic!("the script has no {name}="))
}

/// A temp folder that stands in for `/`, with the fakes.
struct Lab {
    root: PathBuf,
}

impl Drop for Lab {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

impl Lab {
    /// Systemd runs and the helper is active; nothing is armed; no job runs.
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "apsis-deb-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&root);
        assert!(
            !root.to_string_lossy().contains(['\'', '\n']),
            "the temp folder's name can't be quoted"
        );
        let lab = Self { root };
        fs::create_dir_all(lab.bin()).unwrap();
        for tool in TOOLS {
            symlink(real(tool), lab.bin().join(tool)).unwrap();
        }
        lab.fake(
            &lab.bin().join("systemctl"),
            &format!(
                "echo \"systemctl $*\" >> '{calls}'\n\
                 case \"$*\" in\n\
                 \x20   \"is-active --quiet apsis-helper.service\")\n\
                 \x20       [ ! -e '{inactive}' ] || exit 3 ;;\n\
                 \x20   \"stop apsis-helper.service\")\n\
                 \x20       rc=0; flock -n -E 75 '{lock}' true || rc=$?\n\
                 \x20       [ \"$rc\" -ne 75 ] || echo \"  the job lock is held\" >> '{calls}' ;;\n\
                 \x20   \"stop apsis-disarm.timer\")\n\
                 \x20       [ ! -e '{unit}' ] || echo \"  the unit is still there\" >> '{calls}' ;;\n\
                 esac\n\
                 exit 0\n",
                calls = lab.calls_file().display(),
                inactive = lab.root.join("helper-inactive").display(),
                lock = lab.lock().display(),
                unit = lab.at(UNIT_PATH).display(),
            ),
        );
        lab.fake(
            &lab.bin().join("busctl"),
            &format!("echo \"busctl $*\" >> '{}'\n", lab.calls_file().display()),
        );
        fs::create_dir_all(lab.at("/run/systemd/system")).unwrap();
        lab
    }

    /// `absolute` under the lab's root.
    fn at(&self, absolute: &str) -> PathBuf {
        self.root.join(absolute.trim_start_matches('/'))
    }

    /// The only folder on the scripts' `PATH`.
    fn bin(&self) -> PathBuf {
        self.root.join("test-bin")
    }

    fn calls_file(&self) -> PathBuf {
        self.root.join("calls")
    }

    fn lock(&self) -> PathBuf {
        self.at(JOB_LOCK)
    }

    fn link(&self) -> PathBuf {
        self.at("/system-update")
    }

    fn helper_file(&self) -> PathBuf {
        self.at("/usr/libexec/apsis-helper")
    }

    fn fake(&self, file: &Path, body: &str) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, format!("{FAKE}{body}")).unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The fake helper: records its arguments, then does `disarm`.
    fn helper(&self, disarm: Disarm) {
        let remove = format!("rm -f '{}'\n", self.link().display());
        let does = match disarm {
            Disarm::Works => format!("{remove}exit 0\n"),
            Disarm::Fails => format!("printf '%s' \"{DISARM_FAILED}\" >&2\nexit 1\n"),
            Disarm::Unflushed => {
                format!("{remove}printf '%s' \"{DISARM_UNFLUSHED}\" >&2\nexit 1\n")
            }
        };
        self.fake(
            &self.helper_file(),
            &format!(
                "echo \"apsis-helper $*\" >> '{}'\n{does}",
                self.calls_file().display()
            ),
        );
    }

    /// Apsis's link, as the arm makes it: to the state folder, with a plan in it.
    fn arm(&self) {
        let state = self.at(file::DIR);
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join(plan::FILE), "{}\n").unwrap();
        symlink(&state, self.link()).unwrap();
    }

    fn is_armed(&self) -> bool {
        fs::read_link(self.link()).is_ok_and(|target| target == self.at(file::DIR))
    }

    /// Another tool's offline update.
    fn other_tools_link(&self) {
        symlink("/var/lib/other-tool", self.link()).unwrap();
    }

    /// The unit, its wants link and the drop-in, as the arm writes them, and Apsis's
    /// config.
    fn arm_files(&self) {
        for path in [UNIT_PATH, DROP_IN_PATH, apsis_core::config::CONFIG_PATH] {
            fs::create_dir_all(self.at(path).parent().unwrap()).unwrap();
            fs::write(self.at(path), "[Unit]\n").unwrap();
        }
        fs::create_dir_all(self.at(UNIT_WANTS_LINK).parent().unwrap()).unwrap();
        symlink(self.at(UNIT_PATH), self.at(UNIT_WANTS_LINK)).unwrap();
    }

    /// Something is at `absolute`'s name (a link to nothing counts).
    fn has(&self, absolute: &str) -> bool {
        fs::symlink_metadata(self.at(absolute)).is_ok()
    }

    /// Holds the job lock as the helper's job does, until the result drops.
    fn job(&self) -> Held {
        hold(&self.lock())
    }

    fn path_env(&self) -> String {
        self.bin().to_string_lossy().into_owned()
    }

    /// `script` with each path variable moved under the root, if it passes the path guard.
    fn copy_of(&self, script: &str) -> Result<String, String> {
        let mut moved = 0;
        let copy: String = script
            .lines()
            .map(|line| match variable(line) {
                Some((name, path)) => {
                    moved += 1;
                    format!("{name}='{}'\n", self.at(path).display())
                }
                None => format!("{line}\n"),
            })
            .collect();
        if moved == 0 {
            return Err("the script names no path as a variable".to_owned());
        }
        only_under(&copy, &self.root)?;
        Ok(copy)
    }

    /// With `path` as the `PATH`, `systemctl` and `busctl` are this lab's fakes.
    fn fakes_resolve(&self, shell: &Path, path: &str) -> Result<(), String> {
        for command in ["systemctl", "busctl"] {
            let found = Command::new(shell)
                .args(["-c", &format!("command -v {command}")])
                .env_clear()
                .env("PATH", path)
                .output()
                .map_err(|error| format!("{}: {error}", shell.display()))?;
            let found = String::from_utf8_lossy(&found.stdout).trim().to_owned();
            let fake = self.bin().join(command);
            if Path::new(&found) != fake {
                return Err(format!(
                    "{command} is {found:?} there, not the fake {}",
                    fake.display()
                ));
            }
            self.is_fake(&fake)?;
        }
        Ok(())
    }

    fn is_fake(&self, file: &Path) -> Result<(), String> {
        if fs::read_to_string(file).is_ok_and(|text| text.starts_with(FAKE)) {
            return Ok(());
        }
        Err(format!("{} isn't a fake of this test", file.display()))
    }

    /// The helper `copy` would run is this lab's fake.
    fn helper_is_fake(&self, copy: &str) -> Result<(), String> {
        let Some(line) = copy.lines().find(|line| line.starts_with("HELPER=")) else {
            return Ok(());
        };
        let fake = self.helper_file();
        if line != format!("HELPER='{}'", fake.display()) {
            return Err(format!("the copy's helper isn't the fake: {line}"));
        }
        self.is_fake(&fake)
    }

    /// Runs a copy of `script` with `argument`, as dpkg runs the script.
    ///
    /// # Errors
    ///
    /// Nothing ran: root, a path outside the lab, or a command that isn't a fake.
    fn run(&self, script: &str, shell: &Path, argument: &str) -> Result<Output, String> {
        refuse_root(rustix::process::geteuid().is_root())?;
        let copy = self.copy_of(script)?;
        self.fakes_resolve(shell, &self.path_env())?;
        self.helper_is_fake(&copy)?;
        let file = self.root.join("script");
        fs::write(&file, copy).unwrap();
        Command::new(shell)
            .arg(&file)
            .arg(argument)
            .env_clear()
            .env("PATH", self.path_env())
            .current_dir(&self.root)
            .output()
            .map_err(|error| format!("{}: {error}", shell.display()))
    }

    /// What the script called, in order: each fake's name and arguments, and what the fake
    /// `systemctl` found.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.calls_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The `prerm`'s last step, as the fake `systemctl` records it: asked whether the helper
/// runs, then the stop, during which the job lock is found held.
const STOPPED: [&str; 3] = [
    "systemctl is-active --quiet apsis-helper.service",
    "systemctl stop apsis-helper.service",
    "  the job lock is held",
];

/// Test 1 (fix 1). While a job holds the lock the `prerm` refuses, for each of its four
/// operations: exit 75, the one line, and nothing else happens.
#[test]
fn prerm_refuses_with_75_while_a_job_holds_the_lock() {
    for shell in shells() {
        for operation in OPERATIONS {
            let lab = Lab::new("refuse");
            lab.helper(Disarm::Works);
            lab.arm();
            let _job = lab.job();
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(75), "{operation}");
            assert_eq!(stderr(&output), REFUSAL, "{operation}");
            assert_eq!(stdout(&output), "", "{operation}");
            assert_eq!(
                lab.calls(),
                [""; 0],
                "{operation}: nothing is stopped, the helper isn't run"
            );
            assert!(lab.is_armed(), "{operation}: the link is untouched");
        }
    }
}

/// Test 2 (fix 1). With the lock free the `prerm` takes it and still holds it while the
/// helper is stopped, so no job can begin between the test and the stop.
#[test]
fn prerm_holds_the_lock_through_the_stop() {
    for shell in shells() {
        for operation in OPERATIONS {
            let lab = Lab::new("stop");
            lab.helper(Disarm::Works);
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(0), "{operation}");
            assert_eq!(lab.calls(), STOPPED, "{operation}");
            assert_eq!(stdout(&output) + &stderr(&output), "", "{operation}");
            assert_eq!(flock_exit(&lab.lock()), 0, "free once the script is gone");
        }
    }
}

/// Round c. The lock file the `prerm` makes is root's alone (0600), whatever the umask it
/// was started with: `flock` works on a read-only descriptor, so a file others could open
/// would let any user hold the lock.
#[test]
fn prerm_makes_the_lock_file_for_root_alone() {
    for shell in shells() {
        let lab = Lab::new("lock-mode");
        lab.helper(Disarm::Works);
        let output = lab.run(PRERM, &shell, "remove").unwrap();
        assert_eq!(output.status.code(), Some(0));
        let mode = fs::metadata(lab.lock()).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o600);
    }
}

/// Test 3 (fix 1b) and addition A. Apsis's link: the helper disarms, the script says so in
/// one line, the timer and then the helper are stopped. Another tool's link, or none: the
/// helper isn't run and the link is as it was.
#[test]
fn prerm_disarms_apsis_link_only() {
    for shell in shells() {
        for operation in OPERATIONS {
            let lab = Lab::new("disarm");
            lab.helper(Disarm::Works);
            lab.arm();
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(0), "{operation}");
            assert_eq!(stdout(&output), CANCELLED, "{operation}");
            assert_eq!(stderr(&output), "", "{operation}");
            let mut expected = vec!["apsis-helper --disarm", "systemctl stop apsis-disarm.timer"];
            expected.extend(STOPPED);
            assert_eq!(lab.calls(), expected, "{operation}");
            assert!(!lab.has("/system-update"), "{operation}");

            let lab = Lab::new("other-link");
            lab.helper(Disarm::Works);
            lab.other_tools_link();
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(0), "{operation}");
            assert_eq!(stdout(&output) + &stderr(&output), "", "{operation}");
            assert_eq!(lab.calls(), STOPPED, "{operation}: the helper isn't run");
            assert_eq!(
                fs::read_link(lab.link()).unwrap(),
                Path::new("/var/lib/other-tool"),
                "{operation}"
            );

            let lab = Lab::new("no-link");
            lab.helper(Disarm::Works);
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(0), "{operation}");
            assert_eq!(lab.calls(), STOPPED, "{operation}: the helper isn't run");
        }
    }
}

/// Test 4 (the open point, option (e)). The link is still Apsis's after `--disarm`: the
/// script fails with the commands to run by hand, and stops neither the timer (it tries
/// again) nor the helper.
#[test]
fn prerm_refuses_when_the_link_survives_the_disarm() {
    for shell in shells() {
        for operation in OPERATIONS {
            let lab = Lab::new("disarm-fails");
            lab.helper(Disarm::Fails);
            lab.arm();
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(1), "{operation}");
            assert_eq!(
                stderr(&output),
                format!(
                    "{DISARM_FAILED}\
                     apsis: a restore is armed and couldn't be disarmed; as root, run:\n\
                     \x20 rm {}\n\
                     \x20 systemctl stop apsis-disarm.timer\n",
                    lab.link().display()
                ),
                "{operation}"
            );
            assert_eq!(stdout(&output), "", "{operation}");
            assert_eq!(lab.calls(), ["apsis-helper --disarm"], "{operation}");
            assert!(lab.is_armed(), "{operation}");
        }
    }
}

/// Test 5 (option (e)). `--disarm` exits 1 but the link is gone (the flush failed): nothing
/// can be armed any more, so the script warns in one line and goes on.
#[test]
fn prerm_goes_on_when_the_disarm_failed_but_the_link_is_gone() {
    for shell in shells() {
        for operation in OPERATIONS {
            let lab = Lab::new("disarm-unflushed");
            lab.helper(Disarm::Unflushed);
            lab.arm();
            let output = lab.run(PRERM, &shell, operation).unwrap();
            assert_eq!(output.status.code(), Some(0), "{operation}");
            assert_eq!(
                stderr(&output),
                format!(
                    "{DISARM_UNFLUSHED}\
                     apsis: the restore is disarmed, but writing that to disk may have failed;\n\
                     \x20 check that {} is gone after the next restart\n",
                    lab.link().display()
                ),
                "{operation}"
            );
            assert_eq!(
                stdout(&output),
                "",
                "{operation}: not the line of a clean disarm"
            );
            let mut expected = vec!["apsis-helper --disarm", "systemctl stop apsis-disarm.timer"];
            expected.extend(STOPPED);
            assert_eq!(lab.calls(), expected, "{operation}");
        }
    }
}

/// Test 6. Any other argument: nothing, not even a look at the lock.
#[test]
fn prerm_ignores_other_arguments() {
    for shell in shells() {
        for argument in ["anything", "abort-upgrade", ""] {
            let lab = Lab::new("other-argument");
            lab.helper(Disarm::Works);
            lab.arm();
            let _job = lab.job();
            let output = lab.run(PRERM, &shell, argument).unwrap();
            assert_eq!(output.status.code(), Some(0), "{argument:?}");
            assert_eq!(stdout(&output) + &stderr(&output), "", "{argument:?}");
            assert_eq!(lab.calls(), [""; 0], "{argument:?}");
            assert!(lab.is_armed(), "{argument:?}");
        }
    }
}

/// As before fix 1: without systemd (a chroot, an image build) nothing is asked of it, and a
/// helper that isn't running isn't stopped. The lock is taken all the same.
#[test]
fn prerm_stops_only_a_running_helper_and_only_with_systemd() {
    for shell in shells() {
        let lab = Lab::new("no-systemd");
        lab.helper(Disarm::Works);
        fs::remove_dir(lab.at("/run/systemd/system")).unwrap();
        let output = lab.run(PRERM, &shell, "remove").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(lab.calls(), [""; 0]);
        assert!(lab.lock().exists(), "the lock was taken");

        let lab = Lab::new("helper-inactive");
        lab.helper(Disarm::Works);
        fs::write(lab.root.join("helper-inactive"), "").unwrap();
        let output = lab.run(PRERM, &shell, "upgrade").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(lab.calls(), STOPPED[..1]);
    }
}

/// Test 7. What dpkg runs after a refused `prerm`: the `postinst`'s abort cases succeed and
/// do nothing, so the old package stays configured.
#[test]
fn postinst_succeeds_on_the_abort_cases() {
    for shell in shells() {
        for argument in ["abort-upgrade", "abort-remove", "abort-deconfigure"] {
            let lab = Lab::new("abort");
            lab.arm();
            let output = lab.run(POSTINST, &shell, argument).unwrap();
            assert_eq!(output.status.code(), Some(0), "{argument}");
            assert_eq!(stdout(&output) + &stderr(&output), "", "{argument}");
            assert_eq!(lab.calls(), [""; 0], "{argument}");
            assert!(lab.is_armed(), "{argument}");
        }
    }
}

/// As before: `postinst configure` reloads systemd and the bus, and without systemd only the
/// bus is asked.
#[test]
fn postinst_configure_reloads_systemd_and_the_bus() {
    for shell in shells() {
        let lab = Lab::new("configure");
        let output = lab.run(POSTINST, &shell, "configure").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(lab.calls(), FORGOTTEN[2..]);

        let lab = Lab::new("configure-no-systemd");
        fs::remove_dir(lab.at("/run/systemd/system")).unwrap();
        let output = lab.run(POSTINST, &shell, "configure").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(lab.calls(), FORGOTTEN[3..]);
    }
}

/// What `postrm remove` and `purge` ask of systemd and the bus, in order: the timer is
/// stopped while the unit file is still there, then the reloads.
const FORGOTTEN: [&str; 4] = [
    "systemctl stop apsis-disarm.timer",
    "  the unit is still there",
    "systemctl daemon-reload",
    "busctl call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig",
];

/// Test 8 (fix 1b, Q4). `postrm remove` stops the disarm timer, then removes Apsis's link,
/// the unit, its wants link and the drop-in (and their folders when empty). The state
/// folder and the config stay (purge's), and so does another tool's link.
#[test]
fn postrm_remove_removes_apsis_link_and_the_unit_files() {
    let folders = [
        "/etc/systemd/system/pop-upgrade-init.service.d",
        "/etc/systemd/system/system-update.target.wants",
    ];
    for shell in shells() {
        let lab = Lab::new("postrm-remove");
        lab.arm();
        lab.arm_files();
        let output = lab.run(POSTRM, &shell, "remove").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(stdout(&output) + &stderr(&output), "");
        assert_eq!(lab.calls(), FORGOTTEN);
        for gone in ["/system-update", UNIT_PATH, UNIT_WANTS_LINK, DROP_IN_PATH] {
            assert!(!lab.has(gone), "{gone} is still there");
        }
        for folder in folders {
            assert!(!lab.has(folder), "{folder}, empty, is still there");
        }
        assert!(lab.at(file::DIR).join(plan::FILE).exists(), "purge's");
        assert!(lab.has(apsis_core::config::CONFIG_PATH), "purge's");

        // Another tool's link stays, and so does a folder that holds something else.
        let lab = Lab::new("postrm-remove-other");
        lab.other_tools_link();
        lab.arm_files();
        for folder in folders {
            fs::write(lab.at(folder).join("another"), "").unwrap();
        }
        let output = lab.run(POSTRM, &shell, "remove").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            fs::read_link(lab.link()).unwrap(),
            Path::new("/var/lib/other-tool")
        );
        for gone in [UNIT_PATH, UNIT_WANTS_LINK, DROP_IN_PATH] {
            assert!(!lab.has(gone), "{gone} is still there");
        }
        for folder in folders {
            assert!(lab.at(folder).join("another").exists(), "{folder}");
        }

        // Nothing there at all (the usual remove): no error.
        let lab = Lab::new("postrm-remove-nothing");
        let output = lab.run(POSTRM, &shell, "remove").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(stdout(&output) + &stderr(&output), "");
    }
}

/// `postrm purge` (PLAN 6b.13 step 3 item 10): what `remove` removes, and the config and
/// the restore's state folder too. `/system-update` only when it's Apsis's link. Snapshots
/// are never touched.
#[test]
fn purge_removes_the_restores_leftovers_and_only_apsis_link() {
    for shell in shells() {
        let lab = Lab::new("purge");
        lab.arm();
        lab.arm_files();
        let output = lab.run(POSTRM, &shell, "purge").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(lab.calls(), FORGOTTEN);
        for gone in [
            "/system-update",
            UNIT_PATH,
            UNIT_WANTS_LINK,
            DROP_IN_PATH,
            "/etc/systemd/system/pop-upgrade-init.service.d",
            "/etc/apsis",
            "/var/lib/apsis",
        ] {
            assert!(!lab.has(gone), "{gone} is still there");
        }
        assert!(lab.has("/etc/systemd/system"), "not more than Apsis's");
        assert!(lab.has("/var/lib"), "not more than Apsis's");

        // After a remove, as dpkg runs it: nothing left to do is no error.
        let output = lab.run(POSTRM, &shell, "purge").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(stdout(&output) + &stderr(&output), "");

        let lab = Lab::new("purge-other");
        lab.other_tools_link();
        let output = lab.run(POSTRM, &shell, "purge").unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(
            fs::read_link(lab.link()).unwrap(),
            Path::new("/var/lib/other-tool")
        );
    }
    assert!(!POSTRM.contains("timeshift"), "snapshots are never touched");
}

/// An upgrade's `postrm` (and dpkg's other calls) removes nothing: the arm, if any, is the
/// new package's to keep.
#[test]
fn postrm_does_nothing_on_an_upgrade() {
    for shell in shells() {
        for argument in [
            "upgrade",
            "failed-upgrade",
            "abort-install",
            "abort-upgrade",
            "disappear",
        ] {
            let lab = Lab::new("postrm-upgrade");
            lab.arm();
            lab.arm_files();
            let output = lab.run(POSTRM, &shell, argument).unwrap();
            assert_eq!(output.status.code(), Some(0), "{argument}");
            assert_eq!(lab.calls(), [""; 0], "{argument}");
            assert!(lab.is_armed(), "{argument}");
            for kept in [UNIT_PATH, UNIT_WANTS_LINK, DROP_IN_PATH] {
                assert!(lab.has(kept), "{argument}: {kept}");
            }
        }
    }
}

/// Test 9. The three scripts as they are in the package parse, with `sh` and with `dash`.
#[test]
fn maintainer_scripts_parse() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/deb");
    for shell in shells() {
        for script in ["prerm", "postinst", "postrm"] {
            let parsed = Command::new(&shell)
                .arg("-n")
                .arg(folder.join(script))
                .output()
                .unwrap();
            assert!(
                parsed.status.success(),
                "{script} with {}: {}",
                shell.display(),
                stderr(&parsed)
            );
        }
    }
}

/// Addition B. The paths the scripts name are the helper's own: the link and the folder it
/// must point at, the job lock, the helper, the unit's files, and what purge removes. And
/// the comparison that decides whether a link is Apsis's uses the two variables.
#[test]
fn the_scripts_paths_are_the_helpers_constants() {
    let system = arm::Paths::system();
    for script in [PRERM, POSTRM] {
        assert_eq!(Path::new(assigned(script, "LINK")), system.link);
        assert_eq!(assigned(script, "STATE"), file::DIR);
        assert_eq!(Path::new(assigned(script, "STATE")), system.state_dir);
        assert!(
            script.contains("[ \"$(readlink \"$LINK\" 2>/dev/null)\" = \"$STATE\" ]"),
            "Apsis's link is the one that points at the state folder"
        );
    }
    assert_eq!(assigned(PRERM, "LOCK"), JOB_LOCK);
    let helper = assigned(PRERM, "HELPER");
    assert_eq!(helper, "/usr/libexec/apsis-helper");
    assert!(
        DEB_MANIFEST.contains("[\"target/release/apsis-helper\", \"usr/libexec/\", \"755\"]"),
        "the .deb puts the helper where the prerm runs it"
    );
    assert_eq!(assigned(POSTRM, "UNIT"), UNIT_PATH);
    assert_eq!(assigned(POSTRM, "WANTS"), UNIT_WANTS_LINK);
    assert_eq!(assigned(POSTRM, "DROPIN"), DROP_IN_PATH);
    assert_eq!(
        Some(Path::new(assigned(POSTRM, "LIB"))),
        Path::new(file::DIR).parent()
    );
    assert_eq!(
        Some(Path::new(assigned(POSTRM, "ETC"))),
        Path::new(apsis_core::config::CONFIG_PATH).parent()
    );
}

/// Addition C, the root guard: nothing runs as root.
#[test]
fn the_harness_refuses_root() {
    assert!(refuse_root(true).is_err());
    assert!(refuse_root(false).is_ok());
}

/// Addition C, the path guard: a copy that names a path outside the lab isn't run, whether
/// the script spells it out lower down or was never given a variable for it. The three
/// scripts as they are pass.
#[test]
fn the_harness_refuses_a_copy_that_names_a_path_outside_its_folder() {
    let lab = Lab::new("path-guard");
    let shell = &shells()[0];
    for (name, script) in [("prerm", PRERM), ("postinst", POSTINST), ("postrm", POSTRM)] {
        if let Err(refused) = lab.copy_of(script) {
            panic!("{name}: {refused}");
        }
    }
    for line in [
        "ls /etc",
        "ls /etc/apsis",
        "ls /system-update",
        "ls /run/apsis",
        "ls \"/var/lib/apsis\"",
        "/usr/libexec/apsis-helper --disarm",
        "[ \"$(readlink \"$LINK\")\" = /var/lib/apsis/restore ] || exit 0",
        "exec 9>>/run/apsis/job.lock",
        "[ -d /run/systemd/system ] || exit 0",
        "cd /; ls etc",
        "ls \"$LINK\"/../etc",
    ] {
        let script = format!("LINK=/system-update\n{line}\n");
        assert!(lab.copy_of(&script).is_err(), "{line}");
        assert!(lab.run(&script, shell, "remove").is_err(), "{line}: it ran");
    }
    // A path under the lab that climbs out of it.
    let climbs = format!("ls '{}/../etc'\n", lab.link().display());
    assert!(only_under(&climbs, &lab.root).is_err());
    assert!(lab.copy_of("exit 0\n").is_err(), "no variable at all");
    // A comment may name them, and what's allowed is allowed.
    lab.copy_of("# removes /etc/apsis\nLINK=/system-update\nexit 0  # not /etc\n")
        .unwrap();
    lab.copy_of("LINK=/system-update\nreadlink \"$LINK\" 2>/dev/null\n")
        .unwrap();
    assert!(lab.calls().is_empty());
}

/// Addition C, the fakes: before a copy runs, `command -v systemctl` and `command -v
/// busctl` in the script's own `PATH` must be the lab's fakes, and the helper the copy
/// names must be the lab's fake too.
#[test]
fn the_harness_refuses_a_systemctl_busctl_or_helper_that_isnt_its_fake() {
    let shell = &shells()[0];
    let lab = Lab::new("fakes");
    lab.fakes_resolve(shell, &lab.path_env()).unwrap();
    // The system's own folders: whatever is found there, or nothing, isn't the fake.
    assert!(lab.fakes_resolve(shell, "/usr/bin:/bin").is_err());
    assert!(lab.fakes_resolve(shell, "").is_err());
    // No fake helper written: the prerm isn't run. Written: it is.
    assert!(lab.run(PRERM, shell, "anything").is_err());
    lab.helper(Disarm::Works);
    lab.run(PRERM, shell, "anything").unwrap();
    // A helper somewhere else in the lab isn't the one.
    let elsewhere = format!("HELPER='{}'\n", lab.root.join("apsis-helper").display());
    assert!(lab.helper_is_fake(&elsewhere).is_err());
    // Something else under a fake's name.
    for command in ["systemctl", "busctl"] {
        let lab = Lab::new("not-a-fake");
        lab.helper(Disarm::Works);
        fs::write(lab.bin().join(command), "#!/bin/sh\nexit 0\n").unwrap();
        assert!(
            lab.fakes_resolve(shell, &lab.path_env()).is_err(),
            "{command}"
        );
        assert!(lab.run(PRERM, shell, "remove").is_err(), "{command}");
        assert!(lab.calls().is_empty(), "{command}");
    }
}
