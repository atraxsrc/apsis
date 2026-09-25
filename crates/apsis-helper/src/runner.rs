// SPDX-License-Identifier: GPL-3.0-only

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::{Command, Stdio};

use apsis_core::{RunOutput, Runner, find_in_path};

/// Where the helper looks for `timeshift`. Fixed: nothing from the caller or the environment
/// the helper was started with decides which program runs as root.
pub const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// [`Runner`] for the helper, which is already root: runs the argv directly, without a shell.
///
/// The environment is cleared and rebuilt like pkexec's (fixed `PATH`, root's `HOME`, `USER`
/// and `LOGNAME`), with `LC_ALL=C.UTF-8` so the `--list` parser sees Timeshift's untranslated
/// English. stdin is null so nothing can wait for input.
#[derive(Debug, Clone, Copy, Default)]
pub struct DirectRunner;

impl Runner for DirectRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty argv"))?;
        let program =
            find_in_path(program, OsStr::new(SAFE_PATH)).ok_or(io::ErrorKind::NotFound)?;
        let output = command(&program, rest).output()?;
        Ok(RunOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn command(program: &std::path::Path, args: &[OsString]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .env("PATH", SAFE_PATH)
        .env("HOME", "/root")
        .env("USER", "root")
        .env("LOGNAME", "root")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_has_only_the_fixed_environment() {
        let command = command(
            std::path::Path::new("/usr/bin/timeshift"),
            &["--list".into(), "--scripted".into()],
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--list", "--scripted"]);
        let mut env: Vec<_> = command
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.and_then(OsStr::to_str).unwrap()))
            .collect();
        env.sort_unstable();
        assert_eq!(
            env,
            [
                ("HOME", "/root"),
                ("LC_ALL", "C.UTF-8"),
                ("LOGNAME", "root"),
                ("PATH", SAFE_PATH),
                ("USER", "root"),
            ]
        );
    }

    #[test]
    fn missing_program_is_not_found() {
        let error = DirectRunner
            .run(&["apsis-no-such-program".into()])
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
