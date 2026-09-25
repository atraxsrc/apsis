// SPDX-License-Identifier: GPL-3.0-only

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader};
use std::process::{Command, Stdio};

use crate::pkexec::find_in_path;
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

impl Runner for QuietRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty argv"))?;
        let program = find_in_path(program, OsStr::new(&self.path)).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} not found", program.to_string_lossy()),
            )
        })?;
        let mut child = Command::new(program)
            .args(rest)
            .env_clear()
            .env("PATH", &self.path)
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut tail = VecDeque::new();
        if let Some(stderr) = child.stderr.take() {
            for line in BufReader::new(stderr).split(b'\n') {
                if tail.len() == STDERR_LINES {
                    tail.pop_front();
                }
                tail.push_back(String::from_utf8_lossy(&line?).into_owned());
            }
        }
        let status = child.wait()?;
        Ok(RunOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::new(),
            stderr: Vec::from(tail).join("\n"),
        })
    }
}
