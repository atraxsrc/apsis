// SPDX-License-Identifier: GPL-3.0-only

//! `PkexecRunner` pieces that can be checked without running anything as root.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use apsis_core::{find_in_path, pkexec_command};

/// A fresh directory under Cargo's per-test temp dir.
fn temp_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(path: &Path, mode: u32) {
    fs::write(path, "").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn pkexec_command_passes_argv_without_a_shell() {
    let args: Vec<OsString> = ["--create", "--comments", "a; rm -rf / $(x)", "--scripted"]
        .map(OsString::from)
        .to_vec();
    let command = pkexec_command(Path::new("/usr/bin/timeshift"), &args);

    assert_eq!(command.get_program(), "pkexec");
    let got: Vec<&OsStr> = command.get_args().collect();
    assert_eq!(
        got,
        [
            "--disable-internal-agent",
            "/usr/bin/timeshift",
            "--create",
            "--comments",
            "a; rm -rf / $(x)",
            "--scripted",
        ]
    );
}

#[test]
fn pkexec_command_fixes_the_locale() {
    let command = pkexec_command(Path::new("/usr/bin/timeshift"), &[]);
    let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("LC_ALL"), Some(OsStr::new("C.UTF-8")))));
    assert!(envs.contains(&(OsStr::new("LANGUAGE"), None)));
}

#[test]
fn find_in_path_returns_first_executable_match() {
    let dir = temp_dir("find_in_path_first");
    let (a, b) = (dir.join("a"), dir.join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    touch(&a.join("timeshift"), 0o644); // not executable: skipped
    touch(&b.join("timeshift"), 0o755);

    let path = std::env::join_paths([&a, &b]).unwrap();
    assert_eq!(
        find_in_path(OsStr::new("timeshift"), &path),
        Some(b.join("timeshift"))
    );
}

#[test]
fn find_in_path_misses() {
    let dir = temp_dir("find_in_path_miss");
    fs::create_dir_all(dir.join("timeshift")).unwrap(); // a directory, not a program
    let path = std::env::join_paths([&dir]).unwrap();
    assert_eq!(find_in_path(OsStr::new("timeshift"), &path), None);
    assert_eq!(find_in_path(OsStr::new("timeshift"), OsStr::new("")), None);
}

#[test]
fn find_in_path_ignores_relative_path_entries() {
    let dir = temp_dir("find_in_path_relative");
    touch(&dir.join("timeshift"), 0o755);
    let relative = dir.strip_prefix("/").unwrap();
    let path = std::env::join_paths([relative, Path::new(".")]).unwrap();
    assert_eq!(find_in_path(OsStr::new("timeshift"), &path), None);
}

#[test]
fn find_in_path_takes_a_path_as_given() {
    let dir = temp_dir("find_in_path_given");
    let program = dir.join("timeshift");
    touch(&program, 0o755);
    assert_eq!(
        find_in_path(program.as_os_str(), OsStr::new("")),
        Some(program.clone())
    );
    assert_eq!(
        find_in_path(dir.join("nope").as_os_str(), OsStr::new("")),
        None
    );
}
