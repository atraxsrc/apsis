// SPDX-License-Identifier: GPL-3.0-only

//! Timeshift's settings file on disk: reading it with what the settings view needs, and
//! replacing it safely.
//!
//! A write never leaves a half-written file: the new text goes to a temporary file next to it,
//! is flushed to disk, then renamed over the old one (and the folder flushed too). The previous
//! file is kept first as `timeshift.json.bak`, the same way. Checking and editing the text is
//! `apsis_core::settings::edit`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use apsis_core::helper::WireSettingsInfo;
use apsis_core::settings::{self, BACKUP_PATH, CONFIG_PATH, LSBLK_ARGS, Settings};
use apsis_core::{Error, Result, Runner};

/// Timeshift's settings file and its backup.
pub struct Files {
    config: PathBuf,
    backup: PathBuf,
}

impl Files {
    /// `/etc/timeshift/timeshift.json` and `.bak`.
    pub fn system() -> Self {
        Self::new(CONFIG_PATH.into(), BACKUP_PATH.into())
    }

    pub fn new(config: PathBuf, backup: PathBuf) -> Self {
        Self { config, backup }
    }

    /// The file as it is.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConfig`] when there's none yet (Timeshift writes it on its first run).
    pub fn read(&self) -> Result<String> {
        fs::read_to_string(&self.config).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => {
                Error::InvalidConfig("not found; open Timeshift once to set it up".to_owned())
            }
            _ => Error::Io(error),
        })
    }

    /// Checks `settings` against the connected devices and writes them into the file, if it
    /// still reads `expected`. Returns whether anything changed.
    ///
    /// # Errors
    ///
    /// [`Error::SettingsChanged`], [`Error::InvalidSettings`], [`Error::InvalidConfig`], or
    /// what lsblk or the filesystem reported.
    pub fn write(&self, runner: &impl Runner, expected: &str, settings: &Settings) -> Result<bool> {
        let devices = settings::parse_lsblk(&lsblk(runner)?)?;
        let current = self.read()?;
        if current != expected {
            return Err(Error::SettingsChanged);
        }
        let text = settings::edit(&current, settings, &devices)?;
        if text == current {
            return Ok(false);
        }
        let mode = fs::metadata(&self.config)?.permissions().mode() & 0o7777;
        write_atomically(&self.backup, &current, mode)?;
        // Checked again right before the swap: Timeshift may have saved meanwhile.
        if self.read()? != current {
            return Err(Error::SettingsChanged);
        }
        write_atomically(&self.config, &text, mode)?;
        Ok(true)
    }
}

/// Replaces `path` with `text` in one step: a temporary file in the same folder, flushed,
/// renamed over `path`, then the folder flushed so the rename survives a crash.
fn write_atomically(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut name = path
        .file_name()
        .ok_or(io::ErrorKind::InvalidInput)?
        .to_owned();
    name.push(".apsis-tmp");
    let temp = dir.join(name);
    // Left over from a crash: never ours to keep.
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)?;
        // `mode` is masked by the umask on create; set it exactly.
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(dir)?.sync_all()
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// lsblk's JSON ([`LSBLK_ARGS`]).
fn lsblk(runner: &impl Runner) -> Result<String> {
    let argv: Vec<_> = LSBLK_ARGS.iter().map(Into::into).collect();
    let output = runner.run(&argv)?;
    if !output.success {
        return Err(Error::Helper(format!(
            "lsblk failed: {}",
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
}

/// What `ReadSettings` returns: the file, the devices, the users and whether `timeshift-gtk`
/// is open.
///
/// # Errors
///
/// The file can't be read, or lsblk fails.
pub fn info(files: &Files, runner: &impl Runner) -> Result<WireSettingsInfo> {
    let text = files.read()?;
    let devices = lsblk(runner)?;
    let passwd = fs::read_to_string("/etc/passwd")?;
    let users = settings::parse_passwd(&passwd)
        .into_iter()
        .map(|user| {
            let encrypted = encrypted_home(&user.name, &user.home);
            (user.name, user.home, encrypted)
        })
        .collect();
    Ok((text, devices, users, timeshift_gui_open()))
}

/// Timeshift's check for an ecryptfs home: the user's `Private.mnt` names the home folder.
fn encrypted_home(name: &str, home: &str) -> bool {
    let mount_file = format!("/home/.ecryptfs/{name}/.ecryptfs/Private.mnt");
    fs::read_to_string(mount_file).is_ok_and(|text| text.lines().any(|l| l.trim() == home))
}

/// Whether a `timeshift-gtk` process is running.
fn timeshift_gui_open() -> bool {
    let Ok(processes) = fs::read_dir("/proc") else {
        return false;
    };
    processes.flatten().any(|process| {
        fs::read_to_string(process.path().join("comm"))
            .is_ok_and(|comm| comm.trim() == "timeshift-gtk")
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use apsis_core::RunOutput;
    use apsis_core::settings::{Config, Level};

    use super::*;

    const CONFIG: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");

    /// Answers lsblk with the fixture, and remembers what it was asked.
    struct FakeLsblk;

    impl Runner for FakeLsblk {
        fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
            assert_eq!(argv, LSBLK_ARGS.map(OsString::from));
            Ok(RunOutput {
                success: true,
                code: Some(0),
                stdout: LSBLK.to_owned(),
                stderr: String::new(),
            })
        }
    }

    /// A fresh folder under the temp dir holding `timeshift.json` (mode 0644).
    fn folder() -> (PathBuf, Files) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apsis-helper-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("timeshift.json");
        fs::write(&config, CONFIG).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
        let files = Files::new(config, dir.join("timeshift.json.bak"));
        (dir, files)
    }

    fn daily(settings: &mut Settings) {
        settings.schedule[Level::Daily.index()] = true;
    }

    #[test]
    fn write_keeps_a_backup_and_the_mode() {
        let (dir, files) = folder();
        let mut settings = Config::parse(CONFIG).unwrap().settings();
        daily(&mut settings);
        assert!(files.write(&FakeLsblk, CONFIG, &settings).unwrap());

        let written = fs::read_to_string(dir.join("timeshift.json")).unwrap();
        assert_eq!(Config::parse(&written).unwrap().settings(), settings);
        assert_eq!(
            fs::read_to_string(dir.join("timeshift.json.bak")).unwrap(),
            CONFIG
        );
        let mode = |name: &str| fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            (mode("timeshift.json"), mode("timeshift.json.bak")),
            (0o644, 0o644)
        );
        assert!(!dir.join("timeshift.json.apsis-tmp").exists());

        // The next write replaces the one backup.
        let mut again = settings.clone();
        again.counts[Level::Daily.index()] = 9;
        assert!(files.write(&FakeLsblk, &written, &again).unwrap());
        assert_eq!(
            fs::read_to_string(dir.join("timeshift.json.bak")).unwrap(),
            written
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_changed_since_reading_is_left_alone() {
        let (dir, files) = folder();
        let mut settings = Config::parse(CONFIG).unwrap().settings();
        daily(&mut settings);
        let stale = CONFIG.replace("\"2\"", "\"4\"");
        assert!(matches!(
            files.write(&FakeLsblk, &stale, &settings),
            Err(Error::SettingsChanged)
        ));
        assert_eq!(
            fs::read_to_string(dir.join("timeshift.json")).unwrap(),
            CONFIG
        );
        assert!(!dir.join("timeshift.json.bak").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_settings_write_nothing() {
        let (dir, files) = folder();
        let mut settings = Config::parse(CONFIG).unwrap().settings();
        settings.counts[Level::Boot.index()] = 0;
        assert!(matches!(
            files.write(&FakeLsblk, CONFIG, &settings),
            Err(Error::InvalidSettings(_))
        ));
        assert_eq!(
            fs::read_to_string(dir.join("timeshift.json")).unwrap(),
            CONFIG
        );
        assert!(!dir.join("timeshift.json.bak").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unchanged_settings_write_nothing() {
        let (dir, files) = folder();
        let settings = Config::parse(CONFIG).unwrap().settings();
        assert!(!files.write(&FakeLsblk, CONFIG, &settings).unwrap());
        assert!(!dir.join("timeshift.json.bak").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_temp_files_are_replaced() {
        let (dir, files) = folder();
        fs::write(dir.join("timeshift.json.apsis-tmp"), "junk").unwrap();
        fs::write(dir.join("timeshift.json.bak.apsis-tmp"), "junk").unwrap();
        let mut settings = Config::parse(CONFIG).unwrap().settings();
        daily(&mut settings);
        assert!(files.write(&FakeLsblk, CONFIG, &settings).unwrap());
        assert!(!dir.join("timeshift.json.apsis-tmp").exists());
        assert!(!dir.join("timeshift.json.bak.apsis-tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_file_says_to_set_timeshift_up() {
        let files = Files::new(
            "/nonexistent/timeshift.json".into(),
            "/nonexistent/x.bak".into(),
        );
        assert!(matches!(files.read(), Err(Error::InvalidConfig(_))));
    }
}
