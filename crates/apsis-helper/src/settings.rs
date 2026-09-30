// SPDX-License-Identifier: GPL-3.0-only

//! Apsis's config file on disk (`/etc/apsis/config.toml`): reading it (or, before there is
//! one, importing Timeshift's settings), and replacing it safely.
//!
//! A write never leaves a half-written file: the new text goes to a temporary file next to it,
//! is flushed to disk, then renamed over the old one (and the folder flushed too). The previous
//! file is kept first as `config.toml.bak`, the same way. Both are `0644`, owned by root (the
//! helper). Checking the config is `apsis_core::config::validate`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use apsis_core::config::{self, BACKUP_PATH, CONFIG_PATH, Config, System, TIMESHIFT_CONFIG};
use apsis_core::helper::{WireConfigInfo, config_to_wire};
use apsis_core::settings::{self, LSBLK_ARGS, User};
use apsis_core::{Error, Result, Runner};

/// The mode of `config.toml` and its backup.
const MODE: u32 = 0o644;

/// Apsis's config file, its backup, and Timeshift's settings (read once, for the import).
pub struct Files {
    config: PathBuf,
    backup: PathBuf,
    timeshift: PathBuf,
}

impl Files {
    /// `/etc/apsis/config.toml`, `.bak`, and `/etc/timeshift/timeshift.json`.
    pub fn system() -> Self {
        Self::new(
            CONFIG_PATH.into(),
            BACKUP_PATH.into(),
            TIMESHIFT_CONFIG.into(),
        )
    }

    pub fn new(config: PathBuf, backup: PathBuf, timeshift: PathBuf) -> Self {
        Self {
            config,
            backup,
            timeshift,
        }
    }

    /// `config.toml` as it is; `None` when there's none yet.
    ///
    /// # Errors
    ///
    /// It's there but can't be read.
    pub fn read(&self) -> Result<Option<String>> {
        match fs::read_to_string(&self.config) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The config in effect: the file's (converted if it's version 1), else an import of
    /// Timeshift's settings, else the defaults; with notes on a conversion or import (see
    /// `apsis_core::config::effective`).
    ///
    /// # Errors
    ///
    /// A `config.toml` that can't be read.
    pub fn effective(
        &self,
        devices: &[settings::Device],
        system: &System,
    ) -> Result<(Config, Vec<String>)> {
        let text = self.read()?;
        let timeshift = if text.is_none() {
            fs::read_to_string(&self.timeshift).ok()
        } else {
            None
        };
        config::effective(text.as_deref(), timeshift.as_deref(), devices, system)
    }

    /// Checks `config` against the connected devices and writes it, if the file still reads
    /// `expected` (empty: there's no file yet). Returns whether anything changed.
    ///
    /// # Errors
    ///
    /// [`Error::ConfigChanged`], [`Error::InvalidSettings`], [`Error::InvalidConfig`] (the file
    /// there can't be read), or what lsblk or the filesystem reported.
    pub fn write(&self, runner: &impl Runner, expected: &str, config: &Config) -> Result<bool> {
        let devices = settings::parse_lsblk(&lsblk(runner)?)?;
        let current = self.read()?.unwrap_or_default();
        if current != expected {
            return Err(Error::ConfigChanged);
        }
        let old = if current.is_empty() {
            None
        } else {
            Some(Config::read(&current)?.device().to_owned())
        };
        config::validate(config, old.as_deref(), &devices)?;
        let text = config.to_text();
        if text == current {
            return Ok(false);
        }
        if let Some(dir) = self.config.parent() {
            fs::DirBuilder::new().recursive(true).create(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755))?;
        }
        if !current.is_empty() {
            write_atomically(&self.backup, &current, MODE)?;
        }
        // Checked again right before the swap.
        if self.read()?.unwrap_or_default() != current {
            return Err(Error::ConfigChanged);
        }
        write_atomically(&self.config, &text, MODE)?;
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
pub fn lsblk(runner: &impl Runner) -> Result<String> {
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

/// The users whose home folders the filters cover, from `/etc/passwd`, with ecryptfs homes
/// marked.
pub fn users() -> Result<Vec<User>> {
    let passwd = fs::read_to_string("/etc/passwd")?;
    Ok(settings::parse_passwd(&passwd)
        .into_iter()
        .map(|user| User {
            encrypted_home: encrypted_home(&user.name, &user.home),
            ..user
        })
        .collect())
}

/// What the conversion of old settings needs about this system: the users and the names in
/// `/home` (none if it can't be read).
///
/// # Errors
///
/// `/etc/passwd` can't be read.
pub fn system() -> Result<System> {
    let home_entries = fs::read_dir("/home")
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    Ok(System {
        users: users()?,
        home_entries,
    })
}

/// What `ReadConfig` returns: the file, the config in effect, the devices and notes on a
/// conversion or import.
///
/// # Errors
///
/// The file can't be read, or lsblk fails.
pub fn info(files: &Files, runner: &impl Runner) -> Result<WireConfigInfo> {
    let text = files.read()?.unwrap_or_default();
    let devices_json = lsblk(runner)?;
    let devices = settings::parse_lsblk(&devices_json)?;
    let (config, notes) = files.effective(&devices, &system()?)?;
    Ok((text, config_to_wire(&config), devices_json, notes))
}

/// Timeshift's check for an ecryptfs home: the user's `Private.mnt` names the home folder.
fn encrypted_home(name: &str, home: &str) -> bool {
    let mount_file = format!("/home/.ecryptfs/{name}/.ecryptfs/Private.mnt");
    fs::read_to_string(mount_file).is_ok_and(|text| text.lines().any(|l| l.trim() == home))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use apsis_core::RunOutput;

    use super::*;

    const TIMESHIFT: &str = include_str!("../../apsis-core/tests/fixtures/config-rsync.json");
    const LSBLK: &str = include_str!("../../apsis-core/tests/fixtures/lsblk.json");
    /// `sdb1` in the lsblk fixture: connected, ext4.
    const BACKUP_UUID: &str = "00000000-0000-0000-0000-000000000000";
    /// The unlocked LUKS filesystem in the fixture.
    const UNLOCKED_UUID: &str = "33333333-3333-3333-3333-333333333333";

    /// Answers lsblk with the fixture.
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

    /// A fresh folder under the temp dir, standing in for `/etc`: `apsis/` doesn't exist yet,
    /// `timeshift/timeshift.json` does.
    fn folder() -> (PathBuf, Files) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "apsis-helper-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("timeshift")).unwrap();
        fs::write(dir.join("timeshift/timeshift.json"), TIMESHIFT).unwrap();
        let files = Files::new(
            dir.join("apsis/config.toml"),
            dir.join("apsis/config.toml.bak"),
            dir.join("timeshift/timeshift.json"),
        );
        (dir, files)
    }

    fn config(uuid: &str, filters: &[&str]) -> Config {
        Config {
            backup_device_uuid: uuid.to_owned(),
            filters: filters.iter().map(|f| (*f).to_owned()).collect(),
            ..Config::default()
        }
    }

    fn current(text: &str) -> Config {
        match Config::read(text).unwrap() {
            config::Stored::Current(config) => config,
            other => panic!("not version 2: {other:?}"),
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn before_the_first_save_timeshifts_settings_are_imported() {
        let (dir, files) = folder();
        assert_eq!(files.read().unwrap(), None);
        let (config, notes) = files.effective(&[], &System::default()).unwrap();
        assert_eq!(config.backup_device_uuid, BACKUP_UUID);
        assert_eq!(config.filters.len(), 4);
        assert!(notes[0].starts_with("imported from"));
        // Without timeshift.json: empty, no notes.
        fs::remove_file(dir.join("timeshift/timeshift.json")).unwrap();
        assert_eq!(
            files.effective(&[], &System::default()).unwrap(),
            (Config::default(), Vec::new())
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_first_save_makes_the_file_and_then_timeshift_isnt_read() {
        let (dir, files) = folder();
        let first = config(BACKUP_UUID, &["+ /home/user1/**", "- *.iso"]);
        assert!(files.write(&FakeLsblk, "", &first).unwrap());
        let written = fs::read_to_string(dir.join("apsis/config.toml")).unwrap();
        assert_eq!(current(&written), first);
        assert_eq!(mode(&dir.join("apsis/config.toml")), 0o644);
        assert_eq!(mode(&dir.join("apsis")), 0o755);
        assert!(
            !dir.join("apsis/config.toml.bak").exists(),
            "nothing to back up"
        );
        assert!(!dir.join("apsis/config.toml.apsis-tmp").exists());
        // From now on the file decides; Timeshift's settings are left alone.
        let (config, notes) = files.effective(&[], &System::default()).unwrap();
        assert_eq!((config, notes.len()), (first, 0));
        assert_eq!(
            fs::read_to_string(dir.join("timeshift/timeshift.json")).unwrap(),
            TIMESHIFT
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_later_save_keeps_a_backup_of_the_previous_file() {
        let (dir, files) = folder();
        let first = config(BACKUP_UUID, &[]);
        files.write(&FakeLsblk, "", &first).unwrap();
        let first_text = fs::read_to_string(dir.join("apsis/config.toml")).unwrap();
        let second = config(BACKUP_UUID, &["- *.mp3"]);
        assert!(files.write(&FakeLsblk, &first_text, &second).unwrap());
        assert_eq!(
            fs::read_to_string(dir.join("apsis/config.toml.bak")).unwrap(),
            first_text
        );
        assert_eq!(mode(&dir.join("apsis/config.toml.bak")), 0o644);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_changed_since_reading_is_left_alone() {
        let (dir, files) = folder();
        let first = config(BACKUP_UUID, &[]);
        files.write(&FakeLsblk, "", &first).unwrap();
        // The caller read "no file", but there is one now.
        assert!(matches!(
            files.write(&FakeLsblk, "", &config(BACKUP_UUID, &["- x"])),
            Err(Error::ConfigChanged)
        ));
        assert!(!dir.join("apsis/config.toml.bak").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_encrypted_or_missing_device_is_never_written() {
        let (dir, files) = folder();
        for uuid in [UNLOCKED_UUID, "99999999-9999-9999-9999-999999999999", ""] {
            assert!(
                matches!(
                    files.write(&FakeLsblk, "", &config(uuid, &[])),
                    Err(Error::InvalidSettings(_))
                ),
                "{uuid}"
            );
        }
        assert!(!dir.join("apsis/config.toml").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unchanged_config_writes_nothing() {
        let (dir, files) = folder();
        let first = config(BACKUP_UUID, &[]);
        files.write(&FakeLsblk, "", &first).unwrap();
        let text = fs::read_to_string(dir.join("apsis/config.toml")).unwrap();
        assert!(!files.write(&FakeLsblk, &text, &first).unwrap());
        assert!(!dir.join("apsis/config.toml.bak").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_temp_files_are_replaced() {
        let (dir, files) = folder();
        fs::create_dir_all(dir.join("apsis")).unwrap();
        fs::write(dir.join("apsis/config.toml.apsis-tmp"), "junk").unwrap();
        files
            .write(&FakeLsblk, "", &config(BACKUP_UUID, &[]))
            .unwrap();
        assert!(!dir.join("apsis/config.toml.apsis-tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_version_1_file_is_converted_and_saved_as_version_2_with_a_backup() {
        let (dir, files) = folder();
        fs::create_dir_all(dir.join("apsis")).unwrap();
        let old =
            format!("version = 1\nbackup_device_uuid = \"{BACKUP_UUID}\"\nfilters = [\"*.iso\"]\n");
        fs::write(dir.join("apsis/config.toml"), &old).unwrap();
        let (converted, notes) = files.effective(&[], &System::default()).unwrap();
        assert_eq!(converted.filters, ["- *.iso"]);
        assert!(notes[0].starts_with("settings from Apsis 0.3"), "{notes:?}");
        // The device is unchanged, so it may be unplugged; the write goes through.
        assert!(files.write(&NoDevices, &old, &converted).unwrap());
        let written = fs::read_to_string(dir.join("apsis/config.toml")).unwrap();
        assert_eq!(current(&written), converted);
        assert_eq!(
            fs::read_to_string(dir.join("apsis/config.toml.bak")).unwrap(),
            old
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// lsblk with no devices at all.
    struct NoDevices;

    impl Runner for NoDevices {
        fn run(&self, _: &[OsString]) -> io::Result<RunOutput> {
            Ok(RunOutput {
                success: true,
                code: Some(0),
                stdout: "{\"blockdevices\": []}".to_owned(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn a_broken_config_file_is_an_error_not_an_import() {
        let (dir, files) = folder();
        fs::create_dir_all(dir.join("apsis")).unwrap();
        fs::write(dir.join("apsis/config.toml"), "version = 7").unwrap();
        assert!(matches!(
            files.effective(&[], &System::default()),
            Err(Error::InvalidConfig(_))
        ));
        fs::remove_dir_all(dir).unwrap();
    }
}
