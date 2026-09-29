// SPDX-License-Identifier: GPL-3.0-only

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::thread;

use crate::progress::read_segments;
use crate::runner::find_in_path;
use crate::runner::{RunOutput, Runner};

/// Lines of stderr kept.
const STDERR_LINES: usize = 20;

/// [`Runner`] for rsync: stdout goes nowhere (on a whole system it's a line per file, and all
/// of it is in `rsync-log` anyway), stderr's last lines are kept.
///
/// `argv[0]` is looked up in a fixed `PATH`. The environment is only that `PATH` and
/// `LC_ALL=C.UTF-8` (Timeshift's script exports the same, `RsyncTask.vala:177`), stdin is
/// null.
#[derive(Debug, Clone)]
pub struct QuietRunner {
    path: OsString,
    low_priority: bool,
}

/// `ionice -c 3 nice -n <to 19>`: idle I/O class and the lowest CPU priority, set before rsync
/// starts, so the processes rsync forks have them too. Both tools are in essential packages
/// (util-linux, coreutils) and are found on the same fixed `PATH`.
const IONICE_IDLE: [&str; 3] = ["ionice", "-c", "3"];
/// The niceness rsync runs at. `nice -n` adds to the current one, so the step is worked out.
const NICENESS: i32 = 19;

impl QuietRunner {
    pub fn new(path: impl Into<OsString>) -> Self {
        Self {
            path: path.into(),
            low_priority: false,
        }
    }

    /// Runs everything at idle I/O priority and niceness 19 (see [`IONICE_IDLE`]), so a
    /// snapshot doesn't slow the desktop down.
    #[must_use]
    pub fn low_priority(mut self) -> Self {
        self.low_priority = true;
        self
    }

    /// `name` on the fixed `PATH`, or a not-found error naming it.
    fn find(&self, name: &OsStr) -> io::Result<std::path::PathBuf> {
        find_in_path(name, OsStr::new(&self.path)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} not found", name.to_string_lossy()),
            )
        })
    }
}

impl QuietRunner {
    /// rsync as `argv` says, found on the fixed `PATH`, with stdout going to `stdout`.
    fn spawn(&self, argv: &[OsString], stdout: Stdio) -> io::Result<Child> {
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty argv"))?;
        let program = self.find(program)?;
        let mut command = if self.low_priority {
            // `nice` gets rsync's full path, so nothing is looked up twice.
            let [ionice, ionice_args @ ..] = IONICE_IDLE;
            let current = rustix::process::getpriority_process(None).unwrap_or(0);
            let mut command = Command::new(self.find(OsStr::new(ionice))?);
            command
                .args(ionice_args)
                .arg(self.find(OsStr::new("nice"))?)
                .arg("-n")
                .arg((NICENESS - current).max(0).to_string())
                .arg(program);
            command
        } else {
            Command::new(program)
        };
        command
            .args(rest)
            .env_clear()
            .env("PATH", &self.path)
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped())
            .spawn()
    }
}

impl Runner for QuietRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        let mut child = self.spawn(argv, Stdio::null())?;
        let stderr = stderr_tail(child.stderr.take());
        let status = child.wait()?;
        Ok(RunOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::new(),
            stderr,
        })
    }

    /// stdout is read (for `--info=progress2`) and handed over, but still not kept.
    fn run_streaming(
        &self,
        argv: &[OsString],
        on_segment: &mut dyn FnMut(&str) -> bool,
    ) -> io::Result<RunOutput> {
        let mut child = self.spawn(argv, Stdio::piped())?;
        // stderr on its own thread, so neither pipe fills up while the other is read.
        let stderr = child.stderr.take();
        let tail = thread::spawn(move || stderr_tail(stderr));
        let read = child.stdout.take().map_or(Ok(String::new()), |out| {
            read_segments(out, false, on_segment)
        });
        let status = child.wait()?;
        read?;
        Ok(RunOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::new(),
            stderr: tail.join().unwrap_or_default(),
        })
    }
}

/// The last [`STDERR_LINES`] lines of `stderr`, read to its end.
pub(crate) fn stderr_tail(stderr: Option<ChildStderr>) -> String {
    let mut tail = VecDeque::new();
    if let Some(stderr) = stderr {
        for line in BufReader::new(stderr).split(b'\n').map_while(Result::ok) {
            if tail.len() == STDERR_LINES {
                tail.pop_front();
            }
            tail.push_back(String::from_utf8_lossy(&line).into_owned());
        }
    }
    Vec::from(tail).join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> OsString {
        std::env::var_os("PATH").unwrap_or_default()
    }

    /// What a child started by `runner` reports about its own priority.
    fn priorities(runner: &QuietRunner) -> Vec<String> {
        let mut lines = Vec::new();
        let argv = ["sh", "-c", "nice; ionice -p $$"].map(OsString::from);
        let output = runner
            .run_streaming(&argv, &mut |segment| {
                lines.push(segment.trim().to_owned());
                true
            })
            .unwrap();
        assert!(output.success, "{}", output.stderr);
        lines
    }

    #[test]
    fn low_priority_runs_at_nice_19_and_idle_io() {
        let lines = priorities(&QuietRunner::new(path()).low_priority());
        assert_eq!(lines, ["19", "idle"]);
    }

    /// The child keeps this thread's niceness unchanged. Compared with the thread's own value,
    /// not a fixed number: the test process can be reniced from outside (a desktop scheduler),
    /// even to 19, which once made a `!= 19` check fail. Retried if it changes meanwhile.
    #[test]
    fn normal_priority_is_left_alone() {
        for _ in 0..5 {
            let before = rustix::process::getpriority_process(None).unwrap();
            let lines = priorities(&QuietRunner::new(path()));
            let after = rustix::process::getpriority_process(None).unwrap();
            if before == after {
                assert_eq!(lines[0], before.to_string());
                return;
            }
        }
        panic!("this thread's niceness kept changing");
    }

    #[test]
    fn a_missing_program_is_named() {
        let runner = QuietRunner::new("/nonexistent").low_priority();
        let error = runner.run(&["rsync".into()]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("rsync"), "{error}");
    }
}
