// SPDX-License-Identifier: GPL-3.0-only

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use crate::pkexec::find_in_path;
use crate::timeshift::{RunOutput, Runner};

/// Lines of stderr kept.
const STDERR_LINES: usize = 20;

/// [`Runner`] for a restore's rsync: stdout is kept whole (the itemized list the plan is read
/// from), stderr's last lines are kept.
///
/// `argv[0]` is looked up in a fixed `PATH`. The environment is only that `PATH` and
/// `LC_ALL=C.UTF-8`; stdin is null. Descriptors the caller made inheritable stay open in rsync
/// (folder mode's destination, see `dest`).
#[derive(Debug, Clone)]
pub struct RsyncRunner {
    path: OsString,
}

impl RsyncRunner {
    pub fn new(path: impl Into<OsString>) -> Self {
        Self { path: path.into() }
    }
}

impl Runner for RsyncRunner {
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
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // stderr on its own thread, so neither pipe fills up while the other is read.
        let stderr = child.stderr.take();
        let tail = std::thread::spawn(move || {
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
        });
        let mut stdout = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            out.read_to_end(&mut stdout)?;
        }
        let status = child.wait()?;
        Ok(RunOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: tail.join().unwrap_or_default(),
        })
    }
}
