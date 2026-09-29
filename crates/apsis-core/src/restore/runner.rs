// SPDX-License-Identifier: GPL-3.0-only

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Child, Command, Stdio};
use std::thread;

use crate::native::runner::stderr_tail;
use crate::progress::read_segments;
use crate::runner::find_in_path;
use crate::runner::{RunOutput, Runner};

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

impl RsyncRunner {
    fn spawn(&self, argv: &[OsString]) -> io::Result<Child> {
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
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
}

impl Runner for RsyncRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        self.run_streaming(argv, &mut |_| false)
    }

    /// stdout is kept, except the pieces `on_segment` takes (`--info=progress2` lines), so
    /// the itemized list reads as before.
    fn run_streaming(
        &self,
        argv: &[OsString],
        on_segment: &mut dyn FnMut(&str) -> bool,
    ) -> io::Result<RunOutput> {
        let mut child = self.spawn(argv)?;
        // stderr on its own thread, so neither pipe fills up while the other is read.
        let stderr = child.stderr.take();
        let tail = thread::spawn(move || stderr_tail(stderr));
        let stdout = child.stdout.take().map_or(Ok(String::new()), |out| {
            read_segments(out, true, on_segment)
        });
        let status = child.wait()?;
        Ok(RunOutput {
            success: status.success(),
            code: status.code(),
            stdout: stdout?,
            stderr: tail.join().unwrap_or_default(),
        })
    }
}
