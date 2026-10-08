// SPDX-License-Identifier: GPL-3.0-only

//! `apsis-helper --apply-restore`: the offline entry point, and the real
//! [`apsis_core::restore::apply::Runner`] behind core's state machine. No D-Bus. rsync,
//! udevadm, kernelstub, plymouth and systemctl run with a fixed argv and the helper's fixed
//! `PATH`; what the person sees goes through plymouth.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apsis_core::native::{QuietRunner, SNAPSHOTS_DIR, TIMESHIFT_DIR, prune};
use apsis_core::progress::parse_rsync;
use apsis_core::restore::apply::{self, Copied, Paths, SnapshotFound};
use apsis_core::restore::esp::{self, EspError, Manifest};
use apsis_core::restore::plan::Plan;
use apsis_core::restore::{argv, filter, refusal};
use apsis_core::usage::fstype_at;
use apsis_core::{Error, Runner};

use crate::arm;
use crate::check::{self, Live, SnapshotFiles};
use crate::native::{self, Access, MOUNT_POINT, Mounted};
use crate::runner::{DirectRunner, SAFE_PATH};
use crate::settings;
use crate::unlock::{self, Boot};

/// How many lines of rsync's standard error the result keeps.
const STDERR_TAIL_LINES: usize = 20;

/// What the boot screen says while the copy runs.
const COPYING: &str = "Restoring the system. Don't turn off the computer.";

/// `apsis-helper --apply-restore`: the apply for this boot, then the exit code
/// the unit's `FailureAction=reboot` reads ([`apply::exit_code`]). A panic removes Apsis's
/// link before the process dies, so a restart can't come straight back here.
pub fn apply_restore() -> ExitCode {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("apsis-helper: apply-restore panicked: {info}");
        let paths = arm::Paths::system();
        if arm::is_armed(&paths) {
            match fs::remove_file(&paths.link) {
                Ok(()) => eprintln!("apsis-helper: /system-update removed"),
                Err(error) => eprintln!("apsis-helper: /system-update not removed: {error}"),
            }
        }
    }));
    let mut runner = RealRunner::system();
    let end = run(&mut runner);
    eprintln!("apsis-helper: apply-restore ended: {end:?}");
    match apply::exit_code(end, runner.restart_failed) {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// The apply for this boot on `runner`'s root: the boot screen's line first, once (one
/// `plymouth display-message` at the start; a live bar during the copy is
/// 0.5.x's), then core's state machine.
pub fn run<R: Runner>(runner: &mut RealRunner<R>) -> apply::End {
    use apply::Runner as _;
    runner.say(COPYING);
    runner.splash(COPYING);
    let paths = Paths {
        state_dir: &runner.paths.state_dir.clone(),
        esp: &runner.esp.clone(),
        root: &runner.root.clone(),
    };
    apply::apply(&paths, runner)
}

/// The real runner: `/` and `/boot/efi` for real, a temp root in the tests; `tools` runs the
/// programs (the helper's [`QuietRunner`] for real).
pub struct RealRunner<R: Runner> {
    paths: arm::Paths,
    root: PathBuf,
    esp: PathBuf,
    mount_point: PathBuf,
    tools: R,
    /// The backup disk, mounted read-only by `open_backup` until the runner drops.
    mounted: Option<Mounted<DirectRunner>>,
    /// plymouth answered; off after its first failure (no splash: the journal has it all).
    plymouth: bool,
    /// `/` is on a plain partition (`open_backup` saw lsblk say so): nothing unlocks it at
    /// boot, so the boot refresh has nothing to compare ([`Self::check_unlocking`]).
    plain_root: bool,
    /// `systemctl reboot --no-block` didn't go through ([`apply::exit_code`]).
    pub restart_failed: bool,
}

impl RealRunner<QuietRunner> {
    #[must_use]
    pub fn system() -> Self {
        Self::under(
            Path::new("/"),
            Path::new(MOUNT_POINT),
            QuietRunner::new(SAFE_PATH),
        )
    }
}

impl<R: Runner> RealRunner<R> {
    #[must_use]
    pub fn under(root: &Path, mount_point: &Path, tools: R) -> Self {
        Self {
            paths: arm::Paths::under(root),
            root: root.to_owned(),
            esp: root.join("boot/efi"),
            mount_point: mount_point.to_owned(),
            tools,
            mounted: None,
            plymouth: true,
            plain_root: false,
            restart_failed: false,
        }
    }

    fn snapshot_dir(&self, plan: &Plan) -> PathBuf {
        self.mount_point
            .join(TIMESHIFT_DIR)
            .join(SNAPSHOTS_DIR)
            .join(&plan.snapshot)
    }

    /// Runs one of the tools; its output, or why it couldn't run. The standard output is
    /// collected from the stream: the helper's runner hands it over piece by piece and keeps
    /// none of it in `RunOutput::stdout`.
    fn tool(&self, argv: &[String]) -> Result<apsis_core::RunOutput, String> {
        let argv: Vec<OsString> = argv.iter().map(Into::into).collect();
        let mut stdout = String::new();
        let mut output = self
            .tools
            .run_streaming(&argv, &mut |segment| {
                stdout.push_str(segment);
                stdout.push('\n');
                false
            })
            .map_err(|error| format!("{} couldn't be run: {error}", argv[0].to_string_lossy()))?;
        output.stdout = stdout;
        Ok(output)
    }

    fn progress(&mut self, percent: u8) {
        self.plymouth(&plymouth_progress_argv(percent));
    }

    /// The boot screen's line, once.
    fn splash(&mut self, text: &str) {
        self.plymouth(&plymouth_message_argv(text));
    }

    /// One plymouth call; after the first failure plymouth isn't asked again (no splash: the
    /// journal has everything), and the journal says why.
    fn plymouth(&mut self, argv: &[String]) {
        if !self.plymouth {
            return;
        }
        if let Ok(output) = self.tool(argv)
            && !output.success
        {
            self.plymouth = false;
            eprintln!(
                "apsis-helper: plymouth isn't answering ({}): the boot screen stays as it is",
                output.stderr.trim()
            );
        }
    }
}

impl<R: Runner> RealRunner<R> {
    /// After kernelstub, on a system whose `/` isn't on a plain partition: [`unlock::check`]
    /// of the refreshed ESP against the backup taken before the refresh, read with
    /// `lsinitramfs -l`. With no backup there's nothing to compare with (the apply never
    /// refreshes without one).
    fn check_unlocking(&self) -> Result<(), String> {
        use esp::{BACKUP_DIR, BootFile};

        if self.plain_root {
            return Ok(());
        }
        let backup = self.paths.state_dir.join(BACKUP_DIR);
        if fs::symlink_metadata(&backup).is_err() {
            eprintln!("apsis-helper: unlock check: no ESP backup, nothing to compare with");
            return Ok(());
        }
        let manifest = Manifest::load(&self.paths.state_dir)
            .map_err(|error| format!("the ESP backup's manifest couldn't be read: {error}"))?;
        let on_esp = |file: BootFile| self.esp.join(file.esp_path(&manifest.root_uuid));
        let text = |path: &Path| {
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
        };
        let listed = |path: &Path| -> Result<unlock::Pieces, String> {
            let argv = ["lsinitramfs", "-l", &path.to_string_lossy()].map(str::to_owned);
            let output = self.tool(&argv)?;
            if !output.success {
                return Err(format!(
                    "lsinitramfs couldn't list {}: {}",
                    path.display(),
                    tail(&output.stderr)
                ));
            }
            Ok(unlock::pieces(&output.stdout))
        };
        let (entry_before, entry_after) = (
            text(&backup.join(BootFile::CurrentEntry.name()))?,
            text(&on_esp(BootFile::CurrentEntry))?,
        );
        let before = Boot {
            entry: &entry_before,
            pieces: listed(&backup.join(BootFile::Initrd.name()))?,
        };
        let after = Boot {
            entry: &entry_after,
            pieces: listed(&on_esp(BootFile::Initrd))?,
        };
        eprintln!(
            "apsis-helper: unlock check: the initrd before has {}; after, {}; root= is {}",
            before.pieces,
            after.pieces,
            if unlock::root_option(before.entry) == unlock::root_option(after.entry) {
                "the same"
            } else {
                "another"
            }
        );
        unlock::check(&before, &after)
    }
}

/// `udevadm wait --timeout=60 /dev/disk/by-uuid/<uuid>`: the backup disk, up to a minute.
#[must_use]
pub fn udevadm_argv(uuid: &str) -> [String; 4] {
    [
        "udevadm".to_owned(),
        "wait".to_owned(),
        "--timeout=60".to_owned(),
        format!("/dev/disk/by-uuid/{uuid}"),
    ]
}

/// The boot refresh: exactly what Pop!_OS's hooks run. Used as it is
/// only when the restored tree has no `/boot/vmlinuz` or `/boot/initrd.img` link.
#[must_use]
pub fn kernelstub_argv() -> [String; 3] {
    ["kernelstub", "--verbose", "--preserve-live-mode"].map(str::to_owned)
}

/// The boot refresh naming the snapshot's kernel. kernelstub left alone takes the newest
/// kernel in `/boot` by version (`KernelOption.latest_option`, `application.py:167`), and
/// after a rollback's copy that is the protected running kernel (rule 10), so every rollback
/// would end `boot-kept`. `--kernel-path` and `--initrd-path` win over
/// it (`application.py:171-193`) and aren't saved in its configuration. The two paths are
/// what the restored `/boot/vmlinuz` and `/boot/initrd.img` point to: the kernel the check
/// (`esp::check`) compares the ESP with.
#[must_use]
pub fn kernelstub_argv_for(kernel: &Path, initrd: &Path) -> Vec<String> {
    let mut argv = kernelstub_argv().to_vec();
    argv.push("--kernel-path".to_owned());
    argv.push(kernel.to_string_lossy().into_owned());
    argv.push("--initrd-path".to_owned());
    argv.push(initrd.to_string_lossy().into_owned());
    argv
}

/// Where `/boot/<name>` points under `root`, as a path on the live system (`/boot/<target>`
/// for a relative target, the target itself for an absolute one). `None` if it isn't a link.
fn linked_in_boot(root: &Path, name: &str) -> Option<PathBuf> {
    let target = fs::read_link(root.join("boot").join(name)).ok()?;
    Some(if target.is_absolute() {
        target
    } else {
        Path::new("/boot").join(target)
    })
}

#[must_use]
pub fn plymouth_message_argv(text: &str) -> [String; 3] {
    [
        "plymouth".to_owned(),
        "display-message".to_owned(),
        format!("--text={text}"),
    ]
}

#[must_use]
pub fn plymouth_progress_argv(percent: u8) -> [String; 3] {
    [
        "plymouth".to_owned(),
        "system-update".to_owned(),
        format!("--progress={percent}"),
    ]
}

/// The restart, as it works from inside `system-update.target`.
#[must_use]
pub fn reboot_argv() -> [String; 3] {
    ["systemctl", "reboot", "--no-block"].map(str::to_owned)
}

/// The last [`STDERR_TAIL_LINES`] of `text`, joined with newlines.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(STDERR_TAIL_LINES);
    lines[from..].join("\n")
}

impl<R: Runner> apply::Runner for RealRunner<R> {
    fn is_armed(&mut self) -> bool {
        arm::is_armed(&self.paths)
    }

    fn open_backup(&mut self, plan: &Plan) -> Result<(), String> {
        let waited = self.tool(&udevadm_argv(&plan.backup_uuid))?;
        if !waited.success {
            return Err(format!(
                "the backup disk {} wasn't found within 60 seconds",
                plan.backup_uuid
            ));
        }
        // Before the mount, as for every other mount of the disk: a second device with its
        // UUID could be the one `/dev/disk/by-uuid` leads to, and its tree would go over `/`.
        // Read on its own: `Live` below reads `mountinfo` with the disk mounted.
        let devices = settings::lsblk(&DirectRunner)
            .and_then(|json| apsis_core::settings::parse_lsblk(&json))
            .map_err(|error| format!("the disks couldn't be listed: {error}"))?;
        native::only_one_with(&devices, &plan.backup_uuid).map_err(|error| error.to_string())?;
        let mounted = native::mount_by_uuid(
            &DirectRunner,
            &plan.backup_uuid,
            Access::ReadOnly,
            &self.mount_point,
        )
        .map_err(|error| format!("the backup disk couldn't be mounted: {error}"))?;
        self.mounted = Some(mounted);
        // The path checks and the refusals once more, on what's there now.
        let files = SnapshotFiles::read(&self.snapshot_dir(plan));
        let mountinfo = fs::read_to_string(self.root.join("proc/self/mountinfo"))
            .or_else(|_| fs::read_to_string("/proc/self/mountinfo"))
            .map_err(|error| format!("mountinfo couldn't be read: {error}"))?;
        let live = Live::read(&self.root, &DirectRunner, &mountinfo)
            .map_err(|error| format!("the system couldn't be read: {error}"))?;
        if let Err(refusal) = refusal::check(&live.as_system_at_apply(), &files.as_snapshot()) {
            return Err(format!("the restore was refused: {}", refusal.to_wire()));
        }
        self.plain_root = live
            .devices
            .iter()
            .any(|device| device.uuid == live.root_uuid && device.kind == "part");
        // A separate /home being restored must be the plan's partition, mounted.
        if let Some(home) = &plan.separate_home {
            if fstype_at(&mountinfo, Path::new("/home")).is_none() {
                return Err(
                    "/home isn't mounted, and the plan restores it as its own partition".to_owned(),
                );
            }
            let argv: Vec<String> = [&native::FINDMNT_ROOT_UUID[..], &["/home"]]
                .concat()
                .iter()
                .map(|a| (*a).to_owned())
                .collect();
            let found = self.tool(&argv)?;
            if found.stdout.trim() != home.uuid {
                return Err("/home isn't the partition the plan was made with".to_owned());
            }
        }
        Ok(())
    }

    fn find_snapshot(&mut self, plan: &Plan) -> SnapshotFound {
        let dir = self.snapshot_dir(plan);
        SnapshotFound {
            has_localhost: check::is_dir(&dir.join("localhost")),
            info: check::read_nofollow(&dir.join(apsis_core::native::INFO_FILE)),
        }
    }

    fn copy(&mut self, plan: &Plan) -> Copied {
        let localhost = self.snapshot_dir(plan).join("localhost");
        let argv = argv::rsync(
            &localhost,
            &self.root,
            &self.paths.state_dir.join(filter::FILE),
            &self.paths.state_dir.join(argv::LOG_FILE),
            plan.old_format,
        );
        let mut last_percent: Option<u8> = None;
        let mut shown: Vec<u8> = Vec::new();
        // All of rsync's standard output, collected from the stream (the runner keeps none):
        // `Copied::new` looks for the "skipping file deletion" line anywhere in it.
        let mut stdout = String::new();
        let output = self.tools.run_streaming(&argv, &mut |segment| {
            stdout.push_str(segment);
            stdout.push('\n');
            if let Some(progress) = parse_rsync(segment)
                && let Some(percent) = progress.percent
            {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "0..=100"
                )]
                let percent = percent.clamp(0.0, 100.0) as u8;
                if last_percent != Some(percent) {
                    last_percent = Some(percent);
                    shown.push(percent);
                }
            }
            // Every line is kept: `Copied::new` reads the whole output.
            false
        });
        for percent in shown {
            self.progress(percent);
        }
        let copied = match output {
            Ok(output) => Copied::new(output.code, &stdout, tail(&output.stderr)),
            Err(error) => Copied::new(None, "", format!("rsync couldn't be run: {error}")),
        };
        // What rsync wrote is on disk before the state says the copy ended.
        for point in std::iter::once(self.root.as_path())
            .chain(plan.separate_home.as_ref().map(|_| Path::new("/home")))
        {
            if let Ok(dir) = fs::File::open(point) {
                let _ = rustix::fs::syncfs(&dir);
            }
        }
        copied
    }

    fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError> {
        esp::back_up(&self.esp, &self.paths.state_dir, &plan.root_uuid)
    }

    fn refresh_boot(&mut self) -> Result<(), String> {
        let argv = match (
            linked_in_boot(&self.root, "vmlinuz"),
            linked_in_boot(&self.root, "initrd.img"),
        ) {
            (Some(kernel), Some(initrd)) => kernelstub_argv_for(&kernel, &initrd),
            _ => kernelstub_argv().to_vec(),
        };
        let output = self.tool(&argv)?;
        for line in output.stdout.lines().chain(output.stderr.lines()) {
            eprintln!("apsis-helper: kernelstub: {line}");
        }
        if !output.success {
            return Err(format!(
                "kernelstub exited with code {}: {}",
                output.code.map_or("none".to_owned(), |c| c.to_string()),
                tail(&output.stderr)
            ));
        }
        self.check_unlocking()
    }

    fn put_back_esp(&mut self, plan: &Plan) -> Result<(), EspError> {
        esp::put_back(&self.esp, &self.paths.state_dir, &plan.root_uuid)
    }

    fn remove_protected_kernel(&mut self, plan: &Plan) -> Result<bool, String> {
        let version = &plan.running_kernel;
        let in_snapshot = self
            .snapshot_dir(plan)
            .join("localhost/usr/lib/modules")
            .join(version);
        if fs::symlink_metadata(&in_snapshot).is_ok() {
            return Ok(false);
        }
        let boot = self.root.join("boot");
        for file in ["vmlinuz", "initrd.img", "config", "System.map"] {
            let path = boot.join(format!("{file}-{version}"));
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
        }
        let modules = self.root.join("usr/lib/modules");
        if fs::symlink_metadata(modules.join(version)).is_ok() {
            use rustix::fs::{Mode, OFlags, open};
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = open(&modules, flags, Mode::empty())
                .map_err(|error| format!("{}: {error}", modules.display()))?;
            prune::remove_at(&fd, &modules, version).map_err(|error: Error| error.to_string())?;
        }
        Ok(true)
    }

    fn remove_link(&mut self) -> Result<(), String> {
        if !arm::is_armed(&self.paths) {
            return Ok(());
        }
        fs::remove_file(&self.paths.link).map_err(|error| error.to_string())?;
        if arm::is_armed(&self.paths) {
            return Err("still there after removing it".to_owned());
        }
        Ok(())
    }

    fn remove_arm_files(&mut self) -> Result<(), String> {
        arm::remove_arm_files(&self.paths)
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn now(&mut self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }

    /// The journal only: the boot screen keeps the start line (`run`), and the lines of the
    /// steps would replace it.
    fn say(&mut self, line: &str) {
        eprintln!("apsis-helper: {line}");
    }

    fn restart(&mut self) {
        // The backup disk first: `systemctl reboot --no-block` starts the shutdown at once,
        // and systemd stops this unit with TERM while it's still unmounting ("Failed with
        // result 'signal'").
        self.mounted = None;
        self.restart_failed = match self.tool(&reboot_argv()) {
            Ok(output) => !output.success,
            Err(error) => {
                eprintln!("apsis-helper: {error}");
                true
            }
        };
        if self.restart_failed {
            eprintln!("apsis-helper: the reboot call failed; exiting so systemd restarts");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use apsis_core::RunOutput;
    use apsis_core::restore::apply::{CopyEnd, DELETIONS_SKIPPED, Runner as _};
    use apsis_core::restore::filter::Home;
    use apsis_core::restore::plan::Plan;

    use super::*;

    const NAME: &str = "2026-09-25_11-28-53";
    const KERNEL: &str = "7.1.5-76070105-generic";
    const UUID: &str = "00000000-0000-0000-0000-000000000000";

    fn temp(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apsis-apply-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Records every argv and answers each with the next scripted output.
    #[derive(Default)]
    struct FakeTools {
        calls: RefCell<Vec<Vec<String>>>,
        answers: RefCell<Vec<RunOutput>>,
    }

    impl FakeTools {
        fn answer(self, success: bool, code: i32, stdout: &str, stderr: &str) -> Self {
            self.answers.borrow_mut().insert(
                0,
                RunOutput {
                    success,
                    code: Some(code),
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                },
            );
            self
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl apsis_core::Runner for FakeTools {
        fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
            self.calls.borrow_mut().push(
                argv.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect(),
            );
            Ok(self.answers.borrow_mut().pop().unwrap_or(RunOutput {
                success: true,
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            }))
        }

        /// Like the helper's `QuietRunner`: the scripted stdout goes to the callback line by
        /// line and `RunOutput::stdout` stays empty.
        fn run_streaming(
            &self,
            argv: &[OsString],
            on_segment: &mut dyn FnMut(&str) -> bool,
        ) -> io::Result<RunOutput> {
            let mut output = self.run(argv)?;
            for line in output.stdout.lines() {
                on_segment(line);
            }
            output.stdout = String::new();
            Ok(output)
        }
    }

    fn plan() -> Plan {
        Plan {
            snapshot: NAME.to_owned(),
            snapshot_created: 1_790_000_000,
            backup_uuid: UUID.to_owned(),
            home: Home::Keep,
            old_format: false,
            safety_snapshot: None,
            root_uuid: "11111111-1111-1111-1111-111111111111".to_owned(),
            running_kernel: KERNEL.to_owned(),
            root_needs: 1,
            separate_home: None,
            starter_uid: 1000,
            prepared_at: 1_790_000_100,
        }
    }

    fn write(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn the_tools_run_with_a_fixed_argv() {
        assert_eq!(
            udevadm_argv(UUID),
            [
                "udevadm",
                "wait",
                "--timeout=60",
                &format!("/dev/disk/by-uuid/{UUID}")
            ]
        );
        assert_eq!(
            kernelstub_argv(),
            ["kernelstub", "--verbose", "--preserve-live-mode"]
        );
        assert_eq!(
            plymouth_message_argv("Restoring the system."),
            [
                "plymouth",
                "display-message",
                "--text=Restoring the system."
            ]
        );
        assert_eq!(
            plymouth_progress_argv(42),
            ["plymouth", "system-update", "--progress=42"]
        );
        assert_eq!(reboot_argv(), ["systemctl", "reboot", "--no-block"]);
    }

    #[test]
    fn the_arm_is_seen_and_removed_link_first() {
        let root = temp("arm");
        let paths = arm::Paths::under(&root);
        let exe = root.join("usr/libexec/apsis-helper");
        write(exe.clone(), "#!/bin/sh\n");
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::write(paths.state_dir.join("restore.filter"), "+ /***\n").unwrap();
        fs::write(paths.state_dir.join("restore.note"), "note\n").unwrap();
        arm::arm(&paths, &exe).unwrap();
        let mut runner = RealRunner::under(&root, &temp("mount"), FakeTools::default());
        assert!(runner.is_armed());
        runner.remove_link().unwrap();
        assert!(!runner.is_armed());
        assert!(fs::symlink_metadata(&paths.link).is_err());
        assert!(paths.unit.exists(), "the rest waits for remove_arm_files");
        runner.remove_arm_files().unwrap();
        assert!(!paths.unit.exists() && !paths.helper_copy.exists());
        // Another tool's link: not armed, and the link is left exactly where it is.
        symlink("/var/lib/other", &paths.link).unwrap();
        assert!(!runner.is_armed());
        runner.remove_link().unwrap();
        assert!(fs::symlink_metadata(&paths.link).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_snapshot_is_read_from_the_mount_point() {
        let root = temp("root");
        let mount = temp("mount");
        let mut runner = RealRunner::under(&root, &mount, FakeTools::default());
        let found = runner.find_snapshot(&plan());
        assert!(!found.has_localhost && found.info.is_none());
        let dir = mount.join("timeshift/snapshots").join(NAME);
        write(dir.join("info.json"), "{\"created\": 1}");
        fs::create_dir_all(dir.join("localhost")).unwrap();
        let found = runner.find_snapshot(&plan());
        assert!(found.has_localhost);
        assert_eq!(found.info.as_deref(), Some("{\"created\": 1}"));
        // `localhost` a link isn't the folder.
        fs::remove_dir(dir.join("localhost")).unwrap();
        symlink(&root, dir.join("localhost")).unwrap();
        assert!(!runner.find_snapshot(&plan()).has_localhost);
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&mount).unwrap();
    }

    #[test]
    fn the_protected_kernel_goes_only_when_the_snapshot_lacks_it() {
        let root = temp("root");
        let mount = temp("mount");
        for file in ["vmlinuz", "initrd.img", "config", "System.map"] {
            write(root.join(format!("boot/{file}-{KERNEL}")), "x");
        }
        write(
            root.join("usr/lib/modules")
                .join(KERNEL)
                .join("kernel/x.ko"),
            "x",
        );
        let snapshot_modules = mount
            .join("timeshift/snapshots")
            .join(NAME)
            .join("localhost/usr/lib/modules")
            .join(KERNEL);
        fs::create_dir_all(&snapshot_modules).unwrap();
        let mut runner = RealRunner::under(&root, &mount, FakeTools::default());
        assert_eq!(runner.remove_protected_kernel(&plan()), Ok(false));
        assert!(root.join(format!("boot/vmlinuz-{KERNEL}")).exists());
        fs::remove_dir_all(&snapshot_modules).unwrap();
        assert_eq!(runner.remove_protected_kernel(&plan()), Ok(true));
        for file in ["vmlinuz", "initrd.img", "config", "System.map"] {
            assert!(
                !root.join(format!("boot/{file}-{KERNEL}")).exists(),
                "{file}"
            );
        }
        assert!(!root.join("usr/lib/modules").join(KERNEL).exists());
        // Already gone: still true, nothing to fail on.
        assert_eq!(runner.remove_protected_kernel(&plan()), Ok(true));
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&mount).unwrap();
    }

    /// The apply's first words: the boot screen's line, once, before
    /// anything else, through plymouth; a live bar during the copy is 0.5.x's.
    #[test]
    fn the_apply_says_its_line_first_and_leaves_another_tools_link_alone() {
        let root = temp("root");
        let mut runner = RealRunner::under(&root, &temp("mount"), FakeTools::default());
        let end = run(&mut runner);
        assert_eq!(end, apsis_core::restore::apply::End::NotArmed);
        let calls = runner.tools.calls();
        assert_eq!(calls[0], plymouth_message_argv(COPYING));
        assert!(
            !calls.iter().any(|c| c == &reboot_argv()),
            "no restart when not armed"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// The tools' output is read from the stream (the quiet runner keeps none): kernelstub's
    /// lines reach the journal, and a failure carries its code and the end of its errors.
    #[test]
    fn the_boot_refresh_reads_kernelstub_and_reports_a_failure() {
        let root = temp("root");
        let tools = FakeTools::default()
            .answer(true, 0, "kernelstub: Making entry\n", "")
            .answer(false, 1, "", "kernelstub: no ESP\n");
        let mut runner = RealRunner::under(&root, &temp("mount"), tools);
        assert_eq!(runner.refresh_boot(), Ok(()));
        let error = runner.refresh_boot().unwrap_err();
        assert!(
            error.contains("code 1") && error.contains("no ESP"),
            "{error}"
        );
        let calls = runner.tools.calls();
        assert_eq!(calls[0], kernelstub_argv());
        assert_eq!(calls.len(), 2);
        // What `tool` hands back has the streamed output in it.
        let streamed = RealRunner::under(
            &root,
            &temp("mount"),
            FakeTools::default().answer(true, 0, "one\ntwo\n", ""),
        );
        let output = streamed.tool(&["echo".to_owned()]).unwrap();
        assert_eq!(output.stdout, "one\ntwo\n");
        fs::remove_dir_all(&root).unwrap();
    }

    /// After kernelstub the refreshed ESP is compared with the backup taken before it, and
    /// an initrd that lost what unlocks the disk fails the refresh (the apply then puts the
    /// boot files back). Not on a plain partition, where nothing is listed.
    #[test]
    fn a_refresh_whose_initrd_cant_unlock_the_disk_fails() {
        use apsis_core::restore::esp::BootFile;

        const ENCRYPTED: &str = "\
-rw-r--r--   1 root     root           66 Oct  1 10:00 cryptroot/crypttab
-rwxr-xr-x   1 root     root       163944 Apr  8  2024 usr/sbin/cryptsetup
-rwxr-xr-x   1 root     root      3021000 Apr  8  2024 usr/sbin/lvm
";
        const PLAIN: &str = "-rw-r--r--   1 root     root   0 Oct  1 10:00 cryptroot/crypttab\n";
        let root = temp("root");
        let esp_dir = root.join("boot/efi");
        for file in [
            BootFile::Kernel,
            BootFile::Initrd,
            BootFile::Cmdline,
            BootFile::CurrentEntry,
        ] {
            let path = esp_dir.join(file.esp_path(UUID));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("options root=UUID={UUID} ro quiet\n")).unwrap();
        }
        let state_dir = arm::Paths::under(&root).state_dir;
        fs::create_dir_all(&state_dir).unwrap();
        esp::back_up(&esp_dir, &state_dir, UUID).unwrap();
        // Twice: kernelstub, then lsinitramfs of the backup's initrd and of the ESP's.
        let tools = FakeTools::default()
            .answer(true, 0, "", "")
            .answer(true, 0, ENCRYPTED, "")
            .answer(true, 0, ENCRYPTED, "")
            .answer(true, 0, "", "")
            .answer(true, 0, ENCRYPTED, "")
            .answer(true, 0, PLAIN, "");
        let mut runner = RealRunner::under(&root, &temp("mount"), tools);
        assert_eq!(runner.refresh_boot(), Ok(()));
        let error = runner.refresh_boot().unwrap_err();
        assert!(error.contains("cryptsetup, lvm"), "{error}");
        let calls = runner.tools.calls();
        assert_eq!(calls.len(), 6);
        assert_eq!(calls[1][..2], ["lsinitramfs", "-l"]);
        assert!(calls[1][2].ends_with("esp-backup/initrd.img"), "{calls:?}");
        assert!(
            calls[2][2].ends_with(&format!("boot/efi/EFI/Pop_OS-{UUID}/initrd.img")),
            "{calls:?}"
        );
        // Another root= in the new entry fails it too.
        fs::write(
            esp_dir.join(BootFile::CurrentEntry.esp_path(UUID)),
            "options root=/dev/sdX9 ro quiet\n",
        )
        .unwrap();
        let error = runner.refresh_boot().unwrap_err();
        assert!(error.contains("root= option"), "{error}");
        // On a plain partition: kernelstub only, whatever the entry says.
        let calls_before = runner.tools.calls().len();
        runner.plain_root = true;
        assert_eq!(runner.refresh_boot(), Ok(()));
        assert_eq!(runner.tools.calls().len(), calls_before + 1);
        fs::remove_dir_all(&root).unwrap();
    }

    /// kernelstub left alone takes the newest kernel in `/boot` by version (its
    /// `KernelOption.latest_option`), and after a rollback's copy that is the protected
    /// running kernel, not the snapshot's: every rollback would end `boot-kept`. So the refresh
    /// names the kernel and initrd the restored tree's links point to, with the options kernelstub
    /// has for it. Without the links: the plain call, and the check decides.
    #[test]
    fn the_boot_refresh_names_the_kernel_the_links_point_to() {
        let root = temp("root");
        fs::create_dir_all(root.join("boot")).unwrap();
        symlink("vmlinuz-7.1.5-generic", root.join("boot/vmlinuz")).unwrap();
        symlink("initrd.img-7.1.5-generic", root.join("boot/initrd.img")).unwrap();
        let mut runner = RealRunner::under(&root, &temp("mount"), FakeTools::default());
        assert_eq!(runner.refresh_boot(), Ok(()));
        assert_eq!(
            runner.tools.calls()[0],
            [
                "kernelstub",
                "--verbose",
                "--preserve-live-mode",
                "--kernel-path",
                "/boot/vmlinuz-7.1.5-generic",
                "--initrd-path",
                "/boot/initrd.img-7.1.5-generic",
            ]
        );
        // An absolute link target is taken as it is; a missing link means the plain call.
        fs::remove_file(root.join("boot/vmlinuz")).unwrap();
        symlink("/boot/vmlinuz-7.0.11-generic", root.join("boot/vmlinuz")).unwrap();
        runner.refresh_boot().unwrap();
        assert_eq!(runner.tools.calls()[1][4], "/boot/vmlinuz-7.0.11-generic");
        fs::remove_file(root.join("boot/initrd.img")).unwrap();
        runner.refresh_boot().unwrap();
        assert_eq!(runner.tools.calls()[2], kernelstub_argv());
        fs::remove_dir_all(&root).unwrap();
    }

    /// The journal's lines stay in the journal: only the start line reaches the boot screen
    /// (a line like "copying, attempt 1 of 3" would replace it).
    #[test]
    fn say_goes_to_the_journal_only_and_restart_through_the_tools() {
        let root = temp("root");
        let tools = FakeTools::default().answer(false, 1, "", "Failed to reboot");
        let mut runner = RealRunner::under(&root, &temp("mount"), tools);
        runner.say("one");
        runner.say("two");
        runner.restart();
        assert!(runner.restart_failed);
        let calls = runner.tools.calls();
        assert_eq!(calls, [reboot_argv().to_vec()]);
        let mut ok = RealRunner::under(&root, &temp("mount"), FakeTools::default());
        ok.restart();
        assert!(!ok.restart_failed);
        fs::remove_dir_all(&root).unwrap();
    }

    /// The copy hands core rsync's exit, all of its standard output (the "skipping file
    /// deletion" line can be anywhere in it) and the end of its standard error.
    #[test]
    fn the_copy_hands_core_the_exit_the_output_and_the_errors_tail() {
        let root = temp("root");
        let mount = temp("mount");
        let stdout = format!("file one\n{DELETIONS_SKIPPED}\nfile two\n");
        let stderr: String = (1..=30).map(|n| format!("rsync: error {n}\n")).collect();
        let tools = FakeTools::default().answer(false, 23, &stdout, &stderr);
        let mut runner = RealRunner::under(&root, &mount, tools);
        let copied = runner.copy(&plan());
        assert_eq!(copied.exit, Some(23));
        assert!(copied.deletions_skipped);
        assert_eq!(copied.end(), CopyEnd::Broke);
        assert!(copied.tail.ends_with("rsync: error 30"), "{}", copied.tail);
        assert!(
            copied.tail.starts_with("rsync: error 11"),
            "{}",
            copied.tail
        );
        assert!(
            !copied.tail.contains("error 1\n"),
            "only the end: {}",
            copied.tail
        );
        // What core says line by line after a plain 23: these twenty.
        assert_eq!(copied.error_lines().count(), STDERR_TAIL_LINES);
        let calls = runner.tools.calls();
        let argv = &calls[0];
        assert_eq!(argv[0], "rsync");
        assert!(argv.iter().any(
            |a| a == "--exclude-from=/var/lib/apsis/restore/restore.filter"
                || a.ends_with("/var/lib/apsis/restore/restore.filter")
        ));
        assert!(argv.last().unwrap().ends_with('/'), "the root with a slash");
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&mount).unwrap();
    }
}
