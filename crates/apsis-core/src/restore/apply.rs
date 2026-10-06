// SPDX-License-Identifier: GPL-3.0-only

//! The apply: what `apsis-helper --apply-restore` does in the offline boot, as a
//! state machine over a [`Runner`].
//!
//! Everything that needs root or a real machine (the link, the backup disk, rsync, the boot
//! refresh, the restart) is the runner's. What's decided here: the order, what's saved to
//! `state.json` before which step, how a boot after a power cut goes on, and which outcome
//! each failure ends in. The plan, the state, the result and the ESP backup are read and
//! written only through [`super::plan`], [`super::state`] and [`super::esp`].
//!
//! No clock is read to decide anything: [`Runner::now`] only dates the
//! result.

use std::fs;
use std::io;
use std::path::Path;

use super::esp::{self, Checked, EspError, Manifest, Previous};
use super::file::FileError;
use super::filter::Home;
use super::plan::{self, Plan};
use super::state::{MAX_ATTEMPTS, MAX_BOOTS, Outcome, Report, State, Step};
use crate::native::Info;

/// The message of the report of last resort.
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
    /// refresh.
    pub root: &'a Path,
}

/// What rsync prints on its standard output when a read error made it stop deleting: from
/// there on, what the snapshot lacks stays.
pub const DELETIONS_SKIPPED: &str = "IO error encountered -- skipping file deletion";

/// How pass 1 ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// rsync's exit code. `None`: it didn't run, or a signal ended it.
    pub exit: Option<i32>,
    /// Its last lines, for the result of a copy that broke.
    pub tail: String,
    /// rsync said [`DELETIONS_SKIPPED`].
    pub deletions_skipped: bool,
}

/// What's at the snapshot's place on the backup disk, read right before a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFound {
    /// `snapshots/<name>/localhost/` is a folder (not followed).
    pub has_localhost: bool,
    /// The text of `snapshots/<name>/info.json`. `None`: missing or unreadable.
    pub info: Option<String>,
}

/// The check before every copy: the snapshot's folder is there, and its `info.json` is the
/// plan's snapshot's. rsync itself only exits 23 when the folder is gone, having done
/// nothing, and the apply would go on from that.
///
/// `info.json` has no name in it. What it's held against is the creation time the plan
/// recorded, the root UUID, and the type.
///
/// # Errors
///
/// Why the copy isn't started, in words for the result.
pub fn check_snapshot(plan: &Plan, found: &SnapshotFound) -> Result<(), String> {
    if !found.has_localhost {
        return Err("the snapshot's folder isn't on the backup disk".to_owned());
    }
    let Some(info) = found.info.as_deref().and_then(Info::parse) else {
        return Err("the snapshot's info.json can't be read".to_owned());
    };
    let wrong = if info.created != plan.snapshot_created {
        "isn't the one this restore was prepared for"
    } else if info.sys_uuid != plan.root_uuid {
        "is of another installation"
    } else if info.kind != "rsync" {
        "isn't an rsync snapshot"
    } else {
        return Ok(());
    };
    Err(format!("the snapshot on the backup disk {wrong}"))
}

/// What a copy's exit means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyEnd {
    /// Exit 0, or 24 (source files that vanished): the apply goes on. `problems` is exit 23
    /// by itself: some files couldn't be written or deleted.
    Ended { problems: bool },
    /// Any other exit, or none, or exit 23 with the deletions skipped: the copy broke.
    Broke,
}

impl Copied {
    /// From a finished rsync: its exit code, all of its standard output, and the last lines
    /// of its errors.
    #[must_use]
    pub fn new(exit: Option<i32>, stdout: &str, tail: String) -> Self {
        Self {
            exit,
            tail,
            deletions_skipped: stdout.lines().any(|line| line == DELETIONS_SKIPPED),
        }
    }

    #[must_use]
    pub fn end(&self) -> CopyEnd {
        match self.exit {
            Some(0 | 24) => CopyEnd::Ended { problems: false },
            // A folder of the snapshot couldn't be read: the tree isn't the snapshot's.
            Some(23) if self.deletions_skipped => CopyEnd::Broke,
            Some(23) => CopyEnd::Ended { problems: true },
            _ => CopyEnd::Broke,
        }
    }

    /// rsync's last error lines, without the empty ones: what the journal gets, one line
    /// each, after a plain exit 23 (check 6, 2026-10-03). The helper has cut them to its
    /// last 20.
    pub fn error_lines(&self) -> impl Iterator<Item = &str> {
        self.tail.lines().filter(|line| !line.trim().is_empty())
    }

    /// The copy's last lines, and that the system may be mixed if deletions were skipped.
    fn why(&self) -> String {
        if self.deletions_skipped {
            format!(
                "{}; rsync skipped its deletions after a read error, so the system may be a \
                 mix of the snapshot and what was there before",
                self.tail
            )
        } else {
            self.tail.clone()
        }
    }
}

/// What the apply needs done on the machine. The helper has the real one; the tests a fake.
pub trait Runner {
    /// `/system-update` is a link to the state folder. If it isn't, the
    /// apply touches neither the link nor anything else but Apsis's own unit files.
    fn is_armed(&mut self) -> bool;

    /// Step 2: waits for the backup disk, mounts it read-only, runs the path checks and the
    /// refusals again, and checks a separate `/home` that's restored.
    ///
    /// # Errors
    ///
    /// Why the restore can't start in this boot, in words for the result.
    fn open_backup(&mut self, plan: &Plan) -> Result<(), String>;

    /// Reads what's at the snapshot's place, for [`check_snapshot`]. Asked right before
    /// every copy, and again after a copy that exited 23 (a disk that vanished mid-copy
    /// looks like a few unreadable files to rsync; check 6, 2026-10-03).
    fn find_snapshot(&mut self, plan: &Plan) -> SnapshotFound;

    /// Step 3: pass 1, rsync over `/`. It returns only when what rsync wrote is on disk
    /// (`sync`): a copy that ended is never run again after a power cut.
    fn copy(&mut self, plan: &Plan) -> Copied;

    /// Step 4: [`esp::back_up`].
    ///
    /// # Errors
    ///
    /// As [`esp::back_up`]. The apply then leaves the boot files alone.
    fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError>;

    /// Step 5: kernelstub alone (`kernelstub --verbose --preserve-live-mode`), told the
    /// kernel and initrd that the restored `/boot/vmlinuz` and `/boot/initrd.img` link to.
    /// No initrd is rebuilt: the snapshot's are used as they are.
    ///
    /// # Errors
    ///
    /// That kernelstub didn't exit 0, and its last lines.
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

    /// Removes `/system-update`: the commit point. Only called when it's
    /// Apsis's ([`Runner::is_armed`]). A link that isn't there is fine.
    ///
    /// # Errors
    ///
    /// The link is still there. The apply then removes nothing else and never calls
    /// [`Runner::restart`]: a restart would come straight back here
    /// (`systemd.offline-updates(7)`).
    fn remove_link(&mut self) -> Result<(), String>;

    /// Removes the rest of the arm: the unit, its wants link, the drop-in (and the wants
    /// folder and the drop-in's folder once they're empty), the helper copy and
    /// `state.json`. Only called once Apsis's link is gone, or when the link was never
    /// Apsis's.
    ///
    /// # Errors
    ///
    /// What couldn't be removed. Without the link it arms nothing.
    fn remove_arm_files(&mut self) -> Result<(), String>;

    /// The clock, in Unix seconds, for the result's time. Never compared with anything.
    fn now(&mut self) -> i64;

    /// One line for the journal. Nothing of it reaches the boot screen: the real runner
    /// writes there itself, its start line and the copy's progress (check 1, 2026-10-02).
    fn say(&mut self, line: &str);

    /// Restarts the computer. Called once, last, by [`apply`] alone: with the link gone
    /// that's the normal boot, and with the link kept on purpose it's the next attempt. Not
    /// called when the link couldn't be removed ([`End::LinkStuck`]) or isn't Apsis's
    /// ([`End::NotArmed`]).
    fn restart(&mut self);
}

/// How one boot's apply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// `/system-update` isn't a link to Apsis's state folder: another tool's update, or no
    /// link. Nothing was read, written or reported, the link is as it was, and
    /// [`Runner::restart`] wasn't called (`systemd.offline-updates(7)`, point 5).
    NotArmed,
    /// The copy broke. The link stays, and the next boot is this attempt.
    Retry { attempt: u32 },
    /// The restore is over, with this outcome in `result.json`.
    Finished(Outcome),
    /// [`MAX_BOOTS`] boots began this restore and none ended it: it's given up. The link is
    /// removed and `result.json` says `failed`. The machine is restarted: with the link
    /// gone that's a normal boot.
    GaveUp,
    /// The restore is over with this outcome in `result.json`, but `/system-update` couldn't
    /// be removed. [`Runner::restart`] wasn't called: the helper exits cleanly, and what
    /// happens to the link next is systemd's (`systemd.offline-updates(7)`, point 7).
    LinkStuck { outcome: Outcome },
}

impl End {
    /// How a restore that's over ended, by whether the link is gone.
    fn over(link_gone: bool, outcome: Outcome) -> Self {
        if link_gone {
            Self::Finished(outcome)
        } else {
            Self::LinkStuck { outcome }
        }
    }
}

/// What `apsis-helper --apply-restore` exits with, from how the apply ended and whether
/// [`Runner::restart`]'s call (`systemctl reboot --no-block`) went through. The unit has
/// `FailureAction=reboot`, so exit 1 is a restart by systemd:
///
/// - [`End::Finished`] and [`End::GaveUp`]: the link is gone and `system-update-cleanup` is
///   skipped, so if the reboot call failed nothing else would restart: exit 1. With the call
///   through, exit 0.
/// - [`End::Retry`]: the same; the attempt is on disk, so systemd's restart is the retry the
///   apply wanted. The one bounded exception to "never exit non-zero with the link in place".
/// - [`End::LinkStuck`] and [`End::NotArmed`]: no restart was called; exit 0 whatever
///   `restart_failed` says, so nothing restarts over a link that's stuck or another tool's.
#[must_use]
pub fn exit_code(end: End, restart_failed: bool) -> u8 {
    match end {
        End::Finished(_) | End::GaveUp | End::Retry { .. } => u8::from(restart_failed),
        End::LinkStuck { .. } | End::NotArmed => 0,
    }
}

/// Runs the apply for this boot, then restarts, unless the link is stuck or isn't Apsis's.
///
/// Whether to restart is decided here, not by the helper. The rule: restart whenever the
/// link is gone (it can only reach a normal boot), and over the link only as a retry. An
/// attempt is on disk before each copy and there are [`MAX_ATTEMPTS`] at most, so that's
/// two restarts over the link in all. A link that can't be removed gets none.
pub fn apply(paths: &Paths<'_>, runner: &mut impl Runner) -> End {
    let end = run(paths, runner);
    if matches!(end, End::Retry { .. } | End::Finished(_) | End::GaveUp) {
        runner.restart();
    }
    end
}

fn run(paths: &Paths<'_>, runner: &mut impl Runner) -> End {
    let dir = paths.state_dir;

    // Step 1: the arm check. A link that isn't Apsis's is another tool's update: it's left
    // alone. What made this run without Apsis's link is a leftover of an arm, and goes.
    if !runner.is_armed() {
        runner.say("no restore is armed: /system-update isn't Apsis's link, leaving it alone");
        if let Err(error) = runner.remove_arm_files() {
            runner.say(&format!(
                "the restore's unit files weren't removed: {error}"
            ));
        }
        return End::NotArmed;
    }
    // Before the boot is counted, nothing runs but the link check above and this read.
    let mut state = match State::load(dir) {
        Ok(state) => state,
        Err(error) => {
            // Nothing can be counted, so the restore ends here and the link goes.
            let plan = Plan::load(dir).ok();
            let message = format!(
                "the restore's state couldn't be read ({error}); whether anything was written \
                 isn't known"
            );
            return finish(paths, runner, plan.as_ref(), None, Outcome::Failed, message);
        }
    };
    if state.boots >= MAX_BOOTS {
        return give_up(paths, runner, state);
    }
    // The apply's first write: this boot, counted before any other work.
    let counted = State {
        boots: state.boots + 1,
        ..state
    };
    match counted.save(dir) {
        Ok(()) => state = counted,
        // The result is there already: the cleanup below needs no count.
        Err(error) if state.step == Step::End => {
            runner.say(&format!("state.json wasn't saved: {error}"));
        }
        Err(error) => {
            // A boot that can't be counted does nothing else.
            let plan = Plan::load(dir).ok();
            let message =
                format!("state.json couldn't be saved ({error}), so nothing was done in this boot");
            let outcome = unstarted(state);
            return finish(paths, runner, plan.as_ref(), Some(state), outcome, message);
        }
    }
    let plan = match Plan::load(dir) {
        Ok(plan) => plan,
        Err(error) => {
            let written = if state.written {
                "the system may be partly restored"
            } else {
                "nothing was written"
            };
            let message = format!("the restore's plan couldn't be read ({error}); {written}");
            return finish(paths, runner, None, Some(state), Outcome::Failed, message);
        }
    };
    // The result is written: an earlier boot got that far, and only the cleanup is left.
    if state.step == Step::End {
        return match Report::load(dir) {
            Ok(report) => End::over(clean_up(paths, runner), report.outcome),
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
        // The snapshot, right before the copy: gone or another one, and nothing is touched.
        if let Err(reason) = check_snapshot(&plan, &runner.find_snapshot(&plan)) {
            let outcome = unstarted(state);
            return finish(paths, runner, Some(&plan), Some(state), outcome, reason);
        }
        let copying = State {
            attempts: state.attempts + 1,
            written: true,
            step: Step::Copy,
            problems: false,
            ..state
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
        runner.say(&format!(
            "the copy ended: rsync {}",
            copied
                .exit
                .map_or_else(|| "didn't exit".to_owned(), |code| format!("exited {code}"))
        ));
        // A plain 23 is also what rsync 3.2.7 gives when the whole backup disk vanishes
        // under it: its "skipping file deletion" line comes only from a later folder's
        // deletion pass, and once the walk collapses none comes (check 6, 2026-10-03). So
        // after a plain 23 the snapshot is looked at once more: gone, the copy broke.
        let broke = match copied.end() {
            CopyEnd::Ended { problems: false } => Ok(false),
            CopyEnd::Ended { problems: true } => {
                match check_snapshot(&plan, &runner.find_snapshot(&plan)) {
                    Ok(()) => {
                        // Which files, in rsync's own words: the result names the log
                        // only. Here alone: a copy that broke has them in its message.
                        for line in copied.error_lines() {
                            runner.say(&format!("rsync stderr: {line}"));
                        }
                        Ok(true)
                    }
                    Err(reason) => Err(format!(
                        "{}; after the copy, {reason}: the disk went away during the copy, \
                         so the system may be a mix of the snapshot and what was there before",
                        copied.tail
                    )),
                }
            }
            CopyEnd::Broke => Err(copied.why()),
        };
        let problems = match broke {
            Ok(problems) => problems,
            Err(why) if state.attempts >= MAX_ATTEMPTS => {
                let message = format!("the copy broke on each of {MAX_ATTEMPTS} tries: {why}");
                return finish(
                    paths,
                    runner,
                    Some(&plan),
                    Some(state),
                    Outcome::Failed,
                    message,
                );
            }
            Err(why) => {
                let attempt = state.attempts + 1;
                runner.say(&format!(
                    "the copy broke ({why}): restarting to try again, attempt {attempt} of \
                     {MAX_ATTEMPTS}"
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
    let (outcome, message) = boot_files(paths, runner, &plan, &mut state);
    finish(paths, runner, Some(&plan), Some(state), outcome, message)
}

/// The cap: [`MAX_BOOTS`] boots began and none ended, so something stops the helper in each
/// of them. The link goes first, before anything that could stop it again; then the result.
/// [`apply`] restarts after it only if the link is gone. The ESP backup stays, for a
/// recovery by hand.
fn give_up(paths: &Paths<'_>, runner: &mut impl Runner, state: State) -> End {
    let Paths {
        state_dir,
        esp,
        root,
    } = *paths;
    let link_gone = runner.remove_link().is_ok();
    let plan = Plan::load(state_dir).ok();
    let mut message = format!(
        "the restore was started in {MAX_BOOTS} boots and ended in none, so it was given up; {}",
        if state.written {
            "the system may be partly restored"
        } else {
            "nothing was written"
        }
    );
    if let Some(plan) = &plan {
        clear_temporaries(esp, plan, runner);
        // The boot refresh had started: say so if the ESP boots neither kernel whole.
        if state.step == Step::Refresh
            && esp::check(esp, root, &plan.root_uuid).is_err()
            && !esp::boots_kernel(esp, root, &plan.root_uuid, &plan.running_kernel)
        {
            message.push_str(
                "; the boot files are neither the restored kernel's nor the ones from before",
            );
        }
    }
    save_result(state_dir, runner, plan.as_ref(), Outcome::Failed, message);
    let end = State {
        step: Step::End,
        ..state
    };
    if let Err(error) = end.save(state_dir) {
        runner.say(&format!("state.json wasn't saved: {error}"));
    }
    if !link_gone {
        runner.say("/system-update couldn't be removed: not restarting");
        return End::LinkStuck {
            outcome: Outcome::Failed,
        };
    }
    if let Err(error) = runner.remove_arm_files() {
        runner.say(&format!(
            "the restore's unit files weren't removed: {error}"
        ));
    }
    remove_working_files(state_dir, runner);
    End::GaveUp
}

/// The preparation's filter and note go once the restore is over and the link is gone:
/// what the arm kept of them stays, as `last-restore.*`.
fn remove_working_files(state_dir: &Path, runner: &mut impl Runner) {
    for name in plan::WORKING_FILES {
        match fs::remove_file(state_dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => runner.say(&format!("{name} wasn't removed: {error}")),
        }
    }
}

/// Removes what a put-back that was cut left on the ESP ([`esp::clear_temporaries`]).
fn clear_temporaries(esp: &Path, plan: &Plan, runner: &mut impl Runner) {
    if let Err(error) = esp::clear_temporaries(esp, &plan.root_uuid) {
        runner.say(&format!(
            "a temporary file on the ESP wasn't removed: {error}"
        ));
    }
}

/// A restore that stops before this boot's copy: it never started, unless an earlier boot's
/// copy wrote something.
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
    state: &mut State,
) -> (Outcome, String) {
    let Paths {
        state_dir,
        esp,
        root,
    } = *paths;
    let problems = state.problems;

    // Step 4. The backup is of the boot files from before the restore, so it's only ever
    // taken while `state.json` says the boot refresh hasn't started.
    if state.step == Step::BootFiles {
        // The power went during the backup: no manifest, and the ESP is still untouched.
        match esp::remove_partial(state_dir) {
            Ok(true) => {
                runner.say("an ESP backup that was cut short is removed and taken once more");
            }
            Ok(false) => {}
            Err(error) => {
                return (
                    Outcome::BootKept,
                    format!(
                        "a partial backup of the boot files couldn't be removed ({error}), so \
                         they were left as they are"
                    ),
                );
            }
        }
    }
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
    } else if state.step != Step::BootFiles {
        runner.say("the boot refresh had started, and the ESP backup is gone");
        return unverified(paths, runner, plan, problems);
    } else if let Err(error) = runner.back_up_esp(plan) {
        return (
            Outcome::BootKept,
            format!(
                "the boot files couldn't be backed up ({error}), so they were left as they are"
            ),
        );
    }

    // From here the boot files may be changed: on disk before the refresh starts.
    if state.step != Step::Refresh {
        let refreshing = State {
            step: Step::Refresh,
            ..*state
        };
        if let Err(error) = refreshing.save(state_dir) {
            return (
                Outcome::BootKept,
                format!(
                    "state.json couldn't be saved ({error}), so the boot files were left as \
                     they are"
                ),
            );
        }
        *state = refreshing;
    }
    // A put-back that the power cut left its temporary file on the ESP: room for the refresh.
    clear_temporaries(esp, plan, runner);

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
    // No end leaves a temporary file of a put-back on the ESP.
    if let Some(plan) = plan {
        clear_temporaries(paths.esp, plan, runner);
    }
    let saved = save_result(dir, runner, plan, outcome, message);
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
    End::over(clean_up(paths, runner), saved)
}

/// Writes `result.json`, the minimal one if the real one is refused, and says how the
/// restore ended. Gives the outcome that's in the file.
fn save_result(
    dir: &Path,
    runner: &mut impl Runner,
    plan: Option<&Plan>,
    outcome: Outcome,
    message: String,
) -> Outcome {
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
            save_minimal(dir, runner, &report)
        }
    };
    runner.say(&format!(
        "the restore ended: {}{}{}",
        outcome.word(),
        if report.message.is_empty() { "" } else { ": " },
        report.message
    ));
    saved
}

/// The report of last resort, when the real one is refused: the same outcome,
/// [`MINIMAL_MESSAGE`], and no time unless the clock gave one. If that's refused too, it's
/// the snapshot's name: only `failed` may lack it, so the bare report is `failed` and its
/// message names the real outcome. Gives the outcome that's in the file.
fn save_minimal(dir: &Path, runner: &mut impl Runner, refused: &Report) -> Outcome {
    let minimal = Report {
        message: MINIMAL_MESSAGE.to_owned(),
        when: refused.when.filter(|when| *when > 0),
        ..refused.clone()
    };
    if minimal.save(dir).is_ok() {
        return minimal.outcome;
    }
    let bare = Report {
        outcome: Outcome::Failed,
        snapshot: None,
        safety_snapshot: None,
        message: format!(
            "{MINIMAL_MESSAGE} (the restore ended: {})",
            refused.outcome.word()
        ),
        ..minimal
    };
    let saved: Result<(), FileError> = bare.save(dir);
    if let Err(error) = saved {
        runner.say(&format!(
            "the minimal result.json wasn't saved either: {error}"
        ));
    }
    bare.outcome
}

/// Step 8's cleanup: the link first, then the rest of the arm and the ESP backup. Says
/// whether the link is gone. If it isn't, nothing else is removed (the arm stays whole) and
/// the caller must not restart.
fn clean_up(paths: &Paths<'_>, runner: &mut impl Runner) -> bool {
    if let Err(error) = runner.remove_link() {
        runner.say(&format!(
            "/system-update couldn't be removed ({error}): not restarting"
        ));
        return false;
    }
    if let Err(error) = runner.remove_arm_files() {
        runner.say(&format!(
            "the restore's unit files weren't removed: {error}"
        ));
    }
    // The plan went into `result.json`; what's left of it is a leftover for the helper's
    // next start (check 1, 2026-10-02).
    match fs::remove_file(paths.state_dir.join(plan::FILE)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => runner.say(&format!("request.json wasn't removed: {error}")),
    }
    remove_working_files(paths.state_dir, runner);
    if let Err(error) = esp::remove(paths.state_dir) {
        runner.say(&format!("the ESP backup wasn't removed: {error}"));
    }
    true
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::panic::{AssertUnwindSafe, catch_unwind, panic_any};

    use super::super::esp::tests::{
        Lab, NEW, OLD, OLDER, UUID, cmdline, entry, initrd, kernel, lab,
    };
    use super::super::esp::{BACKUP_DIR, BootFile, CheckFailure, MANIFEST_FILE};
    use super::super::plan;
    use super::super::state::{MAX_BOOTS, RESULT_FILE, STATE_FILE};
    use super::*;
    use crate::native::Info;

    const SNAPSHOT: &str = "2026-09-25_11-28-00";
    const SAFETY: &str = "2026-10-01_09-15-42";
    const NOW: i64 = 1_790_000_600;
    /// The snapshot's `created`, in its `info.json` and in the plan.
    const CREATED: i64 = 1_789_990_080;

    /// The lab's machine runs `NEW` (previous `OLD`). The snapshot is from before the kernel
    /// update: `OLD` (previous `OLDER`), and it doesn't have `NEW`.
    fn plan() -> Plan {
        Plan {
            snapshot: SNAPSHOT.to_owned(),
            snapshot_created: CREATED,
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

    /// The snapshot's `info.json` as a create writes it.
    fn info() -> Info {
        Info {
            created: CREATED,
            sys_uuid: UUID.to_owned(),
            sys_distro: "Pop 24.04 (noble)".to_owned(),
            app_version: "apsis 0.5.0".to_owned(),
            file_count: 1234,
            tags: vec!["ondemand".to_owned()],
            comments: String::new(),
            live: false,
            kind: "rsync".to_owned(),
            rsync_flags: Some("-aAX --numeric-ids".to_owned()),
        }
    }

    fn found(info: &Info) -> SnapshotFound {
        SnapshotFound {
            has_localhost: true,
            info: Some(info.to_text()),
        }
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

    /// What a helper that's killed, or panics, unwinds with.
    struct Killed;

    struct Fake<'a> {
        lab: &'a Lab,
        /// Everything asked of the runner, over all boots.
        calls: Vec<&'static str>,
        /// Asked in this boot.
        count: usize,
        cut: Option<Cut>,
        backup: Result<(), String>,
        /// What's at the snapshot's place on the backup disk.
        snapshot: SnapshotFound,
        /// rsync said it skipped its deletions, in every copy that exits 23.
        deletions_skipped: bool,
        /// The backup disk leaves during the first copy: from then on the snapshot isn't
        /// found, until a test puts it back.
        disk_leaves_in_copy: bool,
        /// The exit codes of the copies to come; exit 0 when it's empty.
        exits: VecDeque<Option<i32>>,
        /// rsync's last error lines in every copy; `None`: one line naming the exit code.
        tail: Option<String>,
        refresh: Refresh,
        fail_esp_backup: bool,
        break_put_back: bool,
        snapshot_has_running_kernel: bool,
        fail_kernel_removal: bool,
        /// `/system-update` can't be removed.
        stuck_link: bool,
        /// The power goes inside this step, once, with its work half done.
        cut_inside: Option<&'static str>,
        /// The helper dies whenever it gets to this, in every boot.
        die_at: Option<&'static str>,
        /// Restarts made while `/system-update` was still there: each comes back here.
        restarts_over_the_link: usize,
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
                snapshot: found(&info()),
                deletions_skipped: false,
                disk_leaves_in_copy: false,
                exits: VecDeque::new(),
                tail: None,
                refresh: Refresh::Works,
                fail_esp_backup: false,
                break_put_back: false,
                snapshot_has_running_kernel: false,
                fail_kernel_removal: false,
                stuck_link: false,
                cut_inside: None,
                die_at: None,
                restarts_over_the_link: 0,
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
            if self.die_at == Some(name) {
                panic_any(Killed);
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
            self.call("open_backup", |fake| {
                fake.note_state("open_backup");
                fake.backup.clone()
            })
        }

        fn find_snapshot(&mut self, _plan: &Plan) -> SnapshotFound {
            self.call("find_snapshot", |fake| fake.snapshot.clone())
        }

        fn copy(&mut self, _plan: &Plan) -> Copied {
            self.call("copy", |fake| {
                fake.note_state("copy");
                let exit = fake.exits.pop_front().unwrap_or(Some(0));
                if fake.disk_leaves_in_copy {
                    fake.disk_leaves_in_copy = false;
                    fake.snapshot = SnapshotFound {
                        has_localhost: false,
                        info: None,
                    };
                }
                if matches!(exit, Some(0 | 23 | 24)) {
                    // The snapshot's kernels arrive; rule 10 keeps the running one's files.
                    fake.lab.install_kernel(OLD);
                    fake.lab.install_kernel(OLDER);
                    fake.lab.link_kernel(OLD);
                    fake.lab.link_previous(OLDER);
                }
                Copied {
                    exit,
                    tail: fake
                        .tail
                        .clone()
                        .unwrap_or_else(|| format!("rsync error: code {exit:?}")),
                    deletions_skipped: fake.deletions_skipped && exit == Some(23),
                }
            })
        }

        fn back_up_esp(&mut self, plan: &Plan) -> Result<Manifest, EspError> {
            self.call("back_up_esp", |fake| {
                fake.note_state("back_up_esp");
                if fake.cut_inside == Some("back_up_esp") {
                    // Two files are copied, and there's no manifest yet.
                    fake.cut_inside = None;
                    let dir = fake.lab.state.join(BACKUP_DIR);
                    fs::create_dir(&dir).unwrap();
                    for file in [BootFile::Kernel, BootFile::Initrd] {
                        fs::copy(fake.lab.esp_file(file), dir.join(file.name())).unwrap();
                    }
                    panic_any(PowerCut);
                }
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
                if fake.cut_inside == Some("put_back_esp") {
                    // Two files are back, and the third is half written under its
                    // temporary name.
                    fake.cut_inside = None;
                    let dir = fake.lab.state.join(BACKUP_DIR);
                    for file in [BootFile::Kernel, BootFile::Initrd] {
                        fs::copy(dir.join(file.name()), fake.lab.esp_file(file)).unwrap();
                    }
                    let cmdline = fake.lab.esp_file(BootFile::Cmdline);
                    fs::write(cmdline.with_file_name("cmdline.apsis-tmp"), "half").unwrap();
                    panic_any(PowerCut);
                }
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

        fn remove_link(&mut self) -> Result<(), String> {
            self.call("remove_link", |fake| {
                fake.note_state("remove_link");
                fake.result_at_disarm
                    .push(fake.lab.state.join(RESULT_FILE).is_file());
                if fake.stuck_link {
                    return Err("Read-only file system".to_owned());
                }
                let _ = fs::remove_file(fake.lab.root.join("system-update"));
                Ok(())
            })
        }

        fn remove_arm_files(&mut self) -> Result<(), String> {
            self.call("remove_arm_files", |_| Ok(()))
        }

        fn now(&mut self) -> i64 {
            self.call("now", |fake| fake.now)
        }

        fn say(&mut self, line: &str) {
            self.call("say", |fake| fake.said.push(line.to_owned()));
        }

        fn restart(&mut self) {
            self.call("restart", |fake| {
                if is_linked(fake.lab) {
                    fake.restarts_over_the_link += 1;
                }
            });
        }
    }

    /// One boot.
    fn boot(fake: &mut Fake<'_>) -> End {
        let lab = fake.lab;
        fake.count = 0;
        let end = apply(
            &Paths {
                state_dir: &lab.state,
                esp: &lab.esp,
                root: &lab.root,
            },
            fake,
        );
        ended(lab, end)
    }

    /// The invariant of every end, on every path these tests take: whatever the apply
    /// returns, it leaves none of a put-back's temporary files on the ESP.
    fn ended(lab: &Lab, end: End) -> End {
        assert!(
            !has_temporary_files(lab),
            "{end:?} left a temporary file on the ESP: {:?}",
            lab.esp_tree()
                .iter()
                .map(|(name, _)| name)
                .filter(|name| name.ends_with(".apsis-tmp"))
                .collect::<Vec<_>>()
        );
        end
    }

    /// One boot in which the helper dies at [`Fake::die_at`]. `None`: it died.
    fn boot_or_die(fake: &mut Fake<'_>) -> Option<End> {
        match catch_unwind(AssertUnwindSafe(|| boot(fake))) {
            Ok(end) => Some(end),
            Err(payload) => {
                assert!(payload.is::<Killed>(), "a panic that isn't the kill");
                None
            }
        }
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
            // One boot for each attempt, and one for a restore that never started.
            boots: attempts.max(1),
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
        // The plan went with the arm: `result.json` has what the window needs, and nothing
        // is left for the helper's next start to remove (check 1, 2026-10-02).
        assert!(!lab.state.join(plan::FILE).exists());
        // The journal says how the copy ended (check 1, 2026-10-02: the step's timestamps
        // were all it had).
        assert!(
            fake.said
                .iter()
                .any(|l| l == "the copy ended: rsync exited 0"),
            "{:?}",
            fake.said
        );

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
                "find_snapshot",
                "copy",
                "back_up_esp",
                "refresh_boot",
                "remove_protected_kernel",
                "now",
                "remove_link",
                "remove_arm_files",
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
            [state(1, Step::Refresh, false)]
        );
        // The result and the end are saved before the link goes.
        assert_eq!(
            step_states(&fake, "remove_link"),
            [state(1, Step::End, false)]
        );
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
            [state(1, Step::Refresh, true)]
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

    /// The disk was pulled during a copy and is still missing.
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
        assert_eq!(fake.count_of("remove_link"), 0);
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
        assert_eq!(ended(fake.0.lab, end), End::Finished(Outcome::Done));
        assert_eq!(
            report(&lab).message,
            "the previous kernel's boot files: the initrd on the ESP isn't the one /boot links to"
        );
        assert_eq!(fake.0.count_of("put_back_esp"), 0);
    }

    /// The link back after an end: for real, only a link whose removal failed is still
    /// there, and the cleanup stops before the plan then, so the plan is there too.
    fn arm_again(lab: &Lab) {
        let _ = fs::remove_file(lab.root.join("system-update"));
        if !lab.state.join(plan::FILE).exists() {
            plan().save(&lab.state).unwrap();
        }
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
        fn find_snapshot(&mut self, plan: &Plan) -> SnapshotFound {
            self.0.find_snapshot(plan)
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
        fn remove_link(&mut self) -> Result<(), String> {
            self.0.remove_link()
        }
        fn remove_arm_files(&mut self) -> Result<(), String> {
            self.0.remove_arm_files()
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
        assert_eq!(ended(fake.0.lab, end), End::Finished(Outcome::Done));
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
        fn find_snapshot(&mut self, plan: &Plan) -> SnapshotFound {
            self.0.find_snapshot(plan)
        }
        fn copy(&mut self, _plan: &Plan) -> Copied {
            self.0.calls.push("copy");
            Copied {
                exit: Some(0),
                tail: String::new(),
                deletions_skipped: false,
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
        fn remove_link(&mut self) -> Result<(), String> {
            self.0.remove_link()
        }
        fn remove_arm_files(&mut self) -> Result<(), String> {
            self.0.remove_arm_files()
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

    /// Which exits of pass 1 go on, and which are a copy that broke.
    #[test]
    fn a_copys_exit_says_whether_it_ended() {
        let copied = |exit, deletions_skipped| Copied {
            exit,
            tail: String::new(),
            deletions_skipped,
        };
        let end = |exit| copied(exit, false).end();
        // Exit 23 with rsync's deletions skipped isn't a copy with problems: it broke.
        assert_eq!(copied(Some(23), true).end(), CopyEnd::Broke);
        // The line alone decides nothing.
        assert_eq!(
            copied(Some(0), true).end(),
            CopyEnd::Ended { problems: false }
        );
        assert_eq!(end(Some(0)), CopyEnd::Ended { problems: false });
        assert_eq!(end(Some(24)), CopyEnd::Ended { problems: false });
        assert_eq!(end(Some(23)), CopyEnd::Ended { problems: true });
        for exit in [
            None,
            Some(1),
            Some(11),
            Some(12),
            Some(20),
            Some(30),
            Some(-1),
        ] {
            assert_eq!(end(exit), CopyEnd::Broke, "{exit:?}");
        }
    }

    /// What rsync prints on its standard output when a read error made it skip deletions.
    #[test]
    fn a_copys_output_says_whether_deletions_were_skipped() {
        let stdout = "sending incremental file list\n\
                      IO error encountered -- skipping file deletion\n\
                      \nNumber of files: 3\n";
        assert!(Copied::new(Some(23), stdout, "x".to_owned()).deletions_skipped);
        assert_eq!(Copied::new(Some(23), stdout, "x".to_owned()).tail, "x");
        assert!(!Copied::new(Some(23), "Number of files: 3\n", String::new()).deletions_skipped);
        assert!(!Copied::new(Some(23), "", String::new()).deletions_skipped);
    }

    // ---- the snapshot, before the copy ----

    /// The pure check: the snapshot's folder is there, and its `info.json` is the plan's
    /// snapshot's (the same creation time), of this installation, an rsync one.
    #[test]
    fn the_snapshot_is_checked_against_the_plan() {
        let plan = plan();
        assert_eq!(check_snapshot(&plan, &found(&info())), Ok(()));
        let no_folder = SnapshotFound {
            has_localhost: false,
            ..found(&info())
        };
        let no_info = SnapshotFound {
            has_localhost: true,
            info: None,
        };
        let not_json = SnapshotFound {
            has_localhost: true,
            info: Some("{".to_owned()),
        };
        let other = Info {
            created: CREATED + 1,
            ..info()
        };
        let elsewhere = Info {
            sys_uuid: "99999999-2222-3333-4444-555555555555".to_owned(),
            ..info()
        };
        let btrfs = Info {
            kind: "btrfs".to_owned(),
            ..info()
        };
        for (snapshot, reason) in [
            (no_folder, "the snapshot's folder isn't on the backup disk"),
            (no_info, "the snapshot's info.json can't be read"),
            (not_json, "the snapshot's info.json can't be read"),
            (
                found(&other),
                "the snapshot on the backup disk isn't the one this restore was prepared for",
            ),
            (
                found(&elsewhere),
                "the snapshot on the backup disk is of another installation",
            ),
            (
                found(&btrfs),
                "the snapshot on the backup disk isn't an rsync snapshot",
            ),
        ] {
            assert_eq!(
                check_snapshot(&plan, &snapshot),
                Err(reason.to_owned()),
                "{snapshot:?}"
            );
        }
    }

    /// A snapshot that isn't there, or isn't the plan's, never starts: no copy, no attempt,
    /// nothing touched.
    #[test]
    fn a_snapshot_that_fails_the_check_never_starts() {
        let lab = armed("apply-snapshot-gone");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.snapshot = SnapshotFound {
            has_localhost: false,
            info: None,
        };
        assert_eq!(boot(&mut fake), End::Finished(Outcome::NotStarted));
        assert_eq!(fake.count_of("copy"), 0);
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(
            report(&lab).message,
            "the snapshot's folder isn't on the backup disk"
        );
        assert_eq!(State::load(&lab.state).unwrap(), state(0, Step::End, false));
        assert!(!is_linked(&lab));
        assert_eq!(lab.esp_tree(), before);
    }

    /// The same after an earlier boot's copy broke: no further copy, and a restore that
    /// didn't finish.
    #[test]
    fn a_snapshot_that_is_gone_after_a_broken_copy_ends_failed() {
        let lab = armed("apply-snapshot-gone-later");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(12)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        fake.snapshot = found(&Info {
            created: CREATED + 60,
            ..info()
        });
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(fake.count_of("copy"), 1);
        assert_eq!(
            report(&lab).message,
            "the snapshot on the backup disk isn't the one this restore was prepared for"
        );
    }

    /// The check runs before every copy, not only the first.
    #[test]
    fn the_snapshot_is_checked_before_every_copy() {
        let lab = armed("apply-snapshot-each");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(11), Some(11)].into();
        boot_to_the_end(&mut fake);
        assert_eq!(fake.count_of("copy"), 3);
        assert_eq!(fake.count_of("find_snapshot"), 3);
        let copies: Vec<_> = fake
            .calls
            .iter()
            .enumerate()
            .filter(|(_, call)| **call == "copy")
            .map(|(index, _)| {
                fake.calls[..index]
                    .iter()
                    .rev()
                    .find(|call| **call != "say")
            })
            .collect();
        assert_eq!(copies, [Some(&"find_snapshot"); 3]);
    }

    // ---- exit 23 with the deletions skipped ----

    /// rsync exited 23 and said it skipped its deletions: a folder of the snapshot couldn't
    /// be read. That's a copy that broke, not one with problems: it counts, the link stays,
    /// the boot files aren't touched, and it's tried again.
    #[test]
    fn exit_23_with_the_deletions_skipped_is_a_copy_that_broke() {
        let lab = armed("apply-23-skipped");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.deletions_skipped = true;
        fake.exits = [Some(23), Some(23), Some(23)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert!(is_linked(&lab));
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::Copy, false)
        );
        assert_eq!(boot(&mut fake), End::Retry { attempt: 3 });
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Failed));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(lab.esp_tree(), before);
        assert_eq!(
            report(&lab).message,
            "the copy broke on each of 3 tries: rsync error: code Some(23); rsync skipped its \
             deletions after a read error, so the system may be a mix of the snapshot and what \
             was there before"
        );
    }

    /// rsync 3.2.7 exits a plain 23, with no "skipping file deletion" line, when the whole
    /// backup disk vanishes under it (check 6, 2026-10-03: the line comes only from a later
    /// folder's deletion pass, and once the walk collapses none comes). So after a plain 23
    /// the snapshot is looked at again: gone, the copy broke like any other, the boot files
    /// aren't touched, and it's tried again.
    #[test]
    fn exit_23_with_the_snapshot_gone_after_the_copy_is_a_copy_that_broke() {
        let lab = armed("apply-23-disk-gone");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.disk_leaves_in_copy = true;
        fake.exits = [Some(23), Some(0)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert!(is_linked(&lab));
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::Copy, false)
        );
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(fake.count_of("find_snapshot"), 2);
        assert_eq!(lab.esp_tree(), before);
        assert!(
            fake.said.contains(
                &"the copy broke (rsync error: code Some(23); after the copy, the snapshot's \
                  folder isn't on the backup disk: the disk went away during the copy, so the \
                  system may be a mix of the snapshot and what was there before): restarting \
                  to try again, attempt 2 of 3"
                    .to_owned()
            ),
            "{:?}",
            fake.said
        );
        // The disk is back: the next attempt finishes it.
        fake.snapshot = found(&info());
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
    }

    /// The files of the filter's lifetime, as an arm leaves them: the working
    /// pair, the pair the arm kept, and a log.
    const LIFETIME: [&str; 5] = [
        "restore.filter",
        "restore.note",
        "last-restore.filter",
        "last-restore.note",
        "rsync-log",
    ];

    fn armed_with_the_working_files(name: &str) -> Lab {
        let lab = armed(name);
        for file in LIFETIME {
            fs::write(lab.state.join(file), file).unwrap();
        }
        lab
    }

    /// The lab's restore has begun [`MAX_BOOTS`] boots and ended none.
    fn past_the_cap(lab: &Lab) {
        State {
            boots: MAX_BOOTS,
            ..state(1, Step::Copy, false)
        }
        .save(&lab.state)
        .unwrap();
    }

    /// Which of [`LIFETIME`] are in the state folder.
    fn lifetime_left(lab: &Lab) -> Vec<&'static str> {
        LIFETIME
            .into_iter()
            .filter(|file| lab.state.join(file).exists())
            .collect()
    }

    /// Pull 1: a restore that's over removes the preparation's filter and note with the
    /// plan, whatever the outcome, and keeps the pair of its arm and the log.
    #[test]
    fn a_finished_restore_removes_the_working_files_and_keeps_the_arms_pair() {
        const KEPT: [&str; 3] = ["last-restore.filter", "last-restore.note", "rsync-log"];
        let lab = armed_with_the_working_files("apply-lifetime-done");
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(lifetime_left(&lab), KEPT);
        assert!(!lab.state.join(plan::FILE).exists());
        // Never started (the backup disk isn't there): the same.
        let lab = armed_with_the_working_files("apply-lifetime-not-started");
        let mut fake = Fake::new(&lab);
        fake.backup = Err("the backup disk wasn't found within 60 seconds".to_owned());
        assert_eq!(boot(&mut fake), End::Finished(Outcome::NotStarted));
        assert_eq!(lifetime_left(&lab), KEPT);
        // Given up after too many boots: the same.
        let lab = armed_with_the_working_files("apply-lifetime-gave-up");
        past_the_cap(&lab);
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::GaveUp);
        assert_eq!(lifetime_left(&lab), KEPT);
    }

    /// A retry needs the filter again, and a link that can't be removed keeps the arm
    /// whole: neither end removes anything of the filter's lifetime.
    #[test]
    fn a_retry_and_a_stuck_link_remove_none_of_the_working_files() {
        let lab = armed_with_the_working_files("apply-lifetime-retry");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(11)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert_eq!(lifetime_left(&lab), LIFETIME);
        let lab = armed_with_the_working_files("apply-lifetime-stuck");
        let mut fake = Fake::new(&lab);
        fake.stuck_link = true;
        assert_eq!(
            boot(&mut fake),
            End::LinkStuck {
                outcome: Outcome::Done
            }
        );
        assert_eq!(lifetime_left(&lab), LIFETIME);
        // Given up with the link stuck: nothing either.
        let lab = armed_with_the_working_files("apply-lifetime-gave-up-stuck");
        past_the_cap(&lab);
        let mut fake = Fake::new(&lab);
        fake.stuck_link = true;
        assert_eq!(
            boot(&mut fake),
            End::LinkStuck {
                outcome: Outcome::Failed
            }
        );
        assert_eq!(lifetime_left(&lab), LIFETIME);
    }

    /// A plain 23 with the snapshot still there is what it was: a copy with problems.
    #[test]
    fn exit_23_with_the_snapshot_still_there_is_a_copy_with_problems() {
        let lab = armed("apply-23-disk-there");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(23)].into();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Problems));
        assert_eq!(fake.count_of("find_snapshot"), 2);
    }

    /// rsync's own lines among what was said, without their `rsync stderr: ` in front.
    fn rsync_lines<'a>(fake: &'a Fake<'_>) -> Vec<&'a str> {
        fake.said
            .iter()
            .filter_map(|line| line.strip_prefix("rsync stderr: "))
            .collect()
    }

    /// Twenty lines as rsync writes them for files it couldn't write, its summary last.
    fn twenty_errors() -> Vec<String> {
        (1..=19)
            .map(|n| {
                format!(
                    "rsync: [receiver] mkstemp \"/usr/lib/.file{n}.Xy12Ab\" failed: Read-only \
                     file system (30)"
                )
            })
            .chain(std::iter::once(
                "rsync error: some files/attrs were not transferred (see previous errors) \
                 (code 23) at main.c(1338) [sender=3.2.7]"
                    .to_owned(),
            ))
            .collect()
    }

    /// Item 9 (check 6, 2026-10-03): a plain 23 with the snapshot still there says rsync's
    /// last error lines, one line each and in order, after the copy's end and before the
    /// boot files. The result's message stays the one that names the log.
    #[test]
    fn a_plain_exit_23_says_rsyncs_last_errors_one_line_each() {
        let lab = armed("apply-23-tail");
        let mut fake = Fake::new(&lab);
        let errors = twenty_errors();
        fake.tail = Some(errors.join("\n"));
        fake.exits = [Some(23)].into();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Problems));
        assert_eq!(rsync_lines(&fake), errors);
        let at = |start: &str| {
            fake.said
                .iter()
                .position(|line| line.starts_with(start))
                .unwrap_or_else(|| panic!("{start}: {:?}", fake.said))
        };
        let first = at("rsync stderr: ");
        assert_eq!(fake.said[first - 1], "the copy ended: rsync exited 23");
        assert_eq!(first + errors.len(), at("refreshing the boot files"));
        assert_eq!(
            report(&lab).message,
            "some files couldn't be written or deleted; see /var/lib/apsis/restore/rsync-log"
        );
        // Empty lines aren't said.
        let lab = armed("apply-23-tail-gaps");
        let mut fake = Fake::new(&lab);
        fake.tail = Some("one\n\n   \ntwo\n".to_owned());
        fake.exits = [Some(23)].into();
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Problems));
        assert_eq!(rsync_lines(&fake), ["one", "two"]);
    }

    /// Only there: nothing of rsync's is said line by line after exit 0 or 24 (whatever it
    /// wrote to its standard error), after any other exit or none (the copy broke, and the
    /// lines are in that message), after a 23 with the deletions skipped, or after a 23
    /// with the snapshot gone.
    #[test]
    fn rsyncs_errors_are_said_line_by_line_for_a_plain_23_only() {
        for (name, exit, skipped, leaves, end) in [
            ("0", Some(0), false, false, End::Finished(Outcome::Done)),
            ("24", Some(24), false, false, End::Finished(Outcome::Done)),
            ("1", Some(1), false, false, End::Retry { attempt: 2 }),
            ("11", Some(11), false, false, End::Retry { attempt: 2 }),
            ("30", Some(30), false, false, End::Retry { attempt: 2 }),
            ("none", None, false, false, End::Retry { attempt: 2 }),
            (
                "23-skipped",
                Some(23),
                true,
                false,
                End::Retry { attempt: 2 },
            ),
            ("23-gone", Some(23), false, true, End::Retry { attempt: 2 }),
        ] {
            let lab = armed(&format!("apply-tail-not-{name}"));
            let mut fake = Fake::new(&lab);
            fake.tail = Some(twenty_errors().join("\n"));
            fake.deletions_skipped = skipped;
            fake.disk_leaves_in_copy = leaves;
            fake.exits = [exit].into();
            assert_eq!(boot(&mut fake), end, "{name}");
            assert_eq!(rsync_lines(&fake), [""; 0], "{name}: {:?}", fake.said);
        }
    }

    /// The lines are said after the copy is on disk and before the step is saved. A power
    /// cut right after the last of them finds the state still at the copy: the next boot
    /// copies again, as after any cut there, and says that copy's lines.
    #[test]
    fn a_power_cut_after_rsyncs_lines_copies_again_and_says_them_again() {
        let setup = |fake: &mut Fake<'_>| {
            fake.tail = Some("one\ntwo".to_owned());
            fake.exits = [Some(23), Some(23)].into();
        };
        // The look at the snapshot after the copy is the second one; the two lines follow.
        let probe = armed("apply-23-tail-cut-probe");
        let mut fake = Fake::new(&probe);
        setup(&mut fake);
        boot(&mut fake);
        let looked = fake
            .calls
            .iter()
            .enumerate()
            .filter(|(_, call)| **call == "find_snapshot")
            .nth(1)
            .unwrap()
            .0;
        assert_eq!(fake.calls[looked + 1..=looked + 2], ["say", "say"]);

        let lab = armed("apply-23-tail-cut");
        let mut fake = Fake::new(&lab);
        setup(&mut fake);
        assert_eq!(boot_cut(&mut fake, Cut::After(looked + 2)), None);
        assert_eq!(rsync_lines(&fake), ["one", "two"]);
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::Copy, false)
        );
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Problems));
        assert_eq!(fake.count_of("copy"), 2);
        assert_eq!(rsync_lines(&fake), ["one", "two", "one", "two"]);
    }

    /// The next attempt can end well: the disk was back.
    #[test]
    fn a_copy_that_broke_on_skipped_deletions_is_finished_by_the_next_attempt() {
        let lab = armed("apply-23-skipped-retry");
        let mut fake = Fake::new(&lab);
        fake.deletions_skipped = true;
        fake.exits = [Some(23), Some(0)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
    }

    // ---- the boot counter ----

    /// The count is the apply's first write, before the plan is read or the runner is asked
    /// for anything but the link.
    #[test]
    fn every_boot_is_counted_before_any_other_work() {
        let lab = armed("apply-boots");
        let mut fake = Fake::new(&lab);
        fake.exits = [Some(11)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        let counted: Vec<_> = step_states(&fake, "open_backup")
            .iter()
            .map(|state| (state.boots, state.attempts))
            .collect();
        assert_eq!(counted, [(1, 0), (2, 1)]);

        // Also when the plan can't be read.
        let lab = armed("apply-boots-no-plan");
        fs::remove_file(lab.state.join(plan::FILE)).unwrap();
        let mut fake = Fake::new(&lab);
        boot(&mut fake);
        assert_eq!(State::load(&lab.state).unwrap().boots, 1);
    }

    /// The cap: a restore that began [`MAX_BOOTS`] boots and ended none is given up. The
    /// link goes first, then the result. With the link gone the restart can only reach a
    /// normal boot, so the machine is restarted, once.
    #[test]
    fn past_the_cap_the_link_is_removed_and_the_machine_restarts_into_a_normal_boot() {
        let lab = armed("apply-cap");
        let before = lab.esp_tree();
        State {
            boots: MAX_BOOTS,
            ..state(1, Step::Copy, false)
        }
        .save(&lab.state)
        .unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::GaveUp);
        assert_eq!(
            fake.calls,
            [
                "is_armed",
                "remove_link",
                "now",
                "say",
                "remove_arm_files",
                "restart"
            ]
        );
        assert_eq!(fake.count_of("restart"), 1);
        assert_eq!(fake.restarts_over_the_link, 0);
        assert!(!is_linked(&lab));
        let report = report(&lab);
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.snapshot.as_deref(), Some(SNAPSHOT));
        assert_eq!(
            report.message,
            "the restore was started in 5 boots and ended in none, so it was given up; the \
             system may be partly restored"
        );
        assert_eq!(lab.esp_tree(), before);
        // The count isn't written past the cap.
        assert_eq!(State::load(&lab.state).unwrap().boots, MAX_BOOTS);
    }

    #[test]
    fn past_the_cap_a_link_that_cant_be_removed_gets_no_restart_either() {
        let lab = armed("apply-cap-stuck");
        State {
            boots: MAX_BOOTS,
            ..state(1, Step::Copy, false)
        }
        .save(&lab.state)
        .unwrap();
        let mut fake = Fake::new(&lab);
        fake.stuck_link = true;
        assert_eq!(
            boot(&mut fake),
            End::LinkStuck {
                outcome: Outcome::Failed
            }
        );
        assert_eq!(fake.count_of("restart"), 0);
        assert_eq!(report(&lab).outcome, Outcome::Failed);
    }

    /// A helper that's killed, or panics, whenever it gets to one thing: every boot dies
    /// there. Wherever that is after the count, the restore ends within the cap, with the
    /// link gone, and the apply never restarts over a link past it.
    #[test]
    fn a_helper_that_dies_at_any_point_after_the_count_is_bounded() {
        for (die_at, refresh) in [
            ("open_backup", Refresh::Works),
            ("find_snapshot", Refresh::Works),
            ("copy", Refresh::Works),
            ("back_up_esp", Refresh::Works),
            ("refresh_boot", Refresh::Works),
            ("put_back_esp", Refresh::Fails),
            ("remove_protected_kernel", Refresh::Works),
            ("now", Refresh::Works),
            ("say", Refresh::Works),
            ("remove_arm_files", Refresh::Works),
            ("restart", Refresh::Works),
        ] {
            let lab = armed(&format!("apply-killed-{die_at}"));
            let mut fake = Fake::new(&lab);
            fake.die_at = Some(die_at);
            fake.refresh = refresh;
            let mut boots = 0;
            let mut last = None;
            // The unit only runs while the link is there.
            while is_linked(&lab) {
                boots += 1;
                assert!(boots <= MAX_BOOTS + 1, "{die_at}: boot {boots}");
                last = boot_or_die(&mut fake);
            }
            assert!(fake.count_of("copy") <= 3, "{die_at}");
            // The only restarts over the link are the retries: two at most. A boot past the
            // cap restarts once at most, and only with the link gone.
            assert!(fake.restarts_over_the_link <= 2, "{die_at}");
            if boots > MAX_BOOTS {
                assert!(
                    matches!(last, None | Some(End::GaveUp)),
                    "{die_at}: {last:?}"
                );
                let last_boot: Vec<_> = fake
                    .calls
                    .iter()
                    .rev()
                    .take_while(|call| **call != "is_armed")
                    .collect();
                let restarts = last_boot.iter().filter(|call| ***call == "restart").count();
                assert!(restarts <= 1, "{die_at}");
                // Dying at the restart itself, or before it, leaves none.
                let died_first = last.is_none();
                assert_eq!(restarts, usize::from(!died_first), "{die_at}");
            }
        }
    }

    /// The two things before the count, and the link's removal itself, are what the count
    /// can't cover: a helper that dies there every time isn't stopped by the apply.
    #[test]
    fn what_runs_before_the_count_is_only_the_link_check_and_reading_the_state() {
        let lab = armed("apply-before-count");
        let mut fake = Fake::new(&lab);
        fake.die_at = Some("is_armed");
        for _ in 0..MAX_BOOTS + 2 {
            assert_eq!(boot_or_die(&mut fake), None);
        }
        assert_eq!(State::load(&lab.state).unwrap(), State::default());
        assert_eq!(fake.calls, Vec::<&str>::new());
    }

    /// A backup that doesn't verify and a temporary file a cut put-back left: the refresh
    /// isn't run, and the temporary file still goes.
    #[test]
    fn a_temporary_file_is_cleared_also_when_nothing_is_refreshed() {
        let lab = cut_with_a_backup("apply-tmp-unverified");
        fs::write(lab.state.join(BACKUP_DIR).join("initrd.img"), "damaged").unwrap();
        state(1, Step::Refresh, false).save(&lab.state).unwrap();
        let cmdline = lab.esp_file(BootFile::Cmdline);
        fs::write(cmdline.with_file_name("cmdline.apsis-tmp"), "half").unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert!(!has_temporary_files(&lab));
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
        assert_eq!(
            again,
            ["is_armed", "remove_link", "remove_arm_files", "restart"]
        );
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

    /// One boot in which the power goes inside a step ([`Fake::cut_inside`]).
    fn boot_and_lose_power(fake: &mut Fake<'_>) {
        let end = catch_unwind(AssertUnwindSafe(|| boot(fake)));
        assert!(end.is_err_and(|payload| payload.is::<PowerCut>()));
        assert_eq!(fake.cut_inside, None, "the step was never reached");
    }

    fn boot_files_on_the_esp(lab: &Lab) -> Vec<String> {
        BootFile::ALL
            .map(|file| fs::read_to_string(lab.esp_file(file)).unwrap())
            .to_vec()
    }

    /// The seven files as a boot refresh for the snapshot's kernels leaves them.
    fn refreshed() -> Vec<String> {
        vec![
            kernel(OLD),
            initrd(OLD),
            cmdline(OLD),
            entry(OLD),
            kernel(OLDER),
            initrd(OLDER),
            entry(OLDER),
        ]
    }

    fn has_temporary_files(lab: &Lab) -> bool {
        lab.esp_tree()
            .iter()
            .any(|(name, _)| name.ends_with(".apsis-tmp"))
    }

    /// The power went while the backup was being made: two files, no manifest. The state
    /// says the boot refresh hasn't started, so the ESP is as it was: the partial folder is
    /// removed and the backup is taken once more.
    #[test]
    fn a_backup_cut_halfway_is_removed_and_taken_once_more() {
        let lab = armed("apply-backup-cut");
        let mut fake = Fake::new(&lab);
        fake.cut_inside = Some("back_up_esp");
        boot_and_lose_power(&mut fake);
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::BootFiles, false)
        );
        assert!(lab.state.join(BACKUP_DIR).join("initrd.img").is_file());
        assert!(!lab.state.join(BACKUP_DIR).join(MANIFEST_FILE).exists());

        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(fake.count_of("copy"), 1);
        assert_eq!(fake.count_of("back_up_esp"), 2);
        // Fully refreshed, never mixed.
        assert_eq!(boot_files_on_the_esp(&lab), refreshed());
        assert_eq!(passed(&lab), restored_boot_files());
        assert!(!lab.state.join(BACKUP_DIR).exists());
    }

    /// The backup that's taken once more is of the untouched ESP: a refresh that then fails
    /// is put back to exactly what was there.
    #[test]
    fn a_backup_taken_once_more_puts_back_the_esp_of_before() {
        let lab = armed("apply-backup-cut-kept");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.cut_inside = Some("back_up_esp");
        boot_and_lose_power(&mut fake);
        fake.refresh = Refresh::Fails;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(lab.esp_tree(), before);
    }

    /// The same folder without a manifest, but the state says the boot refresh has started:
    /// the ESP may be changed, so no backup is taken of it, and without one the boot files
    /// aren't touched.
    #[test]
    fn a_backup_is_never_taken_again_once_the_refresh_has_started() {
        let lab = cut_with_a_backup("apply-backup-partial");
        fs::remove_file(lab.state.join(BACKUP_DIR).join(MANIFEST_FILE)).unwrap();
        state(1, Step::Refresh, false).save(&lab.state).unwrap();
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

    /// The refresh has started and there's no backup folder at all: the same.
    #[test]
    fn a_backup_that_is_gone_once_the_refresh_has_started_isnt_taken_again() {
        let lab = cut_with_a_backup("apply-backup-gone");
        esp::remove(&lab.state).unwrap();
        state(1, Step::Refresh, false).save(&lab.state).unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
    }

    /// A backup that has its manifest and doesn't verify is damaged, not partial: it's never
    /// removed and retaken, whatever the step.
    #[test]
    fn a_damaged_backup_isnt_partial_and_isnt_taken_again() {
        let lab = cut_with_a_backup("apply-backup-damaged-early");
        fs::write(lab.state.join(BACKUP_DIR).join("initrd.img"), "damaged").unwrap();
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 0);
        assert_eq!(fake.count_of("refresh_boot"), 0);
        assert_eq!(lab.esp_tree(), before);
    }

    // ---- cuts inside a step ----

    /// The power went during the put-back: the kernel and initrd are the old ones, the rest
    /// is what the failed refresh left, and a temporary file is on the ESP. The next boot's
    /// refresh fails again, and its put-back makes the ESP fully what it was.
    #[test]
    fn a_put_back_cut_halfway_is_finished_by_the_next_boot() {
        let lab = armed("apply-put-back-cut");
        let before = lab.esp_tree();
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Fails;
        fake.cut_inside = Some("put_back_esp");
        boot_and_lose_power(&mut fake);
        // Mixed: the kernel is back, the previous pair isn't.
        assert_eq!(boot_files_on_the_esp(&lab)[0], kernel(NEW));
        assert_eq!(boot_files_on_the_esp(&lab)[4], kernel(OLDER));
        assert!(has_temporary_files(&lab));
        assert_eq!(
            State::load(&lab.state).unwrap(),
            state(1, Step::Refresh, false)
        );

        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        assert_eq!(fake.count_of("back_up_esp"), 1);
        assert_eq!(fake.count_of("copy"), 1);
        assert_eq!(lab.esp_tree(), before);
        assert!(esp::boots_kernel(&lab.esp, &lab.root, UUID, NEW));
    }

    /// The same cut, and a boot refresh that works the next time: the ESP is fully the
    /// refreshed one, with nothing of the half put-back left.
    #[test]
    fn a_put_back_cut_halfway_is_replaced_by_a_refresh_that_works() {
        let lab = armed("apply-put-back-cut-done");
        let mut fake = Fake::new(&lab);
        fake.refresh = Refresh::Fails;
        fake.cut_inside = Some("put_back_esp");
        boot_and_lose_power(&mut fake);
        fake.refresh = Refresh::Works;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(boot_files_on_the_esp(&lab), refreshed());
        assert_eq!(passed(&lab), restored_boot_files());
        assert!(!has_temporary_files(&lab));
        assert_eq!(fake.count_of("back_up_esp"), 1);
    }

    // ---- a link that can't be removed ----

    /// systemd.offline-updates(7): with the link still there, a restart goes to the update
    /// again. So the apply never restarts over a link it couldn't remove.
    #[test]
    fn a_link_that_cant_be_removed_ends_without_a_restart() {
        let lab = armed("apply-stuck");
        let mut fake = Fake::new(&lab);
        fake.stuck_link = true;
        let stuck = End::LinkStuck {
            outcome: Outcome::Done,
        };
        assert_eq!(boot(&mut fake), stuck);
        assert_eq!(fake.count_of("restart"), 0);
        // The arm stays whole: nothing of it is removed around a link that's still there.
        assert_eq!(fake.count_of("remove_arm_files"), 0);
        assert!(is_linked(&lab));
        assert_eq!(report(&lab).outcome, Outcome::Done);
        assert_eq!(State::load(&lab.state).unwrap(), state(1, Step::End, false));

        // Whatever starts the next boot, it does no work and again doesn't restart.
        let calls = fake.calls.len();
        assert_eq!(boot(&mut fake), stuck);
        let again: Vec<_> = fake.calls[calls..]
            .iter()
            .copied()
            .filter(|call| *call != "say")
            .collect();
        assert_eq!(again, ["is_armed", "remove_link"]);
        assert_eq!(fake.count_of("restart"), 0);
        assert_eq!(fake.count_of("copy"), 1);
    }

    /// The only restarts with the link in place are the retries, and an attempt is on disk
    /// before each: two at most. The third broken copy ends the restore, and a link that
    /// can't be removed then gets no third restart.
    #[test]
    fn three_broken_copies_and_a_stuck_link_restart_twice_and_no_more() {
        let lab = armed("apply-stuck-failed");
        let mut fake = Fake::new(&lab);
        fake.stuck_link = true;
        fake.exits = [Some(11), Some(11), Some(11)].into();
        assert_eq!(boot(&mut fake), End::Retry { attempt: 2 });
        assert_eq!(boot(&mut fake), End::Retry { attempt: 3 });
        let stuck = End::LinkStuck {
            outcome: Outcome::Failed,
        };
        assert_eq!(boot(&mut fake), stuck);
        assert_eq!(fake.count_of("restart"), 2);
        for _ in 0..3 {
            assert_eq!(boot(&mut fake), stuck);
        }
        assert_eq!(fake.count_of("restart"), 2);
        assert_eq!(fake.restarts_over_the_link, 2);
        assert_eq!(fake.count_of("copy"), 3);
    }

    /// A backup that doesn't verify, and an ESP the refresh had already finished: it boots
    /// the restored kernel, and that's a restore that's done.
    #[test]
    fn a_damaged_backup_with_a_finished_refresh_ends_done() {
        let lab = cut_with_a_backup("apply-backup-damaged-done");
        fs::write(lab.state.join(BACKUP_DIR).join("initrd.img"), "damaged").unwrap();
        state(1, Step::Refresh, false).save(&lab.state).unwrap();
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
        state(1, Step::Refresh, false).save(&lab.state).unwrap();
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

    /// systemd.offline-updates(7), point 5: a link that points somewhere else is another
    /// tool's update. It's left where it is, and the apply doesn't restart under that tool.
    #[test]
    fn a_link_that_isnt_apsiss_is_left_alone() {
        let lab = armed("apply-not-armed");
        fs::remove_file(lab.root.join("system-update")).unwrap();
        std::os::unix::fs::symlink("/var/lib/system-update", lab.root.join("system-update"))
            .unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::NotArmed);
        // Only Apsis's own leftovers go: the unit that ran this, and the helper copy.
        assert_eq!(fake.calls, ["is_armed", "say", "remove_arm_files"]);
        assert_eq!(
            fs::read_link(lab.root.join("system-update")).unwrap(),
            Path::new("/var/lib/system-update")
        );
        assert!(!lab.state.join(RESULT_FILE).exists());
        assert_eq!(State::load(&lab.state).unwrap(), State::default());
    }

    #[test]
    fn without_a_link_nothing_is_read_or_reported() {
        let lab = armed("apply-no-link");
        fs::remove_file(lab.root.join("system-update")).unwrap();
        let mut fake = Fake::new(&lab);
        assert_eq!(boot(&mut fake), End::NotArmed);
        assert_eq!(fake.calls, ["is_armed", "say", "remove_arm_files"]);
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
            [
                "is_armed",
                "now",
                "remove_link",
                "remove_arm_files",
                "restart"
            ]
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
    /// written in its place, with the real outcome.
    #[test]
    fn a_refused_result_is_replaced_by_the_minimal_report_with_its_outcome() {
        let lab = armed("apply-minimal");
        let mut fake = Fake::new(&lab);
        fake.now = 0;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::Done));
        assert_eq!(
            report(&lab),
            Report {
                outcome: Outcome::Done,
                snapshot: Some(SNAPSHOT.to_owned()),
                safety_snapshot: Some(SAFETY.to_owned()),
                home: Home::Keep,
                message: "result could not be saved, see journal".to_owned(),
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

    /// Every outcome keeps itself in the minimal report.
    #[test]
    fn the_minimal_report_keeps_a_boot_kept_outcome() {
        let lab = armed("apply-minimal-kept");
        let mut fake = Fake::new(&lab);
        fake.now = 0;
        fake.refresh = Refresh::Fails;
        assert_eq!(boot(&mut fake), End::Finished(Outcome::BootKept));
        let report = report(&lab);
        assert_eq!(report.outcome, Outcome::BootKept);
        assert_eq!(report.when, None);
    }

    /// What's left when the snapshot's name is what was refused: only `failed` may lack it,
    /// so the real outcome goes into the message.
    #[test]
    fn a_minimal_report_without_a_snapshot_is_failed_and_names_the_outcome() {
        let lab = armed("apply-minimal-bare");
        let mut fake = Fake::new(&lab);
        let refused = Report {
            outcome: Outcome::Done,
            snapshot: Some("newest".to_owned()),
            safety_snapshot: Some(SAFETY.to_owned()),
            home: Home::Restore,
            message: String::new(),
            when: Some(NOW),
        };
        assert_eq!(
            save_minimal(&lab.state, &mut fake, &refused),
            Outcome::Failed
        );
        assert_eq!(
            report(&lab),
            Report {
                outcome: Outcome::Failed,
                snapshot: None,
                safety_snapshot: None,
                home: Home::Restore,
                message: "result could not be saved, see journal (the restore ended: done)"
                    .to_owned(),
                when: Some(NOW),
            }
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
        assert_eq!(report.message, "result could not be saved, see journal");
    }

    /// The helper's exit code: 1 only when a restart was called and its call failed, so the
    /// unit's `FailureAction=reboot` restarts; never over a stuck link or another tool's.
    #[test]
    fn the_exit_code_restarts_through_systemd_only_when_the_reboot_call_failed() {
        let restarting = [
            End::Finished(Outcome::Done),
            End::Finished(Outcome::BootBroken),
            End::GaveUp,
            End::Retry { attempt: 2 },
        ];
        for end in restarting {
            assert_eq!(exit_code(end, false), 0, "{end:?}");
            assert_eq!(exit_code(end, true), 1, "{end:?}");
        }
        let not_restarting = [
            End::LinkStuck {
                outcome: Outcome::Done,
            },
            End::NotArmed,
        ];
        for end in not_restarting {
            assert_eq!(exit_code(end, false), 0, "{end:?}");
            assert_eq!(exit_code(end, true), 0, "{end:?}: nothing may restart");
        }
    }
}
