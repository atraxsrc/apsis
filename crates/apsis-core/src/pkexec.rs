// SPDX-License-Identifier: GPL-3.0-only

use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::timeshift::{RunOutput, Runner};

const PKEXEC: &str = "pkexec";

/// [`Runner`] that runs the command as root through `pkexec`, without a shell.
///
/// `argv[0]` is resolved on the user's `PATH` first, so a missing program is reported as
/// [`io::ErrorKind::NotFound`] (and the polkit prompt names the full path). A missing `pkexec`
/// is reported as a different error, so it isn't mistaken for a missing program.
#[derive(Debug, Clone, Copy, Default)]
pub struct PkexecRunner;

impl Runner for PkexecRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty argv"))?;
        let path = env::var_os("PATH").unwrap_or_default();
        let program = find_in_path(program, &path).ok_or(io::ErrorKind::NotFound)?;

        let output = pkexec_command(&program, rest)
            .output()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => io::Error::other("pkexec not found (install polkit)"),
                _ => e,
            })?;
        Ok(RunOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// `pkexec --disable-internal-agent <program> <args...>` with a fixed locale.
///
/// - `--disable-internal-agent`: without a graphical polkit agent, fail instead of asking for
///   the password on whatever terminal the applet was started from.
/// - `LC_ALL=C.UTF-8`, no `LANGUAGE`: pkexec passes both through, and the `--list` parser
///   matches Timeshift's untranslated English. UTF-8 keeps non-ASCII comments intact.
/// - stdin is null so nothing can block waiting for input.
#[must_use]
pub fn pkexec_command(program: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(PKEXEC);
    command
        .arg("--disable-internal-agent")
        .arg(program)
        .args(args)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("LANGUAGE")
        .stdin(Stdio::null());
    command
}

/// Finds an executable file called `name` in a `PATH`-style list. A `name` containing a `/` is
/// used as given.
#[must_use]
pub fn find_in_path(name: &OsStr, path: &OsStr) -> Option<PathBuf> {
    let name = Path::new(name);
    if name.components().count() > 1 {
        return is_executable(name).then(|| name.to_owned());
    }
    env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
