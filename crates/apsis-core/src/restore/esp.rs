// SPDX-License-Identifier: GPL-3.0-only

//! The boot files on the ESP (PLAN 6b.6, steps 4 and 6): their backup before the boot
//! refresh, the check after it, and putting them back when the check fails.
//!
//! kernelstub boots the kernel and initrd from copies on the ESP. After the copy, the boot
//! refresh (`update-initramfs`, `kernelstub`) rewrites them for the restored kernel. Before
//! that the four files are copied to `esp-backup/` in the state folder, on `/` and protected
//! from the restore. If the refreshed ESP doesn't check out, they're put back, and the ESP
//! boots the kernel it booted before, whose `/boot` files and modules the filter's rule 10
//! kept. Everything here takes the ESP and the root as paths, so it's tested on temp trees.

use std::fs::{self, DirBuilder, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use rustix::io::Errno;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::file::{self, FileError};
use super::filter::is_kernel_version;
use super::refusal::Refusal;
use crate::usage::is_plain_uuid;

/// The backup's folder in the state folder ([`file::DIR`]).
pub const BACKUP_DIR: &str = "esp-backup";

/// In [`BACKUP_DIR`], written last: a backup without it isn't one.
pub const MANIFEST_FILE: &str = "manifest.json";

/// A put-back writes beside the file under this name, then renames.
const TEMP_SUFFIX: &str = ".apsis-tmp";

/// On top of what the files need: the rebuilt initrd isn't the snapshot's byte for byte, and
/// FAT rounds each file up to its clusters.
const ESP_MARGIN: u64 = 16 << 20;

/// The files of the ESP that the boot refresh rewrites (PLAN 6b.6 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootFile {
    /// `EFI/Pop_OS-<root-uuid>/vmlinuz.efi`: the kernel the firmware starts.
    Kernel,
    /// `EFI/Pop_OS-<root-uuid>/initrd.img`.
    Initrd,
    /// `loader/entries/Pop_OS-current.conf`.
    CurrentEntry,
    /// `loader/entries/Pop_OS-oldkern.conf`. Not there on a machine with one kernel.
    OldkernEntry,
}

impl BootFile {
    pub const ALL: [Self; 4] = [
        Self::Kernel,
        Self::Initrd,
        Self::CurrentEntry,
        Self::OldkernEntry,
    ];

    /// Its file name: on the ESP, in the backup, and its key in the manifest.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Kernel => "vmlinuz.efi",
            Self::Initrd => "initrd.img",
            Self::CurrentEntry => "Pop_OS-current.conf",
            Self::OldkernEntry => "Pop_OS-oldkern.conf",
        }
    }

    /// Where it is, from the ESP's top folder.
    #[must_use]
    pub fn esp_path(self, root_uuid: &str) -> PathBuf {
        let folder = match self {
            Self::Kernel | Self::Initrd => format!("EFI/Pop_OS-{root_uuid}"),
            Self::CurrentEntry | Self::OldkernEntry => "loader/entries".to_owned(),
        };
        Path::new(&folder).join(self.name())
    }

    /// Whether a backup needs it. Only the oldkern entry may be missing.
    fn is_required(self) -> bool {
        self != Self::OldkernEntry
    }
}

/// Why the backup, its verification or the put-back failed.
#[derive(Debug, thiserror::Error)]
pub enum EspError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A boot file the backup needs isn't on the ESP.
    #[error("the ESP has no {0}")]
    Missing(&'static str),
    /// A copy isn't what it was copied from, or a backup file isn't what the manifest says.
    #[error("{0} doesn't match")]
    Mismatch(&'static str),
    /// Nothing to work with: a root UUID that isn't one, a link in place of a boot file, a
    /// manifest that's refused or is another installation's.
    #[error("{0}")]
    Invalid(String),
}

impl From<FileError> for EspError {
    fn from(error: FileError) -> Self {
        match error {
            FileError::Io(error) => Self::Io(error),
            FileError::Invalid(reason) => Self::Invalid(reason),
        }
    }
}

/// One backed-up file as the manifest records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// In bytes.
    pub size: u64,
    /// 64 lowercase hex digits.
    pub sha256: String,
}

/// `esp-backup/manifest.json`: what the backup holds. Format and rules: [`super::file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The root filesystem the ESP folder is named after.
    pub root_uuid: String,
    /// In [`BootFile::ALL`]'s order. `None`: it wasn't on the ESP.
    pub files: [Option<Entry>; 4],
}

impl Manifest {
    #[must_use]
    pub fn entry(&self, file: BootFile) -> Option<&Entry> {
        let index = BootFile::ALL.iter().position(|&other| other == file)?;
        self.files[index].as_ref()
    }

    /// # Errors
    ///
    /// [`FileError::Invalid`] if `text` isn't a manifest this Apsis wrote, whole and in range.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let keys = [
            "root_uuid",
            BootFile::Kernel.name(),
            BootFile::Initrd.name(),
            BootFile::CurrentEntry.name(),
            BootFile::OldkernEntry.name(),
        ];
        let map = file::object(text, &keys)?;
        let manifest = Self {
            root_uuid: file::text(&map, "root_uuid")?.to_owned(),
            files: [
                entry(&map, BootFile::Kernel)?,
                entry(&map, BootFile::Initrd)?,
                entry(&map, BootFile::CurrentEntry)?,
                entry(&map, BootFile::OldkernEntry)?,
            ],
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// The file's text.
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] if the manifest is one [`Manifest::parse`] would refuse.
    pub fn to_text(&self) -> Result<String, FileError> {
        self.validate()?;
        let files = BootFile::ALL
            .into_iter()
            .zip(&self.files)
            .map(|(file, entry)| {
                let value = entry.as_ref().map_or(
                    Value::Null,
                    |entry| json!({"size": entry.size, "sha256": entry.sha256}),
                );
                (file.name(), value)
            });
        Ok(file::to_text(
            [("root_uuid", Value::from(self.root_uuid.as_str()))]
                .into_iter()
                .chain(files),
        ))
    }

    /// Reads `state_dir/esp-backup/manifest.json`.
    ///
    /// # Errors
    ///
    /// [`FileError::Io`] if it's missing or unreadable, [`FileError::Invalid`] as for
    /// [`Manifest::parse`].
    pub fn load(state_dir: &Path) -> Result<Self, FileError> {
        Self::parse(&file::load(&state_dir.join(BACKUP_DIR), MANIFEST_FILE)?)
            .map_err(|error| error.within(MANIFEST_FILE))
    }

    fn save(&self, state_dir: &Path) -> Result<(), FileError> {
        file::save(&state_dir.join(BACKUP_DIR), MANIFEST_FILE, &self.to_text()?)
    }

    fn validate(&self) -> Result<(), FileError> {
        file::check_uuid("root_uuid", &self.root_uuid)?;
        for (file, entry) in BootFile::ALL.into_iter().zip(&self.files) {
            let name = file.name();
            match entry {
                None if file.is_required() => {
                    return Err(FileError::Invalid(format!("{name:?} isn't an object")));
                }
                Some(entry) if !is_sha256(&entry.sha256) => {
                    return Err(FileError::Invalid(format!(
                        "{name:?}: \"sha256\" isn't a SHA-256"
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn entry(map: &Map<String, Value>, file: BootFile) -> Result<Option<Entry>, FileError> {
    let name = file.name();
    let fields = match map.get(name) {
        Some(Value::Null) if !file.is_required() => return Ok(None),
        Some(Value::Object(fields)) => fields,
        _ if file.is_required() => {
            return Err(FileError::Invalid(format!("{name:?} isn't an object")));
        }
        _ => {
            return Err(FileError::Invalid(format!(
                "{name:?} isn't an object or null"
            )));
        }
    };
    (|| {
        file::fields(fields, &["size", "sha256"])?;
        Ok(Some(Entry {
            size: file::bytes(fields, "size")?,
            sha256: file::text(fields, "sha256")?.to_owned(),
        }))
    })()
    .map_err(|error: FileError| error.within(&format!("{name:?}")))
}

/// Copies the boot files from the ESP to `state_dir/esp-backup/` (PLAN 6b.6 step 4): each
/// flushed and compared byte for byte with the original, then the manifest with each file's
/// size and SHA-256, written last. A backup that fails leaves no folder behind.
///
/// # Errors
///
/// [`EspError::Io`] with `AlreadyExists` if there's a backup folder already (the caller
/// keeps or [`remove`]s it); [`EspError::Missing`] for a boot file that isn't there;
/// [`EspError::Mismatch`] for a copy that doesn't compare; [`EspError::Invalid`] for a
/// `root_uuid` that isn't one, or a link in place of a boot file.
pub fn back_up(esp: &Path, state_dir: &Path, root_uuid: &str) -> Result<Manifest, EspError> {
    check_root_uuid(root_uuid)?;
    let dir = state_dir.join(BACKUP_DIR);
    DirBuilder::new().mode(0o700).create(&dir)?;
    let made = (|| {
        let mut files = [None, None, None, None];
        for (file, slot) in BootFile::ALL.into_iter().zip(&mut files) {
            let name = file.name();
            let original = esp.join(file.esp_path(root_uuid));
            let copy = dir.join(name);
            let source = match open(&original) {
                Err(EspError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    if file.is_required() {
                        return Err(EspError::Missing(name));
                    }
                    continue;
                }
                other => other?,
            };
            let mut target = file::create_new(&copy)?;
            let entry = copy_hashed(source, &mut target)?;
            target.sync_all()?;
            if !same_bytes(&original, &copy)? {
                return Err(EspError::Mismatch(name));
            }
            *slot = Some(entry);
        }
        File::open(&dir)?.sync_all()?;
        let manifest = Manifest {
            root_uuid: root_uuid.to_owned(),
            files,
        };
        manifest.save(state_dir)?;
        File::open(state_dir)?.sync_all()?;
        Ok(manifest)
    })();
    if made.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    made
}

/// Checks the backup in `state_dir` against its manifest: each file's size and SHA-256.
///
/// # Errors
///
/// [`EspError::Io`] with `NotFound` if there's no manifest (no backup, or one a power cut
/// stopped); [`EspError::Invalid`] for a manifest that's refused; [`EspError::Mismatch`]
/// for a file that's missing, cut short or changed.
pub fn verify(state_dir: &Path) -> Result<Manifest, EspError> {
    let manifest = Manifest::load(state_dir)?;
    let dir = state_dir.join(BACKUP_DIR);
    for file in BootFile::ALL {
        let Some(entry) = manifest.entry(file) else {
            continue;
        };
        let name = file.name();
        let found = match open(&dir.join(name)) {
            Ok(source) => copy_hashed(source, &mut io::sink())?,
            Err(EspError::Io(error)) if error.kind() != io::ErrorKind::NotFound => {
                return Err(error.into());
            }
            Err(_) => return Err(EspError::Mismatch(name)),
        };
        if found != *entry {
            return Err(EspError::Mismatch(name));
        }
    }
    Ok(manifest)
}

/// Puts the backed-up files back on the ESP (PLAN 6b.6 step 6, "boot files failed"). The
/// whole backup is verified first, so a damaged file is never written. Then each file is
/// written to a temporary name in its folder, flushed, renamed over the ESP's, and compared
/// byte for byte with the backup. A file that wasn't backed up is left alone.
///
/// # Errors
///
/// As [`verify`], with the ESP untouched; [`EspError::Invalid`] if the backup is of another
/// root UUID, also untouched; [`EspError::Io`] or [`EspError::Mismatch`] for a file that
/// couldn't be written or doesn't compare afterwards (the ESP is then in an unknown state:
/// `boot-broken`).
pub fn put_back(esp: &Path, state_dir: &Path, root_uuid: &str) -> Result<(), EspError> {
    check_root_uuid(root_uuid)?;
    let manifest = verify(state_dir)?;
    if manifest.root_uuid != root_uuid {
        return Err(EspError::Invalid(
            "the ESP backup is of another installation".to_owned(),
        ));
    }
    let dir = state_dir.join(BACKUP_DIR);
    for file in BootFile::ALL {
        if manifest.entry(file).is_none() {
            continue;
        }
        let name = file.name();
        let backup = dir.join(name);
        let target = esp.join(file.esp_path(root_uuid));
        let folder = target.parent().unwrap_or(esp);
        let temp = folder.join(format!("{name}{TEMP_SUFFIX}"));
        // Left by a put-back that was cut short.
        match fs::remove_file(&temp) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let written = (|| {
            let mut out = file::create_new(&temp)?;
            copy_hashed(open(&backup)?, &mut out)?;
            out.sync_all()?;
            fs::rename(&temp, &target)?;
            File::open(folder)?.sync_all()?;
            Ok(())
        })();
        if let Err(error) = written {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        if !same_bytes(&target, &backup)? {
            return Err(EspError::Mismatch(name));
        }
    }
    Ok(())
}

/// Removes the backup (PLAN 6b.6 step 8). No backup is fine.
///
/// # Errors
///
/// Any I/O error but "not found".
pub fn remove(state_dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(state_dir.join(BACKUP_DIR)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// What the check after the boot refresh found wrong (PLAN 6b.6 step 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckFailure {
    /// `/boot/vmlinuz` isn't a link to a `vmlinuz-<version>`.
    NoKernelLink,
    /// The ESP's `vmlinuz.efi` isn't the file `/boot/vmlinuz` points to.
    KernelDiffers,
    /// The ESP's `initrd.img` isn't the file `/boot/initrd.img` points to.
    InitrdDiffers,
    /// There's no `/usr/lib/modules/<version>/` for the kernel the ESP now boots.
    NoModules { version: String },
    /// There's no `loader/entries/Pop_OS-current.conf`.
    NoEntry,
}

/// The check after the boot refresh: the ESP's kernel and initrd are, byte for byte, the
/// files `root`'s `/boot/vmlinuz` and `/boot/initrd.img` point to, that kernel's modules are
/// in `root`, and the current entry exists. Gives the kernel's version. That both commands
/// exited 0 is the caller's to check.
///
/// # Errors
///
/// The first thing that's wrong. A file that can't be read counts as one that differs.
pub fn check(esp: &Path, root: &Path, root_uuid: &str) -> Result<String, CheckFailure> {
    let boot = root.join("boot");
    // Only the link's last part is used, so the file is looked up in `root`'s own `boot`.
    let linked = |name: &str| {
        let target = fs::read_link(boot.join(name)).ok()?;
        let file = target.file_name()?.to_str()?.to_owned();
        let version = file.strip_prefix(name)?.strip_prefix('-')?;
        is_kernel_version(version).then(|| (boot.join(&file), version.to_owned()))
    };
    let same = |file: BootFile, with: &Path| {
        same_bytes(&esp.join(file.esp_path(root_uuid)), with).unwrap_or(false)
    };

    let (kernel, version) = linked("vmlinuz").ok_or(CheckFailure::NoKernelLink)?;
    if !same(BootFile::Kernel, &kernel) {
        return Err(CheckFailure::KernelDiffers);
    }
    if !linked("initrd.img").is_some_and(|(initrd, _)| same(BootFile::Initrd, &initrd)) {
        return Err(CheckFailure::InitrdDiffers);
    }
    let modules = root.join("usr/lib/modules").join(&version);
    if !fs::symlink_metadata(modules).is_ok_and(|meta| meta.is_dir()) {
        return Err(CheckFailure::NoModules { version });
    }
    let entry = esp.join(BootFile::CurrentEntry.esp_path(root_uuid));
    if !fs::symlink_metadata(entry).is_ok_and(|meta| meta.is_file()) {
        return Err(CheckFailure::NoEntry);
    }
    Ok(version)
}

/// The sizes of a kernel and its initrd, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSizes {
    pub kernel: u64,
    pub initrd: u64,
}

/// What the ESP must have free before a restore: `on_esp` is what it boots now, `restored`
/// the snapshot's `/boot/vmlinuz` and `/boot/initrd.img`. That's what each file grows by
/// when the boot refresh writes the restored one over it, room for a put-back's temporary
/// copy of the largest file, and a margin.
#[must_use]
pub fn esp_needs(on_esp: BootSizes, restored: BootSizes) -> u64 {
    restored
        .kernel
        .saturating_sub(on_esp.kernel)
        .saturating_add(restored.initrd.saturating_sub(on_esp.initrd))
        .saturating_add(on_esp.kernel.max(on_esp.initrd))
        .saturating_add(ESP_MARGIN)
}

/// # Errors
///
/// [`Refusal::BootSpace`] if the ESP has less than `needs` bytes free.
pub fn check_esp_space(needs: u64, free: u64) -> Result<(), Refusal> {
    if free < needs {
        return Err(Refusal::BootSpace { needs, free });
    }
    Ok(())
}

fn check_root_uuid(root_uuid: &str) -> Result<(), EspError> {
    if !is_plain_uuid(root_uuid) {
        return Err(EspError::Invalid(format!("not a root UUID: {root_uuid:?}")));
    }
    Ok(())
}

/// A regular file for reading, never through a link.
fn open(path: &Path) -> Result<File, EspError> {
    let not_regular = || EspError::Invalid(format!("not a regular file: {}", path.display()));
    let file = match file::open_nofollow(path) {
        Err(error) if error.raw_os_error() == Some(Errno::LOOP.raw_os_error()) => {
            return Err(not_regular());
        }
        other => other?,
    };
    if !file.metadata()?.is_file() {
        return Err(not_regular());
    }
    Ok(file)
}

/// Copies all of `from` to `to`, and gives the size and SHA-256 of what was read.
fn copy_hashed(mut from: File, to: &mut impl Write) -> io::Result<Entry> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = from.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        to.write_all(&buffer[..read])?;
        size += read as u64;
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(Entry { size, sha256 })
}

/// Whether two regular files hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> Result<bool, EspError> {
    let (mut a, mut b) = (open(a)?, open(b)?);
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    let mut left = vec![0; 64 * 1024];
    let mut right = vec![0; 64 * 1024];
    loop {
        let read = read_full(&mut a, &mut left)?;
        if read != read_full(&mut b, &mut right)? || left[..read] != right[..read] {
            return Ok(false);
        }
        if read == 0 {
            return Ok(true);
        }
    }
}

/// Reads until `buffer` is full or the file ends.
fn read_full(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::super::file::tests::temp_dir;
    use super::*;

    const UUID: &str = "11111111-2222-3333-4444-555555555555";
    const NEW: &str = "6.9.3-76060903-generic";
    const OLD: &str = "6.8.0-76060800-generic";

    /// A machine as apsis-test: the ESP boots copies of the kernel and initrd, and `/boot`
    /// links to the files they were copied from.
    struct Lab {
        esp: PathBuf,
        root: PathBuf,
        state: PathBuf,
    }

    impl Lab {
        fn esp_file(&self, file: BootFile) -> PathBuf {
            self.esp.join(file.esp_path(UUID))
        }

        fn backup_file(&self, file: BootFile) -> PathBuf {
            self.state.join(BACKUP_DIR).join(file.name())
        }

        /// A kernel in `/boot` and its modules, as a package installs them.
        fn install_kernel(&self, version: &str) {
            let boot = self.root.join("boot");
            fs::write(boot.join(format!("vmlinuz-{version}")), kernel(version)).unwrap();
            fs::write(boot.join(format!("initrd.img-{version}")), initrd(version)).unwrap();
            fs::create_dir_all(self.root.join("usr/lib/modules").join(version)).unwrap();
        }

        /// `/boot/vmlinuz` and `/boot/initrd.img` point to `version`.
        fn link_kernel(&self, version: &str) {
            let boot = self.root.join("boot");
            for name in ["vmlinuz", "initrd.img"] {
                let _ = fs::remove_file(boot.join(name));
                std::os::unix::fs::symlink(format!("{name}-{version}"), boot.join(name)).unwrap();
            }
        }

        /// What kernelstub does: the linked kernel and initrd copied to the ESP.
        fn kernelstub(&self, version: &str) {
            fs::write(self.esp_file(BootFile::Kernel), kernel(version)).unwrap();
            fs::write(self.esp_file(BootFile::Initrd), initrd(version)).unwrap();
            fs::write(self.esp_file(BootFile::CurrentEntry), entry(version)).unwrap();
        }
    }

    fn kernel(version: &str) -> String {
        format!("kernel {version}\n").repeat(4000)
    }

    fn initrd(version: &str) -> String {
        format!("initrd {version}\n").repeat(9000)
    }

    fn entry(version: &str) -> String {
        format!("title Pop!_OS\nlinux /EFI/Pop_OS-{UUID}/vmlinuz.efi\n# {version}\n")
    }

    /// Running `NEW`, booted from the ESP, with an oldkern entry.
    fn lab(name: &str) -> Lab {
        let dir = temp_dir(name);
        let lab = Lab {
            esp: dir.join("esp"),
            root: dir.join("root"),
            state: dir.join("root/var/lib/apsis/restore"),
        };
        fs::create_dir_all(lab.esp.join(format!("EFI/Pop_OS-{UUID}"))).unwrap();
        fs::create_dir_all(lab.esp.join("loader/entries")).unwrap();
        fs::create_dir_all(lab.root.join("boot")).unwrap();
        fs::create_dir_all(&lab.state).unwrap();
        lab.install_kernel(NEW);
        lab.link_kernel(NEW);
        lab.kernelstub(NEW);
        fs::write(lab.esp_file(BootFile::OldkernEntry), entry("oldkern")).unwrap();
        lab
    }

    fn read(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_boot_files_are_the_four_of_the_plan() {
        let paths: Vec<_> = BootFile::ALL
            .iter()
            .map(|file| file.esp_path(UUID).to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            paths,
            [
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/vmlinuz.efi",
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/initrd.img",
                "loader/entries/Pop_OS-current.conf",
                "loader/entries/Pop_OS-oldkern.conf",
            ]
        );
    }

    #[test]
    fn a_backup_copies_the_four_files_into_the_state_folder() {
        let lab = lab("esp-backup");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        for file in BootFile::ALL {
            assert_eq!(
                read(lab.backup_file(file)),
                read(lab.esp_file(file)),
                "{file:?}"
            );
        }
        assert!(lab.state.join("esp-backup/manifest.json").is_file());
    }

    #[test]
    fn the_manifest_records_each_files_size_and_sha256() {
        let lab = lab("esp-manifest");
        fs::write(lab.esp_file(BootFile::CurrentEntry), "abc").unwrap();
        let manifest = back_up(&lab.esp, &lab.state, UUID).unwrap();
        assert_eq!(manifest.root_uuid, UUID);
        assert_eq!(
            manifest.entry(BootFile::CurrentEntry),
            Some(&Entry {
                size: 3,
                sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                    .to_owned()
            })
        );
        assert_eq!(
            manifest.entry(BootFile::Kernel).unwrap().size,
            kernel(NEW).len() as u64
        );
        assert_eq!(Manifest::load(&lab.state).unwrap(), manifest);
    }

    #[test]
    fn the_manifest_is_written_field_by_field_and_reads_back() {
        let entry = |size| {
            Some(Entry {
                size,
                sha256: "ab".repeat(32),
            })
        };
        let manifest = Manifest {
            root_uuid: UUID.to_owned(),
            files: [entry(1), entry(2), entry(3), None],
        };
        let hash = "ab".repeat(32);
        assert_eq!(
            manifest.to_text().unwrap(),
            format!(
                r#"{{
  "version": 1,
  "root_uuid": "{UUID}",
  "vmlinuz.efi": {{
    "size": 1,
    "sha256": "{hash}"
  }},
  "initrd.img": {{
    "size": 2,
    "sha256": "{hash}"
  }},
  "Pop_OS-current.conf": {{
    "size": 3,
    "sha256": "{hash}"
  }},
  "Pop_OS-oldkern.conf": null
}}
"#
            )
        );
        assert_eq!(
            Manifest::parse(&manifest.to_text().unwrap()).unwrap(),
            manifest
        );
    }

    #[test]
    fn a_manifest_that_fails_validation_is_refused() {
        let entry = Some(Entry {
            size: 1,
            sha256: "ab".repeat(32),
        });
        let good = Manifest {
            root_uuid: UUID.to_owned(),
            files: [entry.clone(), entry.clone(), entry.clone(), None],
        };
        let text = good.to_text().unwrap();
        for (from, to, reason) in [
            (UUID, "../x", "\"root_uuid\" isn't a UUID"),
            (
                "\"version\": 1",
                "\"version\": 2",
                "version 2 (this Apsis reads 1)",
            ),
            (
                "\"size\": 1,",
                "\"size\": -1,",
                "\"vmlinuz.efi\": \"size\" isn't a whole number of bytes",
            ),
            (
                "\"Pop_OS-oldkern.conf\": null",
                "\"Pop_OS-oldkern.conf\": 1",
                "\"Pop_OS-oldkern.conf\" isn't an object or null",
            ),
            (
                "\"Pop_OS-oldkern.conf\": null",
                "\"Pop_OS-oldkern.conf\": null, \"grubx64.efi\": null",
                "unknown field \"grubx64.efi\"",
            ),
        ] {
            match Manifest::parse(&text.replacen(from, to, 1)) {
                Err(FileError::Invalid(found)) => assert_eq!(found, reason),
                other => panic!("{to}: {other:?}"),
            }
        }
        let no_kernel = Manifest {
            files: [None, entry.clone(), entry.clone(), None],
            ..good.clone()
        };
        assert!(matches!(no_kernel.to_text(), Err(FileError::Invalid(_))));
        let bad_hash = Manifest {
            files: [
                Some(Entry {
                    size: 1,
                    sha256: "AB".repeat(32),
                }),
                entry.clone(),
                entry,
                None,
            ],
            ..good
        };
        assert!(matches!(bad_hash.to_text(), Err(FileError::Invalid(_))));
    }

    /// A machine with one kernel has no oldkern entry.
    #[test]
    fn a_missing_oldkern_entry_is_recorded_as_none() {
        let lab = lab("esp-no-oldkern");
        fs::remove_file(lab.esp_file(BootFile::OldkernEntry)).unwrap();
        let manifest = back_up(&lab.esp, &lab.state, UUID).unwrap();
        assert_eq!(manifest.entry(BootFile::OldkernEntry), None);
        assert!(!lab.backup_file(BootFile::OldkernEntry).exists());
        verify(&lab.state).unwrap();
    }

    #[test]
    fn a_missing_kernel_fails_the_backup_and_leaves_none_behind() {
        let lab = lab("esp-no-kernel");
        fs::remove_file(lab.esp_file(BootFile::Initrd)).unwrap();
        match back_up(&lab.esp, &lab.state, UUID) {
            Err(EspError::Missing(name)) => assert_eq!(name, "initrd.img"),
            other => panic!("{other:?}"),
        }
        assert!(!lab.state.join(BACKUP_DIR).exists());
    }

    /// Whether an earlier backup is kept or taken again is the caller's (PLAN 6b.10): a
    /// backup is never made over one.
    #[test]
    fn a_backup_is_never_made_over_an_earlier_one() {
        let lab = lab("esp-twice");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.kernelstub(OLD);
        match back_up(&lab.esp, &lab.state, UUID) {
            Err(EspError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::AlreadyExists),
            other => panic!("{other:?}"),
        }
        assert_eq!(read(lab.backup_file(BootFile::Kernel)), kernel(NEW));
        remove(&lab.state).unwrap();
        assert!(!lab.state.join(BACKUP_DIR).exists());
        remove(&lab.state).unwrap();
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        assert_eq!(read(lab.backup_file(BootFile::Kernel)), kernel(OLD));
    }

    #[test]
    fn a_root_uuid_that_could_leave_the_esp_is_refused() {
        let lab = lab("esp-uuid");
        for uuid in ["", "../..", "a/b"] {
            assert!(matches!(
                back_up(&lab.esp, &lab.state, uuid),
                Err(EspError::Invalid(_))
            ));
        }
        assert!(!lab.state.join(BACKUP_DIR).exists());
    }

    #[test]
    fn a_link_in_place_of_a_boot_file_isnt_followed() {
        let lab = lab("esp-link");
        fs::remove_file(lab.esp_file(BootFile::Kernel)).unwrap();
        std::os::unix::fs::symlink(
            lab.root.join(format!("boot/vmlinuz-{NEW}")),
            lab.esp_file(BootFile::Kernel),
        )
        .unwrap();
        assert!(matches!(
            back_up(&lab.esp, &lab.state, UUID),
            Err(EspError::Invalid(_))
        ));
        assert!(!lab.state.join(BACKUP_DIR).exists());
    }

    #[test]
    fn files_are_compared_byte_for_byte() {
        let dir = temp_dir("esp-same");
        let write = |name: &str, text: &str| {
            fs::write(dir.join(name), text).unwrap();
            dir.join(name)
        };
        let long = "0123456789".repeat(20_000);
        let a = write("a", &long);
        let same = write("same", &long);
        let shorter = write("shorter", &long[..long.len() - 1]);
        let mut changed = long.clone();
        changed.replace_range(150_000..150_001, "x");
        let changed = write("changed", &changed);
        assert!(same_bytes(&a, &same).unwrap());
        assert!(!same_bytes(&a, &shorter).unwrap());
        assert!(!same_bytes(&a, &changed).unwrap());
        assert!(same_bytes(&a, &dir.join("missing")).is_err());
    }

    #[test]
    fn a_whole_backup_verifies_against_its_manifest() {
        let lab = lab("esp-verify");
        let manifest = back_up(&lab.esp, &lab.state, UUID).unwrap();
        assert_eq!(verify(&lab.state).unwrap(), manifest);
    }

    /// Same size, other bytes: only the hash tells.
    #[test]
    fn a_backup_file_that_changed_fails_the_verification() {
        let lab = lab("esp-verify-changed");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        let mut text = kernel(NEW);
        text.replace_range(0..1, "K");
        fs::write(lab.backup_file(BootFile::Kernel), text).unwrap();
        match verify(&lab.state) {
            Err(EspError::Mismatch(name)) => assert_eq!(name, "vmlinuz.efi"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_backup_file_cut_short_or_gone_fails_the_verification() {
        let lab = lab("esp-verify-short");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        fs::write(lab.backup_file(BootFile::Initrd), "initrd").unwrap();
        assert!(matches!(verify(&lab.state), Err(EspError::Mismatch(_))));
        fs::remove_file(lab.backup_file(BootFile::Initrd)).unwrap();
        assert!(matches!(verify(&lab.state), Err(EspError::Mismatch(_))));
    }

    /// A power cut during the backup: the manifest is written last, so there is none.
    #[test]
    fn a_backup_without_its_manifest_isnt_one() {
        let lab = lab("esp-verify-partial");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        fs::remove_file(lab.state.join(BACKUP_DIR).join(MANIFEST_FILE)).unwrap();
        match verify(&lab.state) {
            Err(EspError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            put_back(&lab.esp, &lab.state, UUID),
            Err(EspError::Io(_))
        ));
    }

    #[test]
    fn the_check_passes_when_the_esp_boots_what_boot_links_to() {
        let lab = lab("esp-check");
        assert_eq!(check(&lab.esp, &lab.root, UUID), Ok(NEW.to_owned()));
    }

    #[test]
    fn an_esp_kernel_or_initrd_that_isnt_boots_fails_the_check() {
        let lab = lab("esp-check-differs");
        fs::write(lab.esp_file(BootFile::Kernel), kernel(OLD)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::KernelDiffers)
        );
        lab.kernelstub(NEW);
        fs::write(lab.esp_file(BootFile::Initrd), "half an initrd").unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::InitrdDiffers)
        );
        fs::remove_file(lab.esp_file(BootFile::Initrd)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::InitrdDiffers)
        );
    }

    /// The ESP's kernel is the right one, but nothing would load its modules.
    #[test]
    fn a_kernel_without_its_modules_fails_the_check() {
        let lab = lab("esp-check-modules");
        fs::remove_dir(lab.root.join("usr/lib/modules").join(NEW)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::NoModules {
                version: NEW.to_owned()
            })
        );
    }

    #[test]
    fn a_missing_current_entry_fails_the_check() {
        let lab = lab("esp-check-entry");
        fs::remove_file(lab.esp_file(BootFile::CurrentEntry)).unwrap();
        assert_eq!(check(&lab.esp, &lab.root, UUID), Err(CheckFailure::NoEntry));
    }

    #[test]
    fn a_boot_without_a_usable_kernel_link_fails_the_check() {
        let lab = lab("esp-check-link");
        let link = lab.root.join("boot/vmlinuz");
        fs::remove_file(&link).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::NoKernelLink)
        );
        // A file, not a link.
        fs::write(&link, kernel(NEW)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::NoKernelLink)
        );
        // A link that names no kernel version.
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("../etc/passwd", &link).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::NoKernelLink)
        );
    }

    /// The link's folder part is never used: the kernel is looked up in the root's own
    /// `boot`, also for an absolute link.
    #[test]
    fn an_absolute_kernel_link_stays_inside_the_root() {
        let lab = lab("esp-check-absolute");
        let link = lab.root.join("boot/vmlinuz");
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(format!("/boot/vmlinuz-{NEW}"), &link).unwrap();
        assert_eq!(check(&lab.esp, &lab.root, UUID), Ok(NEW.to_owned()));
    }

    /// PLAN 6b.6 steps 4 to 6 for a snapshot from before a kernel update, with a boot
    /// refresh that goes wrong: the ESP gets back the kernel it booted, whose `/boot` files
    /// and modules the filter's rule 10 kept.
    #[test]
    fn a_failed_boot_refresh_is_put_back_to_the_kernel_that_was_running() {
        let lab = lab("esp-put-back");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        // The copy: the snapshot's older kernel arrives, the running one's files stay.
        lab.install_kernel(OLD);
        lab.link_kernel(OLD);
        // kernelstub wrote the entry and a broken kernel, then failed.
        fs::write(lab.esp_file(BootFile::Kernel), "half a kernel").unwrap();
        fs::write(lab.esp_file(BootFile::Initrd), initrd(OLD)).unwrap();
        fs::write(lab.esp_file(BootFile::CurrentEntry), entry(OLD)).unwrap();
        fs::remove_file(lab.esp_file(BootFile::OldkernEntry)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::KernelDiffers)
        );

        put_back(&lab.esp, &lab.state, UUID).unwrap();

        assert_eq!(read(lab.esp_file(BootFile::Kernel)), kernel(NEW));
        assert_eq!(read(lab.esp_file(BootFile::Initrd)), initrd(NEW));
        assert_eq!(read(lab.esp_file(BootFile::CurrentEntry)), entry(NEW));
        assert_eq!(read(lab.esp_file(BootFile::OldkernEntry)), entry("oldkern"));
        // What the ESP boots again is in `/boot` with its modules.
        assert!(
            same_bytes(
                &lab.esp_file(BootFile::Kernel),
                &lab.root.join(format!("boot/vmlinuz-{NEW}"))
            )
            .unwrap()
        );
        assert!(lab.root.join("usr/lib/modules").join(NEW).is_dir());
    }

    /// The same snapshot with a boot refresh that works: the ESP boots the snapshot's kernel.
    #[test]
    fn a_boot_refresh_to_the_snapshots_older_kernel_passes_the_check() {
        let lab = lab("esp-rollback");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.install_kernel(OLD);
        lab.link_kernel(OLD);
        lab.kernelstub(OLD);
        assert_eq!(check(&lab.esp, &lab.root, UUID), Ok(OLD.to_owned()));
    }

    #[test]
    fn putting_back_leaves_no_temporary_files_on_the_esp() {
        let lab = lab("esp-put-back-clean");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.kernelstub(OLD);
        // Left by a put-back that a power cut stopped.
        let leftover = lab
            .esp
            .join(format!("EFI/Pop_OS-{UUID}/vmlinuz.efi.apsis-tmp"));
        fs::write(&leftover, "half").unwrap();
        put_back(&lab.esp, &lab.state, UUID).unwrap();
        let mut names: Vec<_> = fs::read_dir(lab.esp.join(format!("EFI/Pop_OS-{UUID}")))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["initrd.img", "vmlinuz.efi"]);
    }

    /// An oldkern entry that wasn't there at the backup isn't Apsis's to remove.
    #[test]
    fn putting_back_leaves_alone_what_wasnt_backed_up() {
        let lab = lab("esp-put-back-extra");
        fs::remove_file(lab.esp_file(BootFile::OldkernEntry)).unwrap();
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        fs::write(lab.esp_file(BootFile::OldkernEntry), entry("new oldkern")).unwrap();
        put_back(&lab.esp, &lab.state, UUID).unwrap();
        assert_eq!(
            read(lab.esp_file(BootFile::OldkernEntry)),
            entry("new oldkern")
        );
    }

    /// A damaged kernel is never written to the ESP: the backup is verified first, and
    /// nothing on the ESP changes.
    #[test]
    fn a_damaged_backup_is_never_put_back() {
        let lab = lab("esp-put-back-damaged");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.kernelstub(OLD);
        let mut text = initrd(NEW);
        text.replace_range(10..11, "X");
        fs::write(lab.backup_file(BootFile::Initrd), text).unwrap();
        match put_back(&lab.esp, &lab.state, UUID) {
            Err(EspError::Mismatch(name)) => assert_eq!(name, "initrd.img"),
            other => panic!("{other:?}"),
        }
        assert_eq!(read(lab.esp_file(BootFile::Kernel)), kernel(OLD));
        assert_eq!(read(lab.esp_file(BootFile::Initrd)), initrd(OLD));
        assert_eq!(read(lab.esp_file(BootFile::CurrentEntry)), entry(OLD));
    }

    #[test]
    fn a_backup_of_another_installation_is_never_put_back() {
        let lab = lab("esp-put-back-other");
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.kernelstub(OLD);
        let other = "99999999-2222-3333-4444-555555555555";
        assert!(matches!(
            put_back(&lab.esp, &lab.state, other),
            Err(EspError::Invalid(_))
        ));
        assert_eq!(read(lab.esp_file(BootFile::Kernel)), kernel(OLD));
        assert!(!lab.esp.join(format!("EFI/Pop_OS-{other}")).exists());
    }

    const MIB: u64 = 1 << 20;

    fn sizes(kernel: u64, initrd: u64) -> BootSizes {
        BootSizes { kernel, initrd }
    }

    /// Growth of each file, room for the put-back's temporary copy of the largest one, and
    /// the margin.
    #[test]
    fn the_esp_needs_the_growth_a_put_back_copy_and_a_margin() {
        let on_esp = sizes(14 * MIB, 150 * MIB);
        // The same sizes: only the put-back copy and the margin.
        assert_eq!(esp_needs(on_esp, on_esp), (150 + 16) * MIB);
        // A larger kernel and initrd.
        assert_eq!(
            esp_needs(on_esp, sizes(15 * MIB, 170 * MIB)),
            (1 + 20 + 150 + 16) * MIB
        );
        // Smaller ones free nothing in advance.
        assert_eq!(
            esp_needs(on_esp, sizes(10 * MIB, 100 * MIB)),
            (150 + 16) * MIB
        );
        assert_eq!(esp_needs(sizes(u64::MAX, 0), sizes(0, u64::MAX)), u64::MAX);
    }

    #[test]
    fn a_short_esp_is_refused_with_both_numbers() {
        assert_eq!(
            check_esp_space(166 * MIB, 90 * MIB),
            Err(Refusal::BootSpace {
                needs: 166 * MIB,
                free: 90 * MIB
            })
        );
        assert_eq!(check_esp_space(166 * MIB, 166 * MIB), Ok(()));
    }
}
