// SPDX-License-Identifier: GPL-3.0-only

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::thread;

use crate::pkexec::find_in_path;
use crate::progress::read_segments;
use crate::timeshift::{RunOutput, Runner};

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
}

impl QuietRunner {
    pub fn new(path: impl Into<OsString>) -> Self {
        Self { path: path.into() }
    }
}

impl QuietRunner {
    /// rsync as `argv` says, found on the fixed `PATH`, with stdout going to `stdout`.
    fn spawn(&self, argv: &[OsString], stdout: Stdio) -> io::Result<Child> {
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty argv"))?;
        let program = find_in_path(program, OsStr::new(&self.path)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} not found", program.to_string_lossy()),
            )
        })?;
        Command::new(program)
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
