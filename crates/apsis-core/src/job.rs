// SPDX-License-Identifier: GPL-3.0-only

//! What `apsis-helper` is doing right now, as its `Job` method and `JobChanged` signal tell
//! every window and the panel: which kind of job, how far, and how it ended.
//!
//! It carries no comment, caller, error text or path: the signal goes to anyone on the system
//! bus. The error text goes only to the caller that started the job, in `Finished`.

/// A job on the bus, D-Bus type `(sssxdx)`: `(kind, state, snapshot, started, percent,
/// eta_seconds)`. Idle is `("", "", "", 0, -1, -1)`.
pub type WireJob = (String, String, String, i64, f64, i64);

/// What the helper's one lock is held for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Create,
    Delete,
    List,
    Configure,
}

impl JobKind {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Delete => "delete",
            Self::List => "list",
            Self::Configure => "configure",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "create" => Some(Self::Create),
            "delete" => Some(Self::Delete),
            "list" => Some(Self::List),
            "configure" => Some(Self::Configure),
            _ => None,
        }
    }
}

/// Where a job is. The last three are sent once, as it ends; after that the helper is idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    /// A create being stopped: rsync is being ended and its copy removed.
    Stopping,
    Done,
    Failed,
    Stopped,
}

impl JobState {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "running" => Some(Self::Running),
            "stopping" => Some(Self::Stopping),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "stopped" => Some(Self::Stopped),
            _ => None,
        }
    }

    /// Done, failed or stopped: the job is over.
    #[must_use]
    pub fn is_end(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Stopped)
    }
}

/// A job the helper runs, or has just ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub kind: JobKind,
    pub state: JobState,
    /// The snapshot being made or deleted; empty until a create has planned its name.
    pub snapshot: String,
    /// When it started, in Unix seconds.
    pub started: i64,
    /// `0.0..=100.0`, once rsync has said.
    pub percent: Option<f64>,
    pub eta_seconds: Option<u64>,
}

impl Job {
    #[must_use]
    pub fn new(kind: JobKind, started: i64) -> Self {
        Self {
            kind,
            state: JobState::Running,
            snapshot: String::new(),
            started,
            percent: None,
            eta_seconds: None,
        }
    }
}

/// `job` on the bus; `None` is idle.
#[must_use]
pub fn to_wire(job: Option<&Job>) -> WireJob {
    match job {
        None => (String::new(), String::new(), String::new(), 0, -1.0, -1),
        Some(job) => (
            job.kind.word().to_owned(),
            job.state.word().to_owned(),
            job.snapshot.clone(),
            job.started,
            job.percent.unwrap_or(-1.0),
            job.eta_seconds
                .and_then(|s| i64::try_from(s).ok())
                .unwrap_or(-1),
        ),
    }
}

/// The job the helper sent; `None` for idle, or for anything this version doesn't know (a
/// newer helper's kind of job), which it treats as idle rather than failing.
#[must_use]
pub fn from_wire(wire: WireJob) -> Option<Job> {
    let (kind, state, snapshot, started, percent, eta) = wire;
    Some(Job {
        kind: JobKind::from_word(&kind)?,
        state: JobState::from_word(&state)?,
        snapshot,
        started,
        percent: (0.0..=100.0).contains(&percent).then_some(percent),
        eta_seconds: u64::try_from(eta).ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_survive_the_bus_and_idle_is_none() {
        assert_eq!(from_wire(to_wire(None)), None);
        let mut job = Job::new(JobKind::Create, 1_790_000_000);
        job.snapshot = "2026-09-30_14-02-11".to_owned();
        assert_eq!(from_wire(to_wire(Some(&job))), Some(job.clone()));
        job.state = JobState::Stopping;
        job.percent = Some(42.5);
        job.eta_seconds = Some(90);
        assert_eq!(from_wire(to_wire(Some(&job))), Some(job));
        let unknown = (
            "restore".to_owned(),
            "running".to_owned(),
            String::new(),
            0,
            -1.0,
            -1,
        );
        assert_eq!(from_wire(unknown), None);
    }

    #[test]
    fn only_the_last_three_states_end_a_job() {
        for (state, end) in [
            (JobState::Running, false),
            (JobState::Stopping, false),
            (JobState::Done, true),
            (JobState::Failed, true),
            (JobState::Stopped, true),
        ] {
            assert_eq!(state.is_end(), end, "{state:?}");
            assert_eq!(JobState::from_word(state.word()), Some(state));
        }
    }
}
