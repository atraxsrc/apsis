// SPDX-License-Identifier: GPL-3.0-only

//! The apply: what `apsis-helper --apply-restore` does in the offline boot (PLAN 6b.6), as a
//! state machine over a [`Runner`].
//!
//! Everything that needs root or a real machine (the link, the backup disk, rsync, the boot
//! refresh, the restart) is the runner's. What's decided here: the order, what's saved to
//! `state.json` before which step, how a boot after a power cut goes on, and which outcome
//! each failure ends in. The plan, the state, the result and the ESP backup are read and
//! written only through [`super::plan`], [`super::state`] and [`super::esp`].
//!
//! No clock is read to decide anything (PLAN 6b.6 step 1): [`Runner::now`] only dates the
//! result.

use std::fs;
use std::path::Path;

use super::esp::{self, Checked, EspError, Manifest, Previous};
use super::file::FileError;
use super::filter::Home;
use super::plan::Plan;
use super::state::{MAX_ATTEMPTS, Outcome, Report, State, Step};

/// The message of the report of last resort (PLAN 6b.10).
pub const MINIMAL_MESSAGE: &str = "result could not be saved, see journal";

/// rsync's log, named in a result with problems.
const RSYNC_LOG: &str = "/var/lib/apsis/restore/rsync-log";

/// Where the apply works. Paths, so the tests run on temp trees.
#[derive(Debug, Clone, Copy)]
pub struct Paths<'a> {
    /// [`super::file::DIR`].
    pub state_dir: &'a Path,
    /// `/boot/efi`.
    pub esp: &'a Path,
    /// `/`: the tree pass 1 restores, and the one the ESP is checked against after the boot
    /// refresh (PLAN 6b.6 step 6).
    pub root: &'a Path,
}

/// How pass 1 ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// rsync's exit code. `None`: it didn't run, or a signal ended it.
    pub exit: Option<i32>,
    /// Its last lines, for the result of a copy that broke.
    pub tail: String,
}

/// What the apply needs done on the machine. The helper has the real one; the tests a fake.
pub trait Runner {
    /// `/system-update` is a link to the state folder (PLAN 6b.6 step 1).
    fn is_armed(&mut self) -> bool;

    /// Step 2: waits for the backup disk, mounts it read-only, runs the path checks and the
    /// refusals again, and checks a separate `/home` that's restored.
    ///
    /// # Errors
    ///
    /// Why the restore can't start in this boot, in words for the result.
    fn open_backup(&mut self, plan: &Plan) -> Result<(), String>;

    /// Step 3: pass 1, rsync over `/`. It returns only when what rsync wrote is on disk
    /// (`sync`): a copy that ended is never run again after a power cut.
    fn copy(&mut self, plan: &Plan) -> Copied;

    /// Step 4: [`esp::back_up`].
    ///
    /// # Errors
    ///
    /// As [`esp::back_up`]. The apply then leaves the boot files alone.
    fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError>;

    /// Step 5: `update-initramfs -u -k all`, then `kernelstub --verbose`.
    ///
    /// # Errors
    ///
    /// The command that didn't exit 0, and its last lines.
    fn refresh_boot(&mut self) -> Result<(), String>;

    /// Step 6, "boot files failed": [`esp::put_back`].
    ///
    /// # Errors
    ///
    /// As [`esp::put_back`].
    fn put_back_esp(&mut self, plan: &Plan) -> Result<(), EspError>;

    /// Step 7: if the snapshot doesn't have the plan's running kernel, removes that kernel's
    /// rule 10 files and says `true`. Only called after a passed check, and never for a
    /// kernel the ESP boots.
    ///
    /// # Errors
    ///
    /// Why the files couldn't be removed.
    fn remove_protected_kernel(&mut self, plan: &Plan) -> Result<bool, String>;

    /// Removes `/system-update` first, then the unit, its wants link and the helper copy
    /// (PLAN 6b.5). Without the link it removes whatever of the rest is there.
    ///
    /// # Errors
    ///
    /// What couldn't be removed.
    fn disarm(&mut self) -> Result<(), String>;

    /// The clock, in Unix seconds, for the result's time. Never compared with anything.
    fn now(&mut self) -> i64;

    /// One line for the journal, the console and the boot screen.
    fn say(&mut self, line: &str);

    /// Restarts the computer. Called once, last, however the apply ended: with the link
    /// gone that's the normal boot, with the link kept it's the next attempt.
    fn restart(&mut self);
}

/// How one boot's apply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// `/system-update` wasn't Apsis's link: nothing was read, written or reported.
    NotArmed,
    /// The copy broke. The link stays, and the next boot is this attempt.
    Retry { attempt: u32 },
    /// The restore is over, with this outcome in `result.json`.
    Finished(Outcome),
}

/// Runs the apply for this boot, then restarts.
pub fn apply(paths: &Paths<'_>, runner: &mut impl Runner) -> End {
    let end = run(paths, runner);
    runner.restart();
    end
}

fn run(paths: &Paths<'_>, runner: &mut impl Runner) -> End {
    let dir = paths.state_dir;

    // Step 1: the arm check. A link that isn't Apsis's is removed, and that's all.
    if !runner.is_armed() {
        runner.say("no restore is armed: removing /system-update and starting normally");
        clean_up(paths, runner);
        return End::NotArmed;
    }
    let state = State::load(dir);
    let plan = match Plan::load(dir) {
        Ok(plan) => plan,
        Err(error) => {
            let written = match &state {
                Ok(state) if !state.written => "nothing was written",
                _ => "the system may be partly restored",
            };
            let message = format!("the restore's plan couldn't be read ({error}); {written}");
            return finish(paths, runner, None, state.ok(), Outcome::Failed, message);
        }
    };
    let mut state = match state {
        Ok(state) => state,
        Err(error) => {
            let message = format!(
                "the restore's state couldn't be read ({error}); whether anything was written \
                 isn't known"
            );
            return finish(paths, runner, Some(&plan), None, Outcome::Failed, message);
        }
    };
    // The result is written: an earlier boot got that far, and only the cleanup is left.
    if state.step == Step::End {
        return match Report::load(dir) {
            Ok(report) => {
                clean_up(paths, runner);
                End::Finished(report.outcome)
            }
            Err(error) => {
                let message = format!("the restore ended, but its result can't be read ({error})");
                finish(
                    paths,
                    runner,
                    Some(&plan),
                    Some(state),
                    Outcome::Failed,
                    message,
                )
            }
        };
    }

    // Step 2: the backup disk. Nothing is written in this boot if it fails.
    if let Err(reason) = runner.open_backup(&plan) {
        return finish(
            paths,
            runner,
            Some(&plan),
            Some(state),
            unstarted(state),
            reason,
        );
    }

    // Step 3: pass 1, unless an earlier boot's copy ended and is on disk.
    if matches!(state.step, Step::Armed | Step::Copy) {
        if state.attempts >= MAX_ATTEMPTS {
            // The last attempt is counted, and never ended: the power went during it.
            let message = format!("the copy was cut off on each of {MAX_ATTEMPTS} tries");
            return finish(
                paths,
                runner,
                Some(&plan),
                Some(state),
                Outcome::Failed,
                message,
            );
        }
        let copying = State {
            attempts: state.attempts + 1,
            written: true,
            step: Step::Copy,
            problems: false,
        };
        if let Err(error) = copying.save(dir) {
            let message = format!("state.json couldn't be saved ({error}), so nothing was copied");
            return finish(
                paths,
                runner,
                Some(&plan),
                Some(state),
                unstarted(state),
                message,
            );
        }
        state = copying;
        runner.say(&format!(
            "copying, attempt {} of {MAX_ATTEMPTS}",
            state.attempts
        ));
        let copied = runner.copy(&plan);
        let problems = match copied.exit {
            Some(0 | 24) => false,
            Some(23) => true,
            _ if state.attempts >= MAX_ATTEMPTS => {
                let message = format!(
                    "the copy broke on each of {MAX_ATTEMPTS} tries: {}",
                    copied.tail
                );
                return finish(
                    paths,
                    runner,
                    Some(&plan),
                    Some(state),
                    Outcome::Failed,
                    message,
                );
            }
            _ => {
                let attempt = state.attempts + 1;
                runner.say(&format!(
                    "the copy broke ({}): restarting to try again, attempt {attempt} of \
                     {MAX_ATTEMPTS}",
                    copied.tail
                ));
                return End::Retry { attempt };
            }
        };
        let copied = State {
            step: Step::BootFiles,
            problems,
            ..state
        };
        if let Err(error) = copied.save(dir) {
            // What the boot files' steps depend on isn't on disk: they aren't started. The
            // ESP boots the kernel from before, which rule 10 kept.
            let message = format!(
                "state.json couldn't be saved ({error}), so the boot files were left as they are"
            );
            return finish(
                paths,
                runner,
                Some(&plan),
                Some(state),
                Outcome::BootKept,
                message,
            );
        }
        state = copied;
    }

    // Steps 4 to 7.
    let (outcome, message) = boot_files(paths, runner, &plan, state.problems);
    finish(paths, runner, Some(&plan), Some(state), outcome, message)
}

/// A restore that stops before this boot's copy: it never started, unless an earlier boot's
/// copy wrote something (PLAN 6b.10).
fn unstarted(state: State) -> Outcome {
    if state.written {
        Outcome::Failed
    } else {
        Outcome::NotStarted
    }
}

/// Steps 4 to 7: the ESP backup, the boot refresh, the check against the restored tree, and
/// the put-back or the cleanup of the kept kernel.
fn boot_files(
    paths: &Paths<'_>,
    runner: &mut impl Runner,
    plan: &Plan,
    problems: bool,
) -> (Outcome, String) {
    let Paths {
        state_dir,
        esp,
        root,
    } = *paths;

    // Step 4. A backup folder that's there was made by an earlier boot, before or while the
    // boot files were changed: it's kept if it verifies, and never taken again.
    if fs::symlink_metadata(state_dir.join(esp::BACKUP_DIR)).is_ok() {
        let verified = esp::verify(state_dir)
            .map_err(|error| error.to_string())
            .and_then(|manifest| {
                if manifest.root_uuid == plan.root_uuid {
                    Ok(())
                } else {
                    Err("it's of another installation".to_owned())
                }
            });
        if let Err(error) = verified {
            runner.say(&format!("the ESP backup doesn't verify: {error}"));
            return unverified(paths, runner, plan, problems);
        }
        runner.say("the ESP backup of an earlier boot verifies: kept");
    } else if let Err(error) = runner.back_up_esp(plan) {
        return (
            Outcome::BootKept,
            format!(
                "the boot files couldn't be backed up ({error}), so they were left as they are"
            ),
        );
    }

    // Step 5.
    runner.say("refreshing the boot files");
    let refreshed = runner.refresh_boot();

    // Step 6: against the restored tree.
    let failure = match (refreshed, esp::check(esp, root, &plan.root_uuid)) {
        (Ok(()), Ok(checked)) => return passed(runner, plan, problems, &checked, None),
        (Err(error), _) => format!("the boot refresh failed ({error})"),
        (Ok(()), Err(failure)) => format!("the new boot files didn't check out ({failure})"),
    };
    runner.say(&format!("{failure}: putting the boot files back"));
    match runner.put_back_esp(plan) {
        Err(error) => (
            Outcome::BootBroken,
            format!("{failure}; the boot files from before couldn't be put back ({error})"),
        ),
        // `boot-kept` says the ESP boots the kernel from before: that kernel must be whole.
        Ok(()) if !esp::boots_kernel(esp, root, &plan.root_uuid, &plan.running_kernel) => (
            Outcome::BootBroken,
            format!(
                "{failure}; the boot files from before were put back, but that kernel's files \
                 are gone"
            ),
        ),
        Ok(()) => (
            Outcome::BootKept,
            format!("{failure}; the boot files from before were put back"),
        ),
    }
}

/// The ESP backup is there and doesn't verify. The boot files may already be changed, so
/// it's not taken again; without a backup nothing is refreshed and nothing can be put back.
/// The outcome is what the ESP boots as it is.
fn unverified(
    paths: &Paths<'_>,
    runner: &mut impl Runner,
    plan: &Plan,
    problems: bool,
) -> (Outcome, String) {
    const WHY: &str = "the backup of the boot files couldn't be verified after an interruption";
    let Paths { esp, root, .. } = *paths;
    if let Ok(checked) = esp::check(esp, root, &plan.root_uuid) {
        let note = format!("{WHY}; the boot files check out as they are");
        return passed(runner, plan, problems, &checked, Some(note));
    }
    if esp::boots_kernel(esp, root, &plan.root_uuid, &plan.running_kernel) {
        return (
            Outcome::BootKept,
            format!(
                "{WHY}, so they were left as they are; the kernel from before the restore still \
                 boots"
            ),
        );
    }
    (
        Outcome::BootBroken,
        format!(
            "{WHY}, and the boot files are neither the restored kernel's nor the ones from before"
        ),
    )
}

/// Step 7, after a passed check: the kept kernel goes if the snapshot doesn't have it.
fn passed(
    runner: &mut impl Runner,
    plan: &Plan,
    problems: bool,
    checked: &Checked,
    note: Option<String>,
) -> (Outcome, String) {
    let mut outcome = Outcome::Done;
    let mut notes: Vec<String> = note.into_iter().collect();
    if problems {
        outcome = Outcome::Problems;
        notes.push(format!(
            "some files couldn't be written or deleted; see {RSYNC_LOG}"
        ));
    }
    if let Previous::Wrong(failure) = &checked.previous {
        notes.push(format!("the previous kernel's boot files: {failure}"));
    }
    // Never a kernel the ESP boots, whatever the runner would say.
    let running = &plan.running_kernel;
    let booted = checked.version == *running
        || matches!(&checked.previous, Previous::Good { version } if version == running);
    if !booted {
        match runner.remove_protected_kernel(plan) {
            Ok(true) => runner.say(&format!("removed kernel {running}: not in the snapshot")),
            Ok(false) => {}
            Err(error) => {
                outcome = Outcome::Problems;
                notes.push(format!(
                    "the kernel from before the restore couldn't be removed ({error})"
                ));
            }
        }
    }
    (outcome, notes.join("; "))
}

/// Step 8: the result, then the end in `state.json`, then the cleanup. Gives the outcome
/// that's in `result.json`.
fn finish(
    paths: &Paths<'_>,
    runner: &mut impl Runner,
    plan: Option<&Plan>,
    state: Option<State>,
    outcome: Outcome,
    message: String,
) -> End {
    let dir = paths.state_dir;
    let now = runner.now();
    let report = Report {
        outcome,
        snapshot: plan.map(|plan| plan.snapshot.clone()),
        safety_snapshot: plan.and_then(|plan| plan.safety_snapshot.clone()),
        home: plan.map_or(Home::Keep, |plan| plan.home),
        message,
        when: Some(now),
    };
    let saved = match report.save(dir) {
        Ok(()) => outcome,
        Err(error) => {
            runner.say(&format!("result.json wasn't saved: {error}"));
            save_minimal(dir, runner, &report);
            Outcome::Failed
        }
    };
    runner.say(&format!(
        "the restore ended: {}{}{}",
        outcome.word(),
        if report.message.is_empty() { "" } else { ": " },
        report.message
    ));
    // Saved before the cleanup: a boot that still finds the link only cleans up.
    if let Some(state) = state {
        let end = State {
            step: Step::End,
            ..state
        };
        if let Err(error) = end.save(dir) {
            runner.say(&format!("state.json wasn't saved: {error}"));
        }
    }
    clean_up(paths, runner);
    End::Finished(saved)
}

/// The report of last resort (PLAN 6b.10): `failed`, the message, and what's left of the
/// real one that can stand by itself. If even that is refused, nothing but the message.
fn save_minimal(dir: &Path, runner: &mut impl Runner, refused: &Report) {
    let minimal = Report {
        outcome: Outcome::Failed,
        message: format!(
            "{MINIMAL_MESSAGE} (the restore ended: {})",
            refused.outcome.word()
        ),
        when: refused.when.filter(|when| *when > 0),
        ..refused.clone()
    };
    let bare = Report {
        snapshot: None,
        safety_snapshot: None,
        when: None,
        ..minimal.clone()
    };
    let saved: Result<(), FileError> = minimal.save(dir).or_else(|_| bare.save(dir));
    if let Err(error) = saved {
        runner.say(&format!(
            "the minimal result.json wasn't saved either: {error}"
        ));
    }
}

/// Removes the arm, the link first, then the ESP backup.
fn clean_up(paths: &Paths<'_>, runner: &mut impl Runner) {
    if let Err(error) = runner.disarm() {
        runner.say(&format!("disarming failed: {error}"));
    }
    if let Err(error) = esp::remove(paths.state_dir) {
        runner.say(&format!("the ESP backup wasn't removed: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};

    use super::super::esp::tests::{Lab, NEW, OLD, OLDER, UUID, initrd, kernel, lab};
    use super::super::esp::{BACKUP_DIR, BootFile, CheckFailure, MANIFEST_FILE};
    use super::super::plan;
    use super::super::state::{RESULT_FILE, STATE_FILE};
    use super::*;

    const SNAPSHOT: &str = "2026-09-25_11-28-00";
    const SAFETY: &str = "2026-10-01_09-15-42";
    const NOW: i64 = 1_790_000_600;

    /// The lab's machine runs `NEW` (previous `OLD`). The snapshot is from before the kernel
    /// update: `OLD` (previous `OLDER`), and it doesn't have `NEW`.
    fn plan() -> Plan {
        Plan {
            snapshot: SNAPSHOT.to_owned(),
            backup_uuid: "99999999-8888-7777-6666-555555555555".to_owned(),
            home: Home::Keep,
            old_format: false,
            safety_snapshot: Some(SAFETY.to_owned()),
            root_uuid: UUID.to_owned(),
            running_kernel: NEW.to_owned(),
            root_needs: 1,
            separate_home: None,
            starter_uid: 1000,
            prepared_at: 1_790_000_000,
        }
    }

    /// What arming leaves: the plan, a new state, and the link, made last.
    fn arm(lab: &Lab) {
        plan().save(&lab.state).unwrap();
        State::default().save(&lab.state).unwrap();
        std::os::unix::fs::symlink(&lab.state, lab.root.join("system-update")).unwrap();
    }

    fn armed(name: &str) -> Lab {
        let lab = lab(name);
        arm(&lab);
        lab
    }

    fn is_linked(lab: &Lab) -> bool {
        fs::symlink_metadata(lab.root.join("system-update")).is_ok()
    }

    fn report(lab: &Lab) -> Report {
        Report::load(&lab.state).unwrap()
    }

    /// What the boot refresh does in a test.
    #[derive(Clone, Copy, PartialEq)]
    enum Refresh {
        /// kernelstub copies what `/boot`'s links name to the ESP.
        Works,
        /// It leaves half a kernel on the ESP and exits 1.
        Fails,
        /// It exits 0, but the ESP's kernel isn't `/boot`'s.
        Wrong,
    }

    /// Where the power goes: before or after the n-th thing the runner is asked in a boot.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Cut {
        Before(usize),
        After(usize),
    }

    /// What a power cut unwinds with.
    struct PowerCut;

    struct Fake<'a> {
        lab: &'a Lab,
        /// Everything asked of the runner, over all boots.
        calls: Vec<&'static str>,
        /// Asked in this boot.
        count: usize,
        cut: Option<Cut>,
        backup: Result<(), String>,
        /// The exit codes of the copies to come; exit 0 when it's empty.
        exits: VecDeque<Option<i32>>,
        refresh: Refresh,
        fail_esp_backup: bool,
        break_put_back: bool,
        snapshot_has_running_kernel: bool,
        fail_kernel_removal: bool,
        now: i64,
        /// `state.json` as each copy, ESP backup, boot refresh and disarm found it.
        states: Vec<(&'static str, State)>,
        /// Whether `result.json` was there when the link was removed.
        result_at_disarm: Vec<bool>,
        said: Vec<String>,
    }

    impl<'a> Fake<'a> {
        fn new(lab: &'a Lab) -> Self {
            Self {
                lab,
                calls: Vec::new(),
                count: 0,
                cut: None,
                backup: Ok(()),
                exits: VecDeque::new(),
                refresh: Refresh::Works,
                fail_esp_backup: false,
                break_put_back: false,
                snapshot_has_running_kernel: false,
                fail_kernel_removal: false,
                now: NOW,
                states: Vec::new(),
                result_at_disarm: Vec::new(),
                said: Vec::new(),
            }
        }

        /// One thing asked of the runner, with the power cut before or after it.
        fn call<T>(&mut self, name: &'static str, effect: impl FnOnce(&mut Self) -> T) -> T {
            let index = self.count;
            self.count += 1;
            if self.cut == Some(Cut::Before(index)) {
                panic_any(PowerCut);
            }
            self.calls.push(name);
            let value = effect(self);
            if self.cut == Some(Cut::After(index)) {
                panic_any(PowerCut);
            }
            value
        }

        fn note_state(&mut self, at: &'static str) {
            if let Ok(state) = State::load(&self.lab.state) {
                self.states.push((at, state));
            }
        }

        fn count_of(&self, name: &str) -> usize {
            self.calls.iter().filter(|call| **call == name).count()
        }

        /// The version a `/boot` link names.
        fn linked(&self, link: &str) -> String {
            let target = fs::read_link(self.lab.root.join("boot").join(link)).unwrap();
            let name = target.to_str().unwrap();
            name.strip_prefix("vmlinuz-").unwrap().to_owned()
        }
    }

    impl Runner for Fake<'_> {
        fn is_armed(&mut self) -> bool {
            self.call("is_armed", |fake| {
                fs::read_link(fake.lab.root.join("system-update"))
                    .is_ok_and(|target| target == fake.lab.state)
            })
        }

        fn open_backup(&mut self, _plan: &Plan) -> Result<(), String> {
            self.call("open_backup", |fake| fake.backup.clone())
        }

        fn copy(&mut self, _plan: &Plan) -> Copied {
            self.call("copy", |fake| {
                fake.note_state("copy");
                let exit = fake.exits.pop_front().unwrap_or(Some(0));
                if matches!(exit, Some(0 | 23 | 24)) {
                    // The snapshot's kernels arrive; rule 10 keeps the running one's files.
                    fake.lab.install_kernel(OLD);
                    fake.lab.install_kernel(OLDER);
                    fake.lab.link_kernel(OLD);
                    fake.lab.link_previous(OLDER);
                }
                Copied {
                    exit,
                    tail: format!("rsync error: code {exit:?}"),
                }
            })
        }

        fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError> {
            self.call("back_up_esp", |fake| {
                fake.note_state("back_up_esp");
                if fake.fail_esp_backup {
                    return Err(EspError::Missing("vmlinuz.efi"));
                }
                esp::back_up(&fake.lab.esp, &fake.lab.state, &plan.root_uuid)
            })
        }

        fn refresh_boot(&mut self) -> Result<(), String> {
            self.call("refresh_boot", |fake| {
                fake.note_state("refresh_boot");
                let (current, previous) = (fake.linked("vmlinuz"), fake.linked("vmlinuz.old"));
                match fake.refresh {
                    Refresh::Works => {
                        fake.lab.kernelstub(&current);
                        fake.lab.kernelstub_previous(&previous);
                        Ok(())
                    }
                    Refresh::Fails => {
                        fake.lab.kernelstub_previous(&previous);
                        fs::write(fake.lab.esp_file(BootFile::Kernel), "half a kernel").unwrap();
                        Err("kernelstub exited 1".to_owned())
                    }
                    Refresh::Wrong => {
                        fake.lab.kernelstub(&current);
                        fs::write(fake.lab.esp_file(BootFile::Kernel), kernel("other")).unwrap();
                        Ok(())
                    }
                }
            })
        }

        fn put_back_esp(&mut self, plan: &Plan) -> Result<(), EspError> {
            self.call("put_back_esp", |fake| {
                if fake.break_put_back {
                    fs::write(fake.lab.esp_file(BootFile::Initrd), "half an initrd").unwrap();
                    return Err(EspError::Mismatch("initrd.img"));
                }
                esp::put_back(&fake.lab.esp, &fake.lab.state, &plan.root_uuid)
            })
        }

        fn remove_protected_kernel(&mut self, plan: &Plan) -> Result<bool, String> {
            self.call("remove_protected_kernel", |fake| {
                if fake.fail_kernel_removal {
                    return Err("Read-only file system".to_owned());
                }
                if fake.snapshot_has_running_kernel {
                    return Ok(false);
                }
                let version = &plan.running_kernel;
                let boot = fake.lab.root.join("boot");
                let _ = fs::remove_file(boot.join(format!("vmlinuz-{version}")));
                let _ = fs::remove_file(boot.join(format!("initrd.img-{version}")));
                let _ = fs::remove_dir_all(fake.lab.root.join("usr/lib/modules").join(version));
                Ok(true)
            })
        }

        fn disarm(&mut self) -> Result<(), String> {
            self.call("disarm", |fake| {
                fake.note_state("disarm");
                fake.result_at_disarm
                    .push(fake.lab.state.join(RESULT_FILE).is_file());
                let _ = fs::remove_file(fake.lab.root.join("system-update"));
                Ok(())
            })
        }

        fn now(&mut self) -> i64 {
            self.call("now", |fake| fake.now)
        }

        fn say(&mut self, line: &str) {
            self.call("say", |fake| fake.said.push(line.to_owned()));
        }

        fn restart(&mut self) {
            self.call("restart", |_| ());
        }
    }

    /// One boot.
    fn boot(fake: &mut Fake<'_>) -> End {
        let lab = fake.lab;
        fake.count = 0;
        apply(
            &Paths {
                state_dir: &lab.state,
                esp: &lab.esp,
                root: &lab.root,
            },
            fake,
        )
    }

    /// One boot in which the power goes at `cut`. `None`: it went.
    fn boot_cut(fake: &mut Fake<'_>, cut: Cut) -> Option<End> {
        fake.cut = Some(cut);
        let end = catch_unwind(AssertUnwindSafe(|| boot(fake)));
        fake.cut = None;
        match end {
            Ok(end) => Some(end),
            Err(payload) => {
                assert!(payload.is::<PowerCut>(), "a panic that isn't the power cut");
                None
            }
        }
    }

    /// Boots until the restore is over.
    fn boot_to_the_end(fake: &mut Fake<'_>) -> End {
        for _ in 0..8 {
            match boot(fake) {
                End::Retry { .. } => {}
                end => return end,
            }
        }
        panic!("the restore never ended");
    }

    fn step_states(fake: &Fake<'_>, at: &str) -> Vec<State> {
        fake.states
            .iter()
            .filter(|(name, _)| *name == at)
            .map(|(_, state)| *state)
            .collect()
    }

    fn state(attempts: u32, step: Step, problems: bool) -> State {
        State {
            attempts,
            written: attempts > 0,
            step,
            problems,
        }
    }

    fn passed(lab: &Lab) -> Result<Checked, CheckFailure> {
        esp::check(&lab.esp, &lab.root, UUID)
    }

    fn restored_boot_files() -> Result<Checked, CheckFailure> {
        Ok(Checked {
            version: OLD.to_owned(),
            previous: Previous::Good {
                version: OLDER.to_owned(),
            },
        })
    }

    // ---- the whole way ----

    #[test]
    fn a_restore_runs_the_steps_in_order_and_ends_done() {
        let lab = armed("apply-done");
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));

        let steps: Vec<_> = fake
            .calls
            .iter()
            .copied()
            .filter(|call| *call != "say")
            .collect();
        assert_eq!(
            steps,
            [
                "is_armed",
                "open_backup",
                "copy",
                "back_up_esp",
                "refresh_boot",
                "remove_protected_kernel",
                "now",
                "disarm",
                "restart",
            ]
        );
        assert_eq!(
            report(&lab),
            Report {
                outcome: Outcome::Done,
                snapshot: Some(SNAPSHOT.to_owned()),
                safety_snapshot: Some(SAFETY.to_owned()),
                home: Home::Keep,
                message: String::new(),
                when: Some(NOW),
            }
        );
        // The ESP boots the snapshot's kernel, and the kernel from before is gone.
        assert_eq!(passed(&lab), restored_boot_files());
        assert!(!lab.root.join("usr/lib/modules").join(NEW).exists());
        assert!(!is_linked(&lab));
        assert!(!lab.state.join(BACKUP_DIR).exists());
        assert_eq!(State::load(&lab.state).unwrap(), state(1, Step::End, false));
    }

    /// Item 1: what a step depends on is on disk before the step runs.
    #[test]
    fn the_state_is_saved_before_each_step_that_depends_on_it() {
        let lab = armed("apply-state-first");
        let mut fake = Fake::new(&lab);
        boot(&mut fake);
        assert_eq!(step_states(&fake, "copy"), [state(1, Step::Copy, false)]);
        assert_eq!(
            step_states(&fake, "back_up_esp"),
            [state(1, Step::BootFiles, false)]
        );
        assert_eq!(
            step_states(&fake, "refresh_boot"),
            [state(1, Step::BootFiles, false)]
        );
        // The result and the end are saved before the link goes.
        assert_eq!(step_states(&fake, "disarm"), [state(1, Step::End, false)]);
        assert_eq!(fake.result_at_disarm, [true]);
    }

    #[test]
    fn vanished_files_are_no_problem_and_unwritable_ones_are_reported() {
        let lab = armed("apply-24");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(24)].into();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));

        let lab = armed("apply-23");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(23)].into();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Problems));
        assert_eq!(
            step_states(&fake, "refresh_boot"),
            [state(1, Step::BootFiles, true)]
        );
        assert_eq!(
            report(&lab).message,
            "some files couldn't be written or deleted; see /var/lib/apsis/restore/rsync-log"
        );
        assert_eq!(passed(&lab), restored_boot_files());
    }

    // ---- never started ----

    #[test]
    fn a_missing_backup_disk_never_starts_and_isnt_an_attempt() {
        let lab = armed("apply-not-started");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.backup = Err("the backup disk wasn't found".to_owned());
        assert_eq!(boot(&mut fake), End::Finished(Outcome::NotStarted));
        assert_eq!(fake.count_of("copy"), 0);
        assert_eq!(fake.count_of("back_up_esp"), 0);
        let report = report(&lab);
        assert_eq!(report.outcome, Outcome::NotStarted);
        assert_eq!(report.message, "the backup disk wasn't found");
        assert_eq!(State::load(&lab.state).unwrap(), state(0, Step::End, false));
        assert!(!is_linked(&lab));
        assert_eq!(lab.esp_tree(), before);
        assert_eq!(fake.count_of("restart"), 1);
    }

    /// PLAN 6b.10: the disk was pulled during a copy and is still missing.
    #[test]
    fn never_started_after_a_broken_copy_is_a_restore_that_didnt_finish() {
        let lab = armed("apply-not-started-written");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(12)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        fake.backup = Err("the backup disk wasn't found".to_owned());
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(fake.count_of("copy"), 1);
        assert_eq!(report(&lab).message, "the backup disk wasn't found");
        assert!(!is_linked(&lab));
    }

    // ---- the copy ----

    #[test]
    fn a_broken_copy_counts_keeps_the_link_and_is_tried_again() {
        let lab = armed("apply-retry");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(11)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert!(is_linked(&lab));
        assert!(!lab.state.join(RESULT_FILE).exists());
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::Copy, false)
        );
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("disarm"), 0);
        assert_eq!(fake.count_of("restart"), 1);

        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(
            step_states(&fake, "copy"),
            [state(1, Step::Copy, false), state(2, Step::Copy, false)]
        );
        assert_eq!(State::load(&lab.state).unwrap(), state(2, Step::End, false));
    }

    /// A copy that a signal ended, or that never ran, has no exit code.
    #[test]
    fn a_copy_without_an_exit_code_broke() {
        let lab = armed("apply-no-exit");
        let mut fake = Fake::new(&lab);
        fake.exits = [None].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
    }

    #[test]
    fn the_third_broken_copy_gives_up_with_the_boot_files_untouched() {
        let lab = armed("apply-failed");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(11), Some(12), Some(11)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert_eq!(boot(&mut fake), End::Retry { attempt: 3 });
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(fake.count_of("copy"), 3);
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        let report = report(&lab);
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(
            report.message,
            "the copy broke on each of 3 tries: rsync error: code Some(11)"
        );
        assert_eq!(State::load(&lab.state).unwrap(), state(3, Step::End, false));
        assert!(!is_linked(&lab));
        assert_eq!(lab.esp_tree(), before);
        // The kernel the ESP boots is whole (rule 10).
        assert!(esp::boots_kernel(&lab.esp, &lab.root, UUID, NEW));
    }

    /// The power went during the third copy: no fourth is started.
    #[test]
    fn a_third_copy_that_the_power_cut_gives_up_without_a_fourth() {
        let lab = armed("apply-failed-cut");
        state(3, Step::Copy, false).save(&lab.state).unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(fake.count_of("copy"), 0);
        assert_eq!(
            report(&lab).message,
            "the copy was cut off on each of 3 tries"
        );
        assert!(!is_linked(&lab));
    }

    /// Item 1: no copy without its attempt on disk first.
    #[test]
    fn a_state_that_cant_be_saved_stops_before_the_copy() {
        let lab = armed("apply-state-unsaved");
        // The temporary file's name is taken by a folder: every save fails.
        fs::create_dir(lab.state.join(format!("{STATE_FILE}.apsis-tmp"))).unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::NotStarted));
        assert_eq!(fake.count_of("copy"), 0);
        assert!(
            report(&lab)
                .message
                .starts_with("state.json couldn't be saved")
        );
        assert!(!is_linked(&lab));
    }

    // ---- the boot files ----

    #[test]
    fn a_failed_boot_refresh_puts_the_esp_back_and_keeps_the_kernel() {
        let lab = armed("apply-boot-kept");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Fails;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("put_back_esp"), 1);
        assert_eq!(fake.count_of("remove_protected_kernel"), 0);
        assert_eq!(lab.esp_tree(), before);
        assert!(esp::boots_kernel(&lab.esp, &lab.root, UUID, NEW));
        assert_eq!(
            report(&lab).message,
            "the boot refresh failed (kernelstub exited 1); the boot files from before were put \
             back"
        );
        assert!(!is_linked(&lab));
        assert!(!lab.state.join(BACKUP_DIR).exists());
    }

    /// Both commands exited 0, and the ESP still isn't what `/boot` links to: the check is
    /// against the restored tree.
    #[test]
    fn a_refresh_that_exits_0_but_fails_the_check_is_put_back() {
        let lab = armed("apply-check-fails");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Wrong;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(lab.esp_tree(), before);
        assert_eq!(fake.count_of("remove_protected_kernel"), 0);
        assert_eq!(
            report(&lab).message,
            "the new boot files didn't check out (the kernel on the ESP isn't the one /boot \
             links to); the boot files from before were put back"
        );
    }

    #[test]
    fn a_put_back_that_fails_ends_boot_broken() {
        let lab = armed("apply-boot-broken");
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Fails;
        fake.break_put_back = true;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootBroken));
        assert_eq!(fake.count_of("remove_protected_kernel"), 0);
        assert_eq!(
            report(&lab).message,
            "the boot refresh failed (kernelstub exited 1); the boot files from before couldn't \
             be put back (initrd.img doesn't match)"
        );
        assert!(!is_linked(&lab));
    }

    /// A put-back that says it worked is still checked: `boot-kept` means the ESP boots the
    /// kernel from before, whole.
    #[test]
    fn a_put_back_to_a_kernel_that_is_gone_ends_boot_broken() {
        let lab = armed("apply-put-back-gone");
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Fails;
        // Something took the kept kernel's modules.
        fs::remove_dir(lab.root.join("usr/lib/modules").join(NEW)).unwrap();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootBroken));
    }

    /// The backup's own refusal is the apply's backstop: only part of the previous kernel's
    /// files on the ESP (item 4). Nothing was changed, so nothing is put back.
    #[test]
    fn an_esp_backup_that_fails_leaves_the_boot_files_alone() {
        let lab = armed("apply-backup-fails");
        fs::remove_file(lab.esp_file(BootFile::PreviousInitrd)).unwrap();
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 1);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(fake.count_of("put_back_esp"), 0);
        assert_eq!(fake.count_of("remove_protected_kernel"), 0);
        assert_eq!(lab.esp_tree(), before);
        assert_eq!(
            report(&lab).message,
            "the boot files couldn't be backed up (the ESP has only part of the previous \
             kernel's files), so they were left as they are"
        );
    }

    /// A previous pair that isn't the `.old` links' is reported and fails nothing.
    #[test]
    fn a_wrong_previous_pair_after_the_refresh_is_only_reported() {
        let lab = armed("apply-previous-wrong");
        let mut fake = PreviousWrong(Fake::new(&lab));
        let end = apply(
            &Paths {
                state_dir: &lab.state,
                esp: &lab.esp,
                root: &lab.root,
            },
            &mut fake,
        );
        assert_eq!(end, End::Finished(Outcome::Done));
        assert_eq!(
            report(&lab).message,
            "the previous kernel's boot files: the initrd on the ESP isn't the one /boot links to"
        );
        assert_eq!(fake.0.count_of("put_back_esp"), 0);
    }

    fn arm_again(lab: &Lab) {
        let _ = fs::remove_file(lab.root.join("system-update"));
        std::os::unix::fs::symlink(&lab.state, lab.root.join("system-update")).unwrap();
    }

    /// A runner whose boot refresh works, but writes another initrd for the previous pair.
    struct PreviousWrong<'a>(Fake<'a>);

    impl Runner for PreviousWrong<'_> {
        fn is_armed(&mut self) -> bool {
            self.0.is_armed()
        }
        fn open_backup(&mut self, plan: &Plan) -> Result<(), String> {
            self.0.open_backup(plan)
        }
        fn copy(&mut self, plan: &Plan) -> Copied {
            self.0.copy(plan)
        }
        fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError> {
            self.0.back_up_esp(plan)
        }
        fn refresh_boot(&mut self) -> Result<(), String> {
            let done = self.0.refresh_boot();
            let lab = self.0.lab;
            fs::write(lab.esp_file(BootFile::PreviousInitrd), initrd(NEW)).unwrap();
            done
        }
        fn put_back_esp(&mut self, plan: &Plan) -> Result<(), EspError> {
            self.0.put_back_esp(plan)
        }
        fn remove_protected_kernel(&mut self, plan: &Plan) -> Result<bool, String> {
            self.0.remove_protected_kernel(plan)
        }
        fn disarm(&mut self) -> Result<(), String> {
            self.0.disarm()
        }
        fn now(&mut self) -> i64 {
            self.0.now()
        }
        fn say(&mut self, line: &str) {
            self.0.say(line);
        }
        fn restart(&mut self) {
            self.0.restart();
        }
    }

    /// Step 7 is the runner's to decide from the snapshot, but never for a kernel the ESP
    /// boots, and never without a passed check.
    #[test]
    fn the_kept_kernel_is_only_removed_when_the_esp_doesnt_boot_it() {
        // The snapshot has the running kernel: the runner is asked, and removes nothing.
        let lab = armed("apply-kernel-in-snapshot");
        let mut fake = Fake::new(&lab);
        fake.snapshot_has_running_kernel = true;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(fake.count_of("remove_protected_kernel"), 1);
        assert!(lab.root.join("usr/lib/modules").join(NEW).is_dir());

        // The restored system still boots the running kernel: not even asked.
        let lab = armed("apply-kernel-still-booted");
        let mut fake = SnapshotOfNow(Fake::new(&lab));
        let end = apply(
            &Paths {
                state_dir: &lab.state,
                esp: &lab.esp,
                root: &lab.root,
            },
            &mut fake,
        );
        assert_eq!(end, End::Finished(Outcome::Done));
        assert_eq!(fake.0.count_of("remove_protected_kernel"), 0);
        assert!(lab.root.join("usr/lib/modules").join(NEW).is_dir());
    }

    /// A runner whose snapshot is of the system as it is: the copy changes no kernel.
    struct SnapshotOfNow<'a>(Fake<'a>);

    impl Runner for SnapshotOfNow<'_> {
        fn is_armed(&mut self) -> bool {
            self.0.is_armed()
        }
        fn open_backup(&mut self, plan: &Plan) -> Result<(), String> {
            self.0.open_backup(plan)
        }
        fn copy(&mut self, _plan: &Plan) -> Copied {
            self.0.calls.push("copy");
            Copied {
                exit: Some(0),
                tail: String::new(),
            }
        }
        fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError> {
            self.0.back_up_esp(plan)
        }
        fn refresh_boot(&mut self) -> Result<(), String> {
            self.0.refresh_boot()
        }
        fn put_back_esp(&mut self, plan: &Plan) -> Result<(), EspError> {
            self.0.put_back_esp(plan)
        }
        fn remove_protected_kernel(&mut self, plan: &Plan) -> Result<bool, String> {
            self.0.remove_protected_kernel(plan)
        }
        fn disarm(&mut self) -> Result<(), String> {
            self.0.disarm()
        }
        fn now(&mut self) -> i64 {
            self.0.now()
        }
        fn say(&mut self, line: &str) {
            self.0.say(line);
        }
        fn restart(&mut self) {
            self.0.restart();
        }
    }

    #[test]
    fn a_kept_kernel_that_cant_be_removed_is_a_problem_not_a_failure() {
        let lab = armed("apply-kernel-stays");
        let mut fake = Fake::new(&lab);
        fake.fail_kernel_removal = true;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Problems));
        assert_eq!(
            report(&lab).message,
            "the kernel from before the restore couldn't be removed (Read-only file system)"
        );
        assert_eq!(passed(&lab), restored_boot_files());
    }

    // ---- power cuts ----

    /// Item 2: the power goes before and after every single thing the runner is asked, in
    /// the first boot. The boots after it end where an uncut restore ends.
    fn cut_everywhere(
        name: &str,
        setup: impl Fn(&mut Fake<'_>),
        expected: Outcome,
        check: impl Fn(&Lab, &Fake<'_>, Cut),
    ) {
        let asked = {
            let lab = armed(&format!("{name}-uncut"));
            let mut fake = Fake::new(&lab);
            setup(&mut fake);
            assert_eq!(boot(&mut fake), End::Finished(expected));
            fake.count
        };
        assert!(asked > 5, "{asked}");
        for index in 0..asked {
            for cut in [Cut::Before(index), Cut::After(index)] {
                let lab = armed(&format!("{name}-{cut:?}"));
                let mut fake = Fake::new(&lab);
                setup(&mut fake);
                assert_eq!(boot_cut(&mut fake, cut), None, "{cut:?}");
                let end = boot_to_the_end(&mut fake);
                assert!(
                    end == End::Finished(expected) || end == End::NotArmed,
                    "{cut:?}: {end:?}"
                );
                assert_eq!(report(&lab).outcome, expected, "{cut:?}");
                assert!(!is_linked(&lab), "{cut:?}");
                check(&lab, &fake, cut);
            }
        }
    }

    #[test]
    fn a_power_cut_at_any_step_of_a_good_restore_still_ends_done() {
        cut_everywhere(
            "apply-cut-done",
            |_| {},
            Outcome::Done,
            |lab, fake, cut| {
                assert_eq!(passed(lab), restored_boot_files(), "{cut:?}");
                assert!(!lab.root.join("usr/lib/modules").join(NEW).exists());
                // The backup is of the ESP from before the restore: taken once, never again.
                assert_eq!(fake.count_of("back_up_esp"), 1, "{cut:?}");
                // A copy that ended isn't run again; one that was cut is, as a new attempt.
                assert!(fake.count_of("copy") <= 2, "{cut:?}");
            },
        );
    }

    #[test]
    fn a_power_cut_at_any_step_of_a_failed_boot_refresh_still_ends_boot_kept() {
        cut_everywhere(
            "apply-cut-kept",
            |fake| fake.refresh = Refresh::Fails,
            Outcome::BootKept,
            |lab, fake, cut| {
                assert!(esp::boots_kernel(&lab.esp, &lab.root, UUID, NEW), "{cut:?}");
                assert_eq!(
                    fs::read_to_string(lab.esp_file(BootFile::PreviousKernel)).unwrap(),
                    kernel(OLD),
                    "{cut:?}"
                );
                assert_eq!(fake.count_of("back_up_esp"), 1, "{cut:?}");
                assert_eq!(fake.count_of("remove_protected_kernel"), 0, "{cut:?}");
            },
        );
    }

    #[test]
    fn a_power_cut_at_any_step_of_a_restore_that_never_starts_still_ends_not_started() {
        cut_everywhere(
            "apply-cut-not-started",
            |fake| fake.backup = Err("the backup disk wasn't found".to_owned()),
            Outcome::NotStarted,
            |_, fake, cut| assert_eq!(fake.count_of("copy"), 0, "{cut:?}"),
        );
    }

    #[test]
    fn a_power_cut_at_any_step_of_a_restore_with_problems_still_ends_with_problems() {
        cut_everywhere(
            "apply-cut-problems",
            // Every copy has the same unwritable files.
            |fake| fake.exits = [Some(23), Some(23)].into(),
            Outcome::Problems,
            |lab, _, cut| assert_eq!(passed(lab), restored_boot_files(), "{cut:?}"),
        );
    }

    /// The end is saved before the cleanup: a boot that finds it only cleans up.
    #[test]
    fn a_boot_after_the_result_was_saved_only_cleans_up() {
        let lab = armed("apply-end-resume");
        let mut fake = Fake::new(&lab);
        boot(&mut fake);
        arm_again(&lab);
        let calls = fake.calls.len();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        let again: Vec<_> = fake.calls[calls..]
            .iter()
            .copied()
            .filter(|call| *call != "say")
            .collect();
        assert_eq!(again, ["is_armed", "disarm", "restart"]);
        assert!(!is_linked(&lab));
    }

    // ---- the ESP backup after a power cut (item 3) ----

    /// The power went after the boot refresh had changed the ESP. The backup of before is
    /// kept, not taken again over the changed ESP, and it's what's put back.
    #[test]
    fn a_backup_that_verifies_is_kept_after_a_power_cut() {
        let lab = armed("apply-backup-kept");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Fails;
        let index = index_of(
            "apply-backup-kept",
            |fake| fake.refresh = Refresh::Fails,
            "refresh_boot",
        );
        assert_eq!(boot_cut(&mut fake, Cut::After(index)), None);
        assert_ne!(lab.esp_tree(), before);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 1);
        assert_eq!(lab.esp_tree(), before);
    }

    /// Which thing asked in an uncut boot is the first `call`.
    fn index_of(name: &str, setup: impl Fn(&mut Fake<'_>), call: &str) -> usize {
        let probe = armed(&format!("{name}-probe"));
        let mut fake = Fake::new(&probe);
        setup(&mut fake);
        boot(&mut fake);
        fake.calls.iter().position(|asked| *asked == call).unwrap()
    }

    /// A lab stopped where the power went during or after the ESP backup: the copy has
    /// ended, and `esp-backup/` is there.
    fn cut_with_a_backup(name: &str) -> Lab {
        let lab = armed(name);
        let mut fake = Fake::new(&lab);
        let at = index_of(name, |_| {}, "back_up_esp");
        assert_eq!(boot_cut(&mut fake, Cut::After(at)), None);
        assert!(esp::verify(&lab.state).is_ok());
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::BootFiles, false)
        );
        lab
    }

    /// The power went while the backup was being made: no manifest. The copy has run, so
    /// the backup isn't taken again, and without one the boot files aren't touched.
    #[test]
    fn a_backup_without_a_manifest_is_never_retaken_and_the_esp_is_left_alone() {
        let lab = cut_with_a_backup("apply-backup-partial");
        fs::remove_file(lab.state.join(BACKUP_DIR).join(MANIFEST_FILE)).unwrap();
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(fake.count_of("put_back_esp"), 0);
        assert_eq!(fake.count_of("remove_protected_kernel"), 0);
        assert_eq!(lab.esp_tree(), before);
        assert_eq!(
            report(&lab).message,
            "the backup of the boot files couldn't be verified after an interruption, so they \
             were left as they are; the kernel from before the restore still boots"
        );
    }

    /// A backup that doesn't verify, and an ESP the refresh had already finished: it boots
    /// the restored kernel, and that's a restore that's done.
    #[test]
    fn a_damaged_backup_with_a_finished_refresh_ends_done() {
        let lab = cut_with_a_backup("apply-backup-damaged-done");
        fs::write(lab.state.join(BACKUP_DIR).join("initrd.img"), "damaged").unwrap();
        lab.kernelstub(OLD);
        lab.kernelstub_previous(OLDER);
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(fake.count_of("remove_protected_kernel"), 1);
        assert_eq!(
            report(&lab).message,
            "the backup of the boot files couldn't be verified after an interruption; the boot \
             files check out as they are"
        );
    }

    /// A backup that doesn't verify, and an ESP that boots neither the restored kernel nor
    /// the one from before: nothing to put back.
    #[test]
    fn a_damaged_backup_with_a_half_refreshed_esp_ends_boot_broken() {
        let lab = cut_with_a_backup("apply-backup-damaged-broken");
        fs::write(lab.state.join(BACKUP_DIR).join("initrd.img"), "damaged").unwrap();
        fs::write(lab.esp_file(BootFile::Kernel), "half a kernel").unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootBroken));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(fake.count_of("put_back_esp"), 0);
        assert_eq!(fake.count_of("remove_protected_kernel"), 0);
    }

    /// A backup of another installation is no backup of this one.
    #[test]
    fn a_backup_of_another_root_counts_as_unverified() {
        let lab = cut_with_a_backup("apply-backup-other");
        let manifest = lab.state.join(BACKUP_DIR).join(MANIFEST_FILE);
        let text = fs::read_to_string(&manifest).unwrap();
        fs::write(
            &manifest,
            text.replace(UUID, "99999999-2222-3333-4444-555555555555"),
        )
        .unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
    }

    // ---- the arm check (item 6) ----

    #[test]
    fn without_the_link_nothing_is_read_or_reported() {
        let lab = armed("apply-not-armed");
        fs::remove_file(lab.root.join("system-update")).unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::NotArmed);
        assert_eq!(fake.calls, ["is_armed", "say", "disarm", "restart"]);
        assert!(!lab.state.join(RESULT_FILE).exists());
        assert_eq!(State::load(&lab.state).unwrap(), State::default());

        // A link that isn't Apsis's is removed, and that's all.
        std::os::unix::fs::symlink("/var/lib/system-update", lab.root.join("system-update"))
            .unwrap();
        assert_eq!(boot(&mut fake), End::NotArmed);
        assert!(!is_linked(&lab));
        assert!(!lab.state.join(RESULT_FILE).exists());
    }

    #[test]
    fn a_missing_plan_disarms_and_reports_why_without_a_snapshot() {
        let lab = armed("apply-no-plan");
        fs::remove_file(lab.state.join(plan::FILE)).unwrap();
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(
            fake.calls
                .iter()
                .copied()
                .filter(|call| *call != "say")
                .collect::<Vec<_>>(),
            ["is_armed", "now", "disarm", "restart"]
        );
        let report = report(&lab);
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.snapshot, None);
        assert_eq!(report.safety_snapshot, None);
        assert_eq!(report.when, Some(NOW));
        assert!(
            report
                .message
                .starts_with("the restore's plan couldn't be read (")
                && report.message.ends_with("); nothing was written"),
            "{}",
            report.message
        );
        assert!(!is_linked(&lab));
        assert_eq!(lab.esp_tree(), before);
    }

    #[test]
    fn an_invalid_plan_disarms_and_reports_why() {
        let lab = armed("apply-bad-plan");
        let text = fs::read_to_string(lab.state.join(plan::FILE)).unwrap();
        fs::write(lab.state.join(plan::FILE), text.replace(SNAPSHOT, "newest")).unwrap();
        // An earlier boot had started a copy.
        state(1, Step::Copy, false).save(&lab.state).unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(fake.count_of("open_backup"), 0);
        assert_eq!(fake.count_of("copy"), 0);
        let report = report(&lab);
        assert_eq!(report.snapshot, None);
        assert_eq!(
            report.message,
            "the restore's plan couldn't be read (request.json: \"snapshot\" isn't a snapshot \
             name); the system may be partly restored"
        );
        assert!(!is_linked(&lab));
    }

    #[test]
    fn a_missing_or_invalid_state_disarms_and_reports_why() {
        for (name, text) in [("missing", None), ("invalid", Some("{}"))] {
            let lab = armed(&format!("apply-state-{name}"));
            match text {
                None => fs::remove_file(lab.state.join(STATE_FILE)).unwrap(),
                Some(text) => fs::write(lab.state.join(STATE_FILE), text).unwrap(),
            }
            let mut fake = Fake::new(&lab);
            assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed), "{name}");
            assert_eq!(fake.count_of("open_backup"), 0);
            assert_eq!(fake.count_of("copy"), 0);
            let report = report(&lab);
            assert_eq!(report.snapshot.as_deref(), Some(SNAPSHOT));
            assert!(
                report
                    .message
                    .starts_with("the restore's state couldn't be read (")
                    && report
                        .message
                        .ends_with("); whether anything was written isn't known"),
                "{}",
                report.message
            );
            assert!(!is_linked(&lab));
        }
    }

    // ---- no clock (item 7) ----

    /// The apply compares no time with any clock: a plan from 1970, or from the future of a
    /// clock that's hours off, is applied.
    #[test]
    fn a_plan_of_any_age_is_applied() {
        for (name, prepared_at, now) in [
            ("ancient", 1, NOW),
            ("future", NOW + 12 * 3600, NOW),
            ("clock-reset", 1_790_000_000, 86_400),
        ] {
            let lab = lab(&format!("apply-age-{name}"));
            Plan {
                prepared_at,
                ..plan()
            }
            .save(&lab.state)
            .unwrap();
            State::default().save(&lab.state).unwrap();
            arm_again(&lab);
            let mut fake = Fake::new(&lab);
            fake.now = now;
            assert_eq!(boot(&mut fake), End::Finished(Outcome::Done), "{name}");
            assert_eq!(report(&lab).when, Some(now));
            // The clock is asked once, for the result.
            assert_eq!(fake.count_of("now"), 1);
        }
    }

    // ---- the minimal report (item 8) ----

    /// A clock that gives no time: the real result is refused, and the minimal one is
    /// written in its place.
    #[test]
    fn a_refused_result_is_replaced_by_the_minimal_report() {
        let lab = armed("apply-minimal");
        let mut fake = Fake::new(&lab);
        fake.now = 0;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(
            report(&lab),
            Report {
                outcome: Outcome::Failed,
                snapshot: Some(SNAPSHOT.to_owned()),
                safety_snapshot: Some(SAFETY.to_owned()),
                home: Home::Keep,
                message: "result could not be saved, see journal (the restore ended: done)"
                    .to_owned(),
                when: None,
            }
        );
        // The restore itself went the whole way, and is cleaned up.
        assert_eq!(passed(&lab), restored_boot_files());
        assert!(!is_linked(&lab));
        assert!(
            fake.said
                .iter()
                .any(|line| line.contains("result.json") && line.contains("\"when\"")),
            "{:?}",
            fake.said
        );
    }

    #[test]
    fn the_minimal_report_of_an_unreadable_plan_has_neither_snapshot_nor_time() {
        let lab = armed("apply-minimal-null");
        fs::write(lab.state.join(plan::FILE), "{").unwrap();
        let mut fake = Fake::new(&lab);
        fake.now = -5;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        let report = report(&lab);
        assert_eq!((report.snapshot, report.when), (None, None));
        assert_eq!(
            report.message,
            "result could not be saved, see journal (the restore ended: failed)"
        );
    }
}
