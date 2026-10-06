// SPDX-License-Identifier: GPL-3.0-only

//! `state.json` and `result.json`: how far an armed restore got, and how it ended.
//!
//! `state.json` is written on arm and before each step of the apply that depends on it
//! ([`super::apply`]); `result.json` when the apply ends, and it stays for the window to show
//! after login. Format and rules: [`super::file`].

use std::borrow::Cow;
use std::path::Path;

use super::file::{self, FileError};
use super::filter::Home;

pub const STATE_FILE: &str = "state.json";
pub const RESULT_FILE: &str = "result.json";

/// A copy that breaks is tried again at the next boot, this many times in all.
pub const MAX_ATTEMPTS: u32 = 3;

/// The offline boots a restore may begin. A boot is counted before it does
/// anything else, so a helper that dies in each of them is stopped here: three copies, and
/// two boots to spare for power cuts.
pub const MAX_BOOTS: u32 = 5;

/// What's kept of a result's message, in bytes: enough for a tooltip and rsync's last lines.
/// Written as JSON it stays far below the file limit, whatever the characters.
pub const MAX_MESSAGE_BYTES: usize = 2048;

/// Where an armed restore is. A boot after a power cut goes on from here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Step {
    /// Armed, and no copy has started.
    #[default]
    Armed,
    /// Pass 1 started (step 3) and hasn't ended well: it runs again.
    Copy,
    /// Pass 1 ended and is on disk: the ESP backup is next (step 4), and the copy isn't run
    /// again. The boot files are still as they were before the restore.
    BootFiles,
    /// The ESP backup is whole and the boot refresh started (steps 5 to 7): the boot files
    /// may be changed, so no backup is ever taken of them again.
    Refresh,
    /// `result.json` is written: only the cleanup is left (step 8).
    End,
}

impl Step {
    const ALL: [Self; 5] = [
        Self::Armed,
        Self::Copy,
        Self::BootFiles,
        Self::Refresh,
        Self::End,
    ];

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Armed => "armed",
            Self::Copy => "copy",
            Self::BootFiles => "boot-files",
            Self::Refresh => "boot-refresh",
            Self::End => "end",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.word() == word)
    }
}

/// How far an armed restore got. Saved (and so fsynced) before the step that depends on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// Copies that started.
    pub attempts: u32,
    /// Anything under `/` was ever written by this restore.
    pub written: bool,
    pub step: Step,
    /// The copy ended with rsync's exit 23: some files couldn't be written or deleted.
    pub problems: bool,
    /// Offline boots in which the apply began. Its first write in each of them.
    pub boots: u32,
}

impl State {
    /// # Errors
    ///
    /// [`FileError::Invalid`] if `text` isn't a state this Apsis wrote, whole and in range.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let map = file::object(text, &["attempts", "written", "step", "problems", "boots"])?;
        let state = Self {
            attempts: map
                .get("attempts")
                .and_then(serde_json::Value::as_u64)
                .and_then(|attempts| u32::try_from(attempts).ok())
                .ok_or_else(attempts_out_of_range)?,
            written: file::flag(&map, "written")?,
            step: Step::from_word(file::text(&map, "step")?)
                .ok_or_else(|| FileError::Invalid("\"step\" isn't a step".to_owned()))?,
            problems: file::flag(&map, "problems")?,
            boots: map
                .get("boots")
                .and_then(serde_json::Value::as_u64)
                .and_then(|boots| u32::try_from(boots).ok())
                .ok_or_else(boots_out_of_range)?,
        };
        state.validate()?;
        Ok(state)
    }

    /// The file's text.
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] if the state is one [`State::parse`] would refuse.
    pub fn to_text(&self) -> Result<String, FileError> {
        self.validate()?;
        Ok(file::to_text([
            ("attempts", self.attempts.into()),
            ("written", self.written.into()),
            ("step", self.step.word().into()),
            ("problems", self.problems.into()),
            ("boots", self.boots.into()),
        ]))
    }

    /// Reads `dir/state.json`.
    ///
    /// # Errors
    ///
    /// [`FileError::Io`] if it's missing or unreadable, [`FileError::Invalid`] as for
    /// [`State::parse`].
    pub fn load(dir: &Path) -> Result<Self, FileError> {
        Self::parse(&file::load(dir, STATE_FILE)?).map_err(|error| error.within(STATE_FILE))
    }

    /// Writes `dir/state.json` in one step (temporary file, fsync, rename).
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] as for [`State::to_text`], with nothing written;
    /// [`FileError::Io`] if the write fails, with the file as it was.
    pub fn save(&self, dir: &Path) -> Result<(), FileError> {
        file::save(dir, STATE_FILE, &self.to_text()?)
    }

    fn validate(&self) -> Result<(), FileError> {
        if self.attempts > MAX_ATTEMPTS {
            return Err(attempts_out_of_range());
        }
        // The apply sets both before the copy starts.
        let invalid = |reason: String| Err(FileError::Invalid(reason));
        match (self.attempts, self.written) {
            (0, true) => {
                return invalid(
                    "something is marked written but no attempt was counted".to_owned(),
                );
            }
            (1.., false) => {
                return invalid("an attempt was counted but nothing is marked written".to_owned());
            }
            _ => {}
        }
        let word = self.step.word();
        match (self.step, self.attempts) {
            (Step::Armed, 1..) => {
                return invalid(format!("an attempt was counted but the step is {word:?}"));
            }
            (Step::Copy | Step::BootFiles | Step::Refresh, 0) => {
                return invalid(format!("the step is {word:?} but no attempt was counted"));
            }
            _ => {}
        }
        if self.problems && matches!(self.step, Step::Armed | Step::Copy) {
            return invalid("problems are marked before the copy ended".to_owned());
        }
        if self.problems && self.attempts == 0 {
            return invalid("problems are marked but no attempt was counted".to_owned());
        }
        if self.boots > MAX_BOOTS {
            return Err(boots_out_of_range());
        }
        // The boot is counted first, and starts one copy at most.
        if self.attempts > self.boots {
            return invalid("more attempts were counted than boots".to_owned());
        }
        Ok(())
    }
}

fn boots_out_of_range() -> FileError {
    FileError::Invalid(format!("\"boots\" isn't 0 to {MAX_BOOTS}"))
}

fn attempts_out_of_range() -> FileError {
    FileError::Invalid(format!("\"attempts\" isn't 0 to {MAX_ATTEMPTS}"))
}

/// How an armed restore ended. The words are the helper's `RestoreResult` states;
/// its `ready` isn't one: that's a plan that isn't armed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The system is the snapshot's.
    Done,
    /// Restored, but rsync couldn't write or delete some files (exit 23).
    Problems,
    /// Restored, but the new boot files didn't check out and the old ones were put back.
    BootKept,
    /// Restored, and the old boot files couldn't be put back either.
    BootBroken,
    /// Stopped before the copy ran.
    NotStarted,
    /// The copy broke on every attempt, or never came back after one.
    Failed,
}

impl Outcome {
    const ALL: [Self; 6] = [
        Self::Done,
        Self::Problems,
        Self::BootKept,
        Self::BootBroken,
        Self::NotStarted,
        Self::Failed,
    ];

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Problems => "problems",
            Self::BootKept => "boot-kept",
            Self::BootBroken => "boot-broken",
            Self::NotStarted => "not-started",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.word() == word)
    }
}

/// `result.json`: what the window shows after login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    /// The snapshot that was restored, or was going to be. `None` only with
    /// [`Outcome::Failed`]: the plan that named it couldn't be read.
    pub snapshot: Option<String>,
    /// The plan's safety snapshot, if one was taken.
    pub safety_snapshot: Option<String>,
    /// Whether home was kept or restored (the result's tooltip says it).
    pub home: Home,
    /// The helper's reason for anything but `done`; may be empty, may have several lines.
    /// A longer one than [`MAX_MESSAGE_BYTES`] is written and read as `... ` and its end, so
    /// no length is refused.
    pub message: String,
    /// When the apply ended, in Unix seconds, by the clock of that boot. It's never compared
    /// with the plan's time or any other: a hardware clock in local time puts them hours
    /// apart. `None`, with any outcome: the clock, its only source, gave no time.
    pub when: Option<i64>,
}

impl Report {
    /// # Errors
    ///
    /// [`FileError::Invalid`] if `text` isn't a result this Apsis wrote, whole and in range.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let map = file::object(
            text,
            &[
                "outcome",
                "snapshot",
                "safety_snapshot",
                "home",
                "message",
                "when",
            ],
        )?;
        let report = Self {
            outcome: Outcome::from_word(file::text(&map, "outcome")?)
                .ok_or_else(|| FileError::Invalid("\"outcome\" isn't an outcome".to_owned()))?,
            snapshot: file::optional_text(&map, "snapshot")?.map(str::to_owned),
            safety_snapshot: file::optional_text(&map, "safety_snapshot")?.map(str::to_owned),
            home: file::home(&map)?,
            message: cut(file::text(&map, "message")?).into_owned(),
            when: match map.get("when") {
                Some(serde_json::Value::Null) => None,
                _ => Some(file::time(&map, "when")?),
            },
        };
        report.validate()?;
        Ok(report)
    }

    /// The file's text.
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] if the result is one [`Report::parse`] would refuse.
    pub fn to_text(&self) -> Result<String, FileError> {
        self.validate()?;
        Ok(file::to_text([
            ("outcome", self.outcome.word().into()),
            ("snapshot", self.snapshot.as_deref().into()),
            ("safety_snapshot", self.safety_snapshot.as_deref().into()),
            ("home", file::home_word(self.home).into()),
            ("message", cut(&self.message).into()),
            ("when", self.when.into()),
        ]))
    }

    /// Reads `dir/result.json`.
    ///
    /// # Errors
    ///
    /// [`FileError::Io`] if it's missing or unreadable, [`FileError::Invalid`] as for
    /// [`Report::parse`].
    pub fn load(dir: &Path) -> Result<Self, FileError> {
        Self::parse(&file::load(dir, RESULT_FILE)?).map_err(|error| error.within(RESULT_FILE))
    }

    /// Writes `dir/result.json` in one step (temporary file, fsync, rename).
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] as for [`Report::to_text`], with nothing written;
    /// [`FileError::Io`] if the write fails, with the file as it was.
    pub fn save(&self, dir: &Path) -> Result<(), FileError> {
        file::save(dir, RESULT_FILE, &self.to_text()?)
    }

    fn validate(&self) -> Result<(), FileError> {
        let only_failed = |key: &str| {
            if self.outcome == Outcome::Failed {
                return Ok(());
            }
            Err(FileError::Invalid(format!(
                "{key:?} is null, and the outcome isn't \"failed\""
            )))
        };
        match &self.snapshot {
            Some(snapshot) => file::check_snapshot("snapshot", snapshot)?,
            None => only_failed("snapshot")?,
        }
        if let Some(safety) = &self.safety_snapshot {
            file::check_snapshot("safety_snapshot", safety)?;
        }
        self.when
            .map_or(Ok(()), |when| file::check_time("when", when))
    }
}

/// In front of a message that lost its start.
const CUT_MARK: &str = "... ";

/// `message` if it fits [`MAX_MESSAGE_BYTES`]. Else its end, starting on a character, behind
/// [`CUT_MARK`], and that long at most together. The end is what's kept: rsync's summary and
/// exit code come last.
fn cut(message: &str) -> Cow<'_, str> {
    if message.len() <= MAX_MESSAGE_BYTES {
        return Cow::Borrowed(message);
    }
    let mut start = message.len() - (MAX_MESSAGE_BYTES - CUT_MARK.len());
    while !message.is_char_boundary(start) {
        start += 1;
    }
    Cow::Owned(format!("{CUT_MARK}{}", &message[start..]))
}

/// `RestoreResult` on the bus, D-Bus type `(sssxss)`: `(state, snapshot, message, when, home,
/// safety_snapshot)`. `state` is `ready`, an [`Outcome`]'s word, or `""` for nothing; `""`
/// and `0` stand for a `null` snapshot or time. `home` is `keep` or `restore` with an
/// outcome, else `""`; `safety_snapshot` is `""` if none was taken.
pub type WireRestoreResult = (String, String, String, i64, String, String);

/// Where the restore stands, as `RestoreResult` tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultState {
    /// No plan is ready and there's no result.
    None,
    /// A plan waits at the ready prompt.
    Ready,
    /// The last `result.json`.
    Ended(Outcome),
}

/// What `RestoreResult` answers: the window asks when it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreResult {
    pub state: ResultState,
    pub snapshot: Option<String>,
    pub message: String,
    pub when: Option<i64>,
    /// Whether home was kept or restored: `Some` with [`ResultState::Ended`], from the
    /// file; `None` otherwise.
    pub home: Option<Home>,
    /// The safety snapshot of the last result, if one was taken.
    pub safety_snapshot: Option<String>,
}

impl RestoreResult {
    #[must_use]
    pub fn none() -> Self {
        Self {
            state: ResultState::None,
            snapshot: None,
            message: String::new(),
            when: None,
            home: None,
            safety_snapshot: None,
        }
    }

    #[must_use]
    pub fn ready(snapshot: &str) -> Self {
        Self {
            state: ResultState::Ready,
            snapshot: Some(snapshot.to_owned()),
            ..Self::none()
        }
    }

    /// The last result as the wire carries it: its outcome, snapshot, message and time, the
    /// home choice and the safety snapshot.
    #[must_use]
    pub fn of(report: &Report) -> Self {
        Self {
            state: ResultState::Ended(report.outcome),
            snapshot: report.snapshot.clone(),
            message: report.message.clone(),
            when: report.when,
            home: Some(report.home),
            safety_snapshot: report.safety_snapshot.clone(),
        }
    }

    #[must_use]
    pub fn to_wire(&self) -> WireRestoreResult {
        let state = match self.state {
            ResultState::None => "",
            ResultState::Ready => "ready",
            ResultState::Ended(outcome) => outcome.word(),
        };
        (
            state.to_owned(),
            self.snapshot.clone().unwrap_or_default(),
            self.message.clone(),
            self.when.unwrap_or(0),
            self.home
                .map(file::home_word)
                .unwrap_or_default()
                .to_owned(),
            self.safety_snapshot.clone().unwrap_or_default(),
        )
    }

    /// What the helper sent.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Helper`] for a state or home word this version doesn't know, and for
    /// an outcome without a home word.
    pub fn from_wire(wire: WireRestoreResult) -> crate::Result<Self> {
        let (state, snapshot, message, when, home, safety_snapshot) = wire;
        let state = match state.as_str() {
            "" => ResultState::None,
            "ready" => ResultState::Ready,
            word => Outcome::from_word(word)
                .map(ResultState::Ended)
                .ok_or_else(|| crate::Error::Helper(format!("unknown restore state {word:?}")))?,
        };
        let home = match home.as_str() {
            "" if !matches!(state, ResultState::Ended(_)) => None,
            "keep" => Some(Home::Keep),
            "restore" => Some(Home::Restore),
            word => {
                return Err(crate::Error::Helper(format!(
                    "unknown restore home {word:?}"
                )));
            }
        };
        Ok(Self {
            state,
            snapshot: (!snapshot.is_empty()).then_some(snapshot),
            message,
            when: (when != 0).then_some(when),
            home,
            safety_snapshot: (!safety_snapshot.is_empty()).then_some(safety_snapshot),
        })
    }
}

#[cfg(test)]
mod tests {

    /// `RestoreResult`'s answer: a ready plan, the last `result.json`, or nothing;
    /// `""` and `0` stand for a `null` snapshot or time on the wire, and `""` for no home
    /// choice and no safety snapshot.
    #[test]
    fn the_restore_result_survives_the_bus() {
        let none = RestoreResult::none();
        assert_eq!(
            none.to_wire(),
            (
                String::new(),
                String::new(),
                String::new(),
                0,
                String::new(),
                String::new()
            )
        );
        assert_eq!(RestoreResult::from_wire(none.to_wire()).unwrap(), none);
        let ready = RestoreResult::ready("2026-09-25_11-28-53");
        let wire = ready.to_wire();
        assert_eq!(wire.0, "ready");
        assert_eq!((wire.4.as_str(), wire.5.as_str()), ("", ""));
        assert_eq!(RestoreResult::from_wire(ready.to_wire()).unwrap(), ready);
        for outcome in Outcome::ALL {
            for (home, word) in [(Home::Keep, "keep"), (Home::Restore, "restore")] {
                for safety in [Some("2026-10-02_07-00-00"), None] {
                    let report = Report {
                        outcome,
                        snapshot: (outcome != Outcome::Failed)
                            .then(|| "2026-09-25_11-28-53".to_owned()),
                        safety_snapshot: safety.map(str::to_owned),
                        home,
                        message: "some words\nand more".to_owned(),
                        when: (outcome != Outcome::NotStarted).then_some(1_790_000_000),
                    };
                    let result = RestoreResult::of(&report);
                    assert_eq!(result.state, ResultState::Ended(outcome));
                    assert_eq!(result.home, Some(home));
                    assert_eq!(result.safety_snapshot, report.safety_snapshot);
                    let wire = result.to_wire();
                    assert_eq!(wire.0, outcome.word());
                    assert_eq!(wire.1.is_empty(), report.snapshot.is_none());
                    assert_eq!(wire.3 == 0, report.when.is_none());
                    assert_eq!(wire.4, word);
                    assert_eq!(wire.5, safety.unwrap_or(""));
                    assert_eq!(RestoreResult::from_wire(wire).unwrap(), result);
                }
            }
        }
        let wire = |state: &str, home: &str| {
            (
                state.to_owned(),
                String::new(),
                String::new(),
                0,
                home.to_owned(),
                String::new(),
            )
        };
        // A state this version doesn't know.
        assert!(RestoreResult::from_wire(wire("moon", "")).is_err());
        // A home word it doesn't know, and a result that doesn't say what happened to home.
        assert!(RestoreResult::from_wire(wire("done", "kept")).is_err());
        assert!(RestoreResult::from_wire(wire("done", "")).is_err());
    }

    /// A `result.json` as every build since the first has written it (the six fields, the
    /// safety snapshot a name or `null`): the home choice and the safety snapshot reach the
    /// wire from the file.
    #[test]
    fn an_earlier_builds_result_json_still_reads_and_gives_home_and_the_safety_snapshot() {
        let dir = temp_dir("result-earlier-build");
        for (home, safety, wire_safety) in [
            ("keep", "\"2026-10-01_09-15-42\"", SAFETY),
            ("restore", "null", ""),
        ] {
            let text = format!(
                r#"{{
  "version": 1,
  "outcome": "done",
  "snapshot": "2026-09-25_11-28-00",
  "safety_snapshot": {safety},
  "home": "{home}",
  "message": "",
  "when": 1790000600
}}
"#
            );
            std::fs::write(dir.join(RESULT_FILE), text).unwrap();
            let wire = RestoreResult::of(&Report::load(&dir).unwrap()).to_wire();
            assert_eq!(
                wire,
                (
                    "done".to_owned(),
                    SNAPSHOT.to_owned(),
                    String::new(),
                    1_790_000_600,
                    home.to_owned(),
                    wire_safety.to_owned()
                )
            );
        }
        // A file without the two fields was never written by any build: it's refused like
        // any other that isn't whole, and the helper answers that there's no result.
        std::fs::write(
            dir.join(RESULT_FILE),
            "{\n  \"version\": 1,\n  \"outcome\": \"done\",\n  \"snapshot\": \"2026-09-25_11-28-00\",\n  \"message\": \"\",\n  \"when\": 1790000600\n}\n",
        )
        .unwrap();
        assert_eq!(
            invalid(Report::load(&dir)),
            "result.json: no \"safety_snapshot\""
        );
    }
    use super::super::file::tests::temp_dir;
    use super::*;

    const SNAPSHOT: &str = "2026-09-25_11-28-00";
    const SAFETY: &str = "2026-10-01_09-15-42";

    fn invalid<T: std::fmt::Debug>(result: Result<T, FileError>) -> String {
        match result {
            Err(FileError::Invalid(reason)) => reason,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn a_new_state_has_no_attempt_and_nothing_written() {
        assert_eq!(
            State::default(),
            State {
                attempts: 0,
                written: false,
                step: Step::Armed,
                problems: false,
                boots: 0,
            }
        );
        assert_eq!(
            State::default().to_text().unwrap(),
            r#"{
  "version": 1,
  "attempts": 0,
  "written": false,
  "step": "armed",
  "problems": false,
  "boots": 0
}
"#
        );
    }

    fn state(attempts: u32, step: Step, problems: bool) -> State {
        State {
            attempts,
            written: attempts > 0,
            step,
            problems,
            boots: attempts,
        }
    }

    fn state_text(attempts: &str, written: &str, step: &str, problems: &str) -> String {
        format!(
            r#"{{"version": 1, "attempts": {attempts}, "written": {written}, "step": {step}, "problems": {problems}, "boots": 3}}"#
        )
    }

    /// The offline boots that began are counted, so a helper that dies in every one of them
    /// is stopped (`apply`).
    #[test]
    fn the_boot_count_is_kept_and_capped() {
        assert_eq!(MAX_BOOTS, 5);
        for boots in [1, 3, 5] {
            let state = State {
                boots,
                ..state(1, Step::Copy, false)
            };
            assert_eq!(State::parse(&state.to_text().unwrap()).unwrap(), state);
        }
        let over = State {
            boots: 6,
            ..state(1, Step::Copy, false)
        };
        assert_eq!(invalid(over.to_text()), "\"boots\" isn't 0 to 5");
        let text = state_text("1", "true", "\"copy\"", "false");
        for (boots, reason) in [
            ("6", "\"boots\" isn't 0 to 5"),
            ("-1", "\"boots\" isn't 0 to 5"),
            ("\"3\"", "\"boots\" isn't 0 to 5"),
        ] {
            let text = text.replace("\"boots\": 3", &format!("\"boots\": {boots}"));
            assert_eq!(invalid(State::parse(&text)), reason, "{boots}");
        }
        assert_eq!(
            invalid(State::parse(&text.replace(", \"boots\": 3", ""))),
            "no \"boots\""
        );
    }

    /// The boot is counted first, and a boot starts one copy at most.
    #[test]
    fn more_attempts_than_boots_are_refused() {
        let state = State {
            boots: 1,
            ..state(2, Step::Copy, false)
        };
        assert_eq!(
            invalid(state.to_text()),
            "more attempts were counted than boots"
        );
    }

    #[test]
    fn every_step_has_its_word() {
        for (step, word) in [
            (Step::Armed, "armed"),
            (Step::Copy, "copy"),
            (Step::BootFiles, "boot-files"),
            (Step::Refresh, "boot-refresh"),
            (Step::End, "end"),
        ] {
            let text = state(1, step, false).to_text();
            if step == Step::Armed {
                // Armed is before the first copy.
                assert!(matches!(text, Err(FileError::Invalid(_))));
                assert!(state(0, step, false).to_text().unwrap().contains(word));
            } else {
                assert!(text.unwrap().contains(&format!("\"step\": \"{word}\"")));
            }
        }
    }

    /// The step and the counters are saved together, so they agree.
    #[test]
    fn a_step_that_doesnt_fit_the_attempts_is_refused() {
        for (attempts, step, problems, reason) in [
            (
                1,
                Step::Armed,
                false,
                "an attempt was counted but the step is \"armed\"",
            ),
            (
                0,
                Step::Copy,
                false,
                "the step is \"copy\" but no attempt was counted",
            ),
            (
                0,
                Step::BootFiles,
                false,
                "the step is \"boot-files\" but no attempt was counted",
            ),
            (
                1,
                Step::Copy,
                true,
                "problems are marked before the copy ended",
            ),
            (
                0,
                Step::End,
                true,
                "problems are marked but no attempt was counted",
            ),
        ] {
            let state = state(attempts, step, problems);
            assert_eq!(invalid(state.to_text()), reason, "{state:?}");
        }
        assert_eq!(
            invalid(State::parse(&state_text("1", "true", "\"ready\"", "false"))),
            "\"step\" isn't a step"
        );
    }

    #[test]
    fn every_state_an_apply_can_reach_round_trips() {
        for state in [
            state(0, Step::Armed, false),
            state(1, Step::Copy, false),
            state(3, Step::Copy, false),
            state(2, Step::BootFiles, false),
            state(2, Step::BootFiles, true),
            state(2, Step::Refresh, false),
            state(1, Step::Refresh, true),
            state(0, Step::End, false),
            state(3, Step::End, true),
        ] {
            assert_eq!(State::parse(&state.to_text().unwrap()).unwrap(), state);
        }
    }

    /// The apply sets both before the copy, so a counted attempt has always written.
    #[test]
    fn an_attempt_without_the_written_flag_is_refused() {
        assert_eq!(
            invalid(State::parse(&state_text("1", "false", "\"copy\"", "false"))),
            "an attempt was counted but nothing is marked written"
        );
        assert_eq!(
            invalid(State::parse(&state_text("0", "true", "\"armed\"", "false"))),
            "something is marked written but no attempt was counted"
        );
    }

    #[test]
    fn more_attempts_than_a_restore_gets_are_refused() {
        assert_eq!(MAX_ATTEMPTS, 3);
        assert_eq!(
            invalid(State::parse(&state_text("4", "true", "\"copy\"", "false"))),
            "\"attempts\" isn't 0 to 3"
        );
        let state = state(4, Step::Copy, false);
        assert!(matches!(state.to_text(), Err(FileError::Invalid(_))));
    }

    #[test]
    fn a_state_field_of_the_wrong_kind_is_refused() {
        let copy = "\"copy\"";
        for (text, reason) in [
            (
                state_text("\"1\"", "true", copy, "false"),
                "\"attempts\" isn't 0 to 3",
            ),
            (
                state_text("-1", "true", copy, "false"),
                "\"attempts\" isn't 0 to 3",
            ),
            (
                state_text("1", "1", copy, "false"),
                "\"written\" isn't true or false",
            ),
            (state_text("1", "true", "1", "false"), "\"step\" isn't text"),
            (
                state_text("1", "true", copy, "null"),
                "\"problems\" isn't true or false",
            ),
            (
                r#"{"version": 1, "attempts": 1}"#.to_owned(),
                "no \"written\"",
            ),
            // The two-field file of before the step was saved.
            (
                r#"{"version": 1, "attempts": 1, "written": true}"#.to_owned(),
                "no \"step\"",
            ),
            (
                state_text("1", "true", copy, "false").replace("\"version\": 1", "\"version\": 3"),
                "version 3 (this Apsis reads 1)",
            ),
        ] {
            assert_eq!(invalid(State::parse(&text)), reason, "{text}");
        }
    }

    #[test]
    fn a_saved_state_loads_from_state_json_and_replaces_the_one_before() {
        let dir = temp_dir("state-save");
        State::default().save(&dir).unwrap();
        let started = state(1, Step::Copy, false);
        started.save(&dir).unwrap();
        assert!(dir.join("state.json").is_file());
        assert_eq!(State::load(&dir).unwrap(), started);
    }

    #[test]
    fn a_state_file_cut_short_is_refused_and_named() {
        let dir = temp_dir("state-cut");
        std::fs::write(dir.join(STATE_FILE), "{\n  \"version\": 1,\n  \"attem").unwrap();
        assert_eq!(invalid(State::load(&dir)), "state.json: not a JSON object");
    }

    fn report() -> Report {
        Report {
            outcome: Outcome::Done,
            snapshot: Some(SNAPSHOT.to_owned()),
            safety_snapshot: Some(SAFETY.to_owned()),
            home: Home::Keep,
            message: String::new(),
            when: Some(1_790_000_600),
        }
    }

    /// The report of last resort. Only `failed` may lack the snapshot.
    #[test]
    fn a_failed_result_may_have_no_snapshot_and_no_time() {
        let minimal = Report {
            outcome: Outcome::Failed,
            snapshot: None,
            safety_snapshot: None,
            home: Home::Keep,
            message: "result could not be saved, see journal".to_owned(),
            when: None,
        };
        assert_eq!(
            minimal.to_text().unwrap(),
            r#"{
  "version": 1,
  "outcome": "failed",
  "snapshot": null,
  "safety_snapshot": null,
  "home": "keep",
  "message": "result could not be saved, see journal",
  "when": null
}
"#
        );
        assert_eq!(Report::parse(&minimal.to_text().unwrap()).unwrap(), minimal);
        // One of the two alone is fine too.
        for report in [
            Report {
                snapshot: Some(SNAPSHOT.to_owned()),
                ..minimal.clone()
            },
            Report {
                when: Some(1_790_000_600),
                ..minimal.clone()
            },
        ] {
            assert_eq!(Report::parse(&report.to_text().unwrap()).unwrap(), report);
        }
    }

    #[test]
    fn any_other_outcome_needs_its_snapshot() {
        for outcome in [
            Outcome::Done,
            Outcome::Problems,
            Outcome::BootKept,
            Outcome::BootBroken,
            Outcome::NotStarted,
        ] {
            let no_snapshot = Report {
                outcome,
                snapshot: None,
                ..report()
            };
            assert_eq!(
                invalid(no_snapshot.to_text()),
                "\"snapshot\" is null, and the outcome isn't \"failed\""
            );
        }
    }

    /// The clock is the time's only source: when it gives none, the result still says how
    /// the restore ended.
    #[test]
    fn any_outcome_may_have_no_time() {
        for outcome in [
            Outcome::Done,
            Outcome::Problems,
            Outcome::BootKept,
            Outcome::BootBroken,
            Outcome::NotStarted,
            Outcome::Failed,
        ] {
            let report = Report {
                outcome,
                when: None,
                ..report()
            };
            let text = report.to_text().unwrap();
            assert!(text.contains("\"when\": null"), "{text}");
            assert_eq!(Report::parse(&text).unwrap(), report);
        }
    }

    /// A hardware clock in local time puts the apply's clock hours off the plan's: a result's
    /// time is taken by itself, and no report knows a plan.
    #[test]
    fn a_results_time_is_only_checked_to_be_a_time() {
        for when in [1, 1_000, i64::MAX] {
            let report = Report {
                when: Some(when),
                ..report()
            };
            assert_eq!(Report::parse(&report.to_text().unwrap()).unwrap(), report);
        }
    }

    #[test]
    fn the_result_is_written_field_by_field() {
        assert_eq!(
            report().to_text().unwrap(),
            r#"{
  "version": 1,
  "outcome": "done",
  "snapshot": "2026-09-25_11-28-00",
  "safety_snapshot": "2026-10-01_09-15-42",
  "home": "keep",
  "message": "",
  "when": 1790000600
}
"#
        );
    }

    /// The words of the helper's `RestoreResult`.
    #[test]
    fn every_outcome_has_its_word_and_round_trips() {
        for (outcome, word) in [
            (Outcome::Done, "done"),
            (Outcome::Problems, "problems"),
            (Outcome::BootKept, "boot-kept"),
            (Outcome::BootBroken, "boot-broken"),
            (Outcome::NotStarted, "not-started"),
            (Outcome::Failed, "failed"),
        ] {
            assert_eq!(outcome.word(), word);
            let report = Report {
                outcome,
                safety_snapshot: None,
                home: Home::Restore,
                message: "rsync: write failed on \"/usr/lib/x\": No space left (28)\nrsync error: code 11".to_owned(),
                ..report()
            };
            let text = report.to_text().unwrap();
            assert!(text.contains(&format!("\"outcome\": \"{word}\"")), "{text}");
            assert_eq!(Report::parse(&text).unwrap(), report);
        }
    }

    #[test]
    fn a_result_field_of_the_wrong_kind_or_range_is_refused() {
        let text = report().to_text().unwrap();
        for (from, to, reason) in [
            ("\"done\"", "\"ready\"", "\"outcome\" isn't an outcome"),
            ("\"done\"", "\"\"", "\"outcome\" isn't an outcome"),
            ("\"done\"", "1", "\"outcome\" isn't text"),
            (SNAPSHOT, "newest", "\"snapshot\" isn't a snapshot name"),
            (SAFETY, "none", "\"safety_snapshot\" isn't a snapshot name"),
            (
                "\"keep\"",
                "\"kept\"",
                "\"home\" is neither \"keep\" nor \"restore\"",
            ),
            (
                "\"message\": \"\"",
                "\"message\": null",
                "\"message\" isn't text",
            ),
            ("1790000600", "0", "\"when\" isn't a time"),
            ("1790000600", "\"today\"", "\"when\" isn't a time"),
            (
                "\"version\": 1",
                "\"version\": 2",
                "version 2 (this Apsis reads 1)",
            ),
            (
                "\"when\"",
                "\"kernel\": \"6.9\",\n  \"when\"",
                "unknown field \"kernel\"",
            ),
        ] {
            assert_eq!(
                invalid(Report::parse(&text.replace(from, to))),
                reason,
                "{to}"
            );
        }
    }

    #[test]
    fn a_result_that_wouldnt_read_back_isnt_written() {
        let dir = temp_dir("result-bad");
        let bad = Report {
            when: Some(0),
            ..report()
        };
        assert!(matches!(bad.save(&dir), Err(FileError::Invalid(_))));
        assert!(!dir.join(RESULT_FILE).exists());
    }

    /// A failure's reason can be all of rsync's output: it's cut, never a reason to lose
    /// the result. The end is kept: rsync's summary and exit code come last.
    #[test]
    fn a_one_mebibyte_message_keeps_its_end_and_the_result_still_saves() {
        let dir = temp_dir("result-long");
        let end = "\nrsync error: error in file IO (code 11) at receiver.c(380)";
        let long = Report {
            outcome: Outcome::Failed,
            message: format!("{}{end}", "x".repeat(1 << 20)),
            ..report()
        };
        long.save(&dir).unwrap();
        let read = Report::load(&dir).unwrap();
        assert_eq!(read.message.len(), MAX_MESSAGE_BYTES);
        assert_eq!(
            read.message,
            format!("... {}{end}", "x".repeat(MAX_MESSAGE_BYTES - 4 - end.len()))
        );
        assert_eq!(
            read,
            Report {
                message: read.message.clone(),
                ..long
            }
        );
    }

    #[test]
    fn a_message_is_cut_at_a_character_boundary() {
        // Two-byte characters, then one byte: the cut falls inside a character, and moves
        // to the next one.
        let message = format!("{}a", "é".repeat(1 << 19));
        let cut = Report {
            message,
            ..report()
        };
        let read = Report::parse(&cut.to_text().unwrap()).unwrap();
        assert_eq!(read.message, format!("... {}a", "é".repeat(1021)));
        assert_eq!(read.message.len(), MAX_MESSAGE_BYTES - 1);
    }

    /// The worst case for the file's size: every byte written as a `\u00XX` escape.
    #[test]
    fn a_message_of_control_characters_still_fits_the_file() {
        let dir = temp_dir("result-escapes");
        let report = Report {
            message: "\u{1}".repeat(1 << 20),
            ..report()
        };
        report.save(&dir).unwrap();
        assert_eq!(
            Report::load(&dir).unwrap().message,
            format!("... {}", "\u{1}".repeat(MAX_MESSAGE_BYTES - 4))
        );
    }

    /// Reading a cut message back, and writing it again, changes nothing more.
    #[test]
    fn a_cut_message_isnt_cut_again() {
        let report = Report {
            message: "z".repeat(MAX_MESSAGE_BYTES + 1),
            ..report()
        };
        let once = Report::parse(&report.to_text().unwrap()).unwrap();
        assert_eq!(
            once.message,
            format!("... {}", "z".repeat(MAX_MESSAGE_BYTES - 4))
        );
        assert_eq!(Report::parse(&once.to_text().unwrap()).unwrap(), once);
    }

    #[test]
    fn a_message_at_the_limit_is_kept_whole() {
        let report = Report {
            message: "y".repeat(MAX_MESSAGE_BYTES),
            ..report()
        };
        assert_eq!(Report::parse(&report.to_text().unwrap()).unwrap(), report);
    }

    #[test]
    fn a_saved_result_loads_from_result_json() {
        let dir = temp_dir("result-save");
        report().save(&dir).unwrap();
        assert!(dir.join("result.json").is_file());
        assert_eq!(Report::load(&dir).unwrap(), report());
    }
}
