// SPDX-License-Identifier: GPL-3.0-only

//! The boot files on the ESP (PLAN 6b.6, steps 4 and 6): their backup before the boot
//! refresh, the check after it, and putting them back when the check fails.
//!
//! kernelstub boots the kernel and initrd from copies on the ESP. After the copy, the boot
//! refresh (`update-initramfs`, `kernelstub`) rewrites them for the restored kernel. Before
//! that the files of [`SET`] are copied to `esp-backup/` in the state folder, on `/` and
//! protected from the restore. If the refreshed ESP doesn't check out, they're put back, and the ESP
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

/// The files of the ESP that the boot refresh rewrites (PLAN 6b.6 step 4), in [`SET`]'s
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootFile {
    /// The kernel the firmware starts.
    Kernel,
    Initrd,
    /// The kernel's command line, as kernelstub last wrote it.
    Cmdline,
    CurrentEntry,
    /// The kernel the oldkern entry starts.
    PreviousKernel,
    PreviousInitrd,
    OldkernEntry,
}

/// A folder of the ESP that holds boot files.
#[derive(Clone, Copy)]
enum Folder {
    /// `EFI/Pop_OS-<root-uuid>/`.
    Kernels,
    /// `loader/entries/`.
    Entries,
}

/// One row of [`SET`].
struct Row {
    file: BootFile,
    folder: Folder,
    /// On the ESP, in the backup, and its key in the manifest.
    name: &'static str,
    /// Whether a backup needs it. The others are there together or not at all.
    required: bool,
}

const fn row(file: BootFile, folder: Folder, name: &'static str, required: bool) -> Row {
    Row {
        file,
        folder,
        name,
        required,
    }
}

/// **The ESP's file set: the only files Apsis reads, backs up or writes there.** Names as
/// kernelstub writes them on Pop!_OS 24.04 (`vmlinuz-previous.efi`, but
/// `initrd.img-previous`). The previous pair and the oldkern entry aren't there on a machine
/// with one kernel installed.
///
/// Never here (PLAN 6b.6 step 4): `loader/entries/Recovery-*`, `loader/loader.conf`,
/// `loader/random-seed`, `loader/entries.srel`, `EFI/BOOT/`, `EFI/systemd/`.
const SET: [Row; BootFile::COUNT] = [
    row(BootFile::Kernel, Folder::Kernels, "vmlinuz.efi", true),
    row(BootFile::Initrd, Folder::Kernels, "initrd.img", true),
    row(BootFile::Cmdline, Folder::Kernels, "cmdline", true),
    row(
        BootFile::CurrentEntry,
        Folder::Entries,
        "Pop_OS-current.conf",
        true,
    ),
    row(
        BootFile::PreviousKernel,
        Folder::Kernels,
        "vmlinuz-previous.efi",
        false,
    ),
    row(
        BootFile::PreviousInitrd,
        Folder::Kernels,
        "initrd.img-previous",
        false,
    ),
    row(
        BootFile::OldkernEntry,
        Folder::Entries,
        "Pop_OS-oldkern.conf",
        false,
    ),
];

impl BootFile {
    pub const COUNT: usize = 7;

    /// Every file of [`SET`], in its order.
    pub const ALL: [Self; Self::COUNT] = {
        let mut all = [Self::Kernel; Self::COUNT];
        let mut index = 0;
        while index < Self::COUNT {
            all[index] = SET[index].file;
            index += 1;
        }
        all
    };

    fn row(self) -> &'static Row {
        &SET[self as usize]
    }

    /// Its file name: on the ESP, in the backup, and its key in the manifest.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.row().name
    }

    /// Where it is, from the ESP's top folder.
    #[must_use]
    pub fn esp_path(self, root_uuid: &str) -> PathBuf {
        let folder = match self.row().folder {
            Folder::Kernels => format!("EFI/Pop_OS-{root_uuid}"),
            Folder::Entries => "loader/entries".to_owned(),
        };
        Path::new(&folder).join(self.name())
    }

    /// Whether a backup needs it. The previous pair and the oldkern entry may be missing,
    /// all three together.
    fn is_required(self) -> bool {
        self.row().required
    }
}

/// Whether the optional files are there together or not at all: `found` is in
/// [`BootFile::ALL`]'s order.
fn previous_is_whole(found: [bool; BootFile::COUNT]) -> bool {
    let mut optional = BootFile::ALL
        .into_iter()
        .zip(found)
        .filter(|(file, _)| !file.is_required())
        .map(|(_, found)| found);
    let first = optional.next();
    optional.all(|found| Some(found) == first)
}

/// Why the backup, its verification or the put-back failed.
#[derive(Debug, thiserror::Error)]
pub enum EspError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A boot file the backup needs isn't on the ESP.
    #[error("the ESP has no {0}")]
    Missing(&'static str),
    /// Only some of the previous pair and the oldkern entry are on the ESP.
    #[error("the ESP has only part of the previous kernel's files")]
    PreviousIncomplete,
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
    pub files: [Option<Entry>; BootFile::COUNT],
}

impl Manifest {
    #[must_use]
    pub fn entry(&self, file: BootFile) -> Option<&Entry> {
        self.files[file as usize].as_ref()
    }

    /// # Errors
    ///
    /// [`FileError::Invalid`] if `text` isn't a manifest this Apsis wrote, whole and in range.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let keys: Vec<_> = ["root_uuid"]
            .into_iter()
            .chain(BootFile::ALL.map(BootFile::name))
            .collect();
        let map = file::object(text, &keys)?;
        let mut files = [const { None }; BootFile::COUNT];
        for (file, slot) in BootFile::ALL.into_iter().zip(&mut files) {
            *slot = entry(&map, file)?;
        }
        let manifest = Self {
            root_uuid: file::text(&map, "root_uuid")?.to_owned(),
            files,
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
        if !previous_is_whole(self.files.each_ref().map(Option::is_some)) {
            let names: Vec<_> = BootFile::ALL
                .into_iter()
                .filter(|file| !file.is_required())
                .map(|file| format!("{:?}", file.name()))
                .collect();
            return Err(FileError::Invalid(format!(
                "{} aren't all there or all null",
                names.join(", ")
            )));
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
        let mut files = [const { None }; BootFile::COUNT];
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
        if !previous_is_whole(files.each_ref().map(Option::is_some)) {
            return Err(EspError::PreviousIncomplete);
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

/// Removes a backup that a power cut stopped: the folder is there and has no manifest, which
/// is written last. Only a real folder at the backup's own name in `state_dir` is removed,
/// never through a link, and never one that has a manifest, readable or not. Says whether
/// it removed one.
///
/// Whether the ESP is still what it was, so that the backup may be taken once more, is the
/// caller's to know ([`super::apply`]: the boot refresh hasn't started).
///
/// # Errors
///
/// Any I/O error but "not found".
pub fn remove_partial(state_dir: &Path) -> io::Result<bool> {
    let dir = state_dir.join(BACKUP_DIR);
    // Asked of the name itself: a link there isn't a folder.
    if !fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir()) {
        return Ok(false);
    }
    match fs::symlink_metadata(dir.join(MANIFEST_FILE)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
        Ok(_) => return Ok(false),
    }
    fs::remove_dir_all(&dir)?;
    File::open(state_dir)?.sync_all()?;
    Ok(true)
}

/// Removes the temporary files a put-back leaves on the ESP when a power cut stops it:
/// `<name>.apsis-tmp` beside each boot file of [`SET`], and nothing else.
///
/// # Errors
///
/// Any I/O error but "not found"; `InvalidInput` for a `root_uuid` that isn't one.
pub fn clear_temporaries(esp: &Path, root_uuid: &str) -> io::Result<()> {
    if !is_plain_uuid(root_uuid) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    for file in BootFile::ALL {
        let path = esp.join(file.esp_path(root_uuid));
        let temp = path.with_file_name(format!("{}{TEMP_SUFFIX}", file.name()));
        match fs::remove_file(temp) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
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

/// What the check found wrong with a kernel pair on the ESP (PLAN 6b.6 step 6): the current
/// pair against `/boot/vmlinuz` and `/boot/initrd.img`, or the previous pair against
/// `/boot/vmlinuz.old` and `/boot/initrd.img.old`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckFailure {
    /// The pair's kernel link in `/boot` isn't a link to a `vmlinuz-<version>`.
    #[error("the kernel link in /boot doesn't name a kernel")]
    NoKernelLink,
    /// The ESP's kernel isn't the file the kernel link points to.
    #[error("the kernel on the ESP isn't the one /boot links to")]
    KernelDiffers,
    /// The ESP's initrd isn't the file the initrd link points to.
    #[error("the initrd on the ESP isn't the one /boot links to")]
    InitrdDiffers,
    /// There's no `/usr/lib/modules/<version>/` for the pair's kernel.
    #[error("there are no modules for kernel {version}")]
    NoModules { version: String },
    /// There's no `loader/entries/Pop_OS-current.conf`.
    #[error("the ESP has no Pop_OS-current.conf")]
    NoEntry,
    /// Only some of the previous pair and the oldkern entry are on the ESP.
    #[error("the ESP has only part of the previous kernel's files")]
    PreviousIncomplete,
}

/// The previous pair on the ESP, as [`check`] found it. Never a failure of the check: the
/// current pair is what boots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Previous {
    /// One kernel installed: no previous pair, no oldkern entry.
    Absent,
    /// It's what the `.old` links point to, and its modules are there.
    Good { version: String },
    /// To report (the journal, `result.json`'s message), not to refuse or put back for.
    Wrong(CheckFailure),
}

/// What a passed [`check`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The kernel the ESP boots.
    pub version: String,
    pub previous: Previous,
}

/// The ESP check: the ESP's kernel and initrd are, byte for byte, the files `root`'s
/// `/boot/vmlinuz` and `/boot/initrd.img` point to, that kernel's modules are in `root`, and
/// the current entry exists. If the previous pair is on the ESP, it's checked the same way
/// against `/boot/vmlinuz.old` and `/boot/initrd.img.old`, and what's wrong with it is
/// reported in [`Checked::previous`].
///
/// `root` is the tree the ESP is checked against (PLAN 6b.6): the live system before arming
/// ([`check_before_arming`]), and the restored tree after the boot refresh in the apply. In
/// the apply, that both commands exited 0 is the caller's to check.
///
/// # Errors
///
/// The first thing that's wrong with the current pair. A file that can't be read counts as
/// one that differs.
pub fn check(esp: &Path, root: &Path, root_uuid: &str) -> Result<Checked, CheckFailure> {
    let version = check_pair(esp, root, root_uuid, "", BootFile::Kernel, BootFile::Initrd)?;
    let on_esp = |file: BootFile| {
        fs::symlink_metadata(esp.join(file.esp_path(root_uuid))).is_ok_and(|meta| meta.is_file())
    };
    if !on_esp(BootFile::CurrentEntry) {
        return Err(CheckFailure::NoEntry);
    }
    let found = BootFile::ALL.map(on_esp);
    let previous = if !previous_is_whole(found) {
        Previous::Wrong(CheckFailure::PreviousIncomplete)
    } else if !found[BootFile::PreviousKernel as usize] {
        Previous::Absent
    } else {
        match check_pair(
            esp,
            root,
            root_uuid,
            ".old",
            BootFile::PreviousKernel,
            BootFile::PreviousInitrd,
        ) {
            Ok(version) => Previous::Good { version },
            Err(failure) => Previous::Wrong(failure),
        }
    };
    Ok(Checked { version, previous })
}

/// [`check`] against the live system, before arming (PLAN 6b.7): rule 10 keeps the kernel
/// the ESP boots, so the ESP must boot what `/boot` links to.
///
/// # Errors
///
/// [`Refusal::BootFiles`] for a current pair that fails the check, and for only part of the
/// previous pair and the oldkern entry: [`back_up`] would fail on that in the apply, after
/// the copy. A whole previous pair that isn't the `.old` links' never refuses.
pub fn check_before_arming(esp: &Path, root: &Path, root_uuid: &str) -> Result<Checked, Refusal> {
    let checked = check(esp, root, root_uuid).map_err(Refusal::BootFiles)?;
    if checked.previous == Previous::Wrong(CheckFailure::PreviousIncomplete) {
        return Err(Refusal::BootFiles(CheckFailure::PreviousIncomplete));
    }
    Ok(checked)
}

/// Whether the ESP's current pair is, byte for byte, `version`'s own `vmlinuz-<version>` and
/// `initrd.img-<version>` in `root`'s `/boot`, with its modules there. The `/boot` links
/// aren't read: after a restore they name the snapshot's kernel, and this asks about the one
/// the filter's rule 10 kept. It's what "the ESP boots the kernel from before" means, for
/// [`super::apply`].
#[must_use]
pub fn boots_kernel(esp: &Path, root: &Path, root_uuid: &str, version: &str) -> bool {
    if !is_kernel_version(version) {
        return false;
    }
    let same = |file: BootFile, name: &str| {
        let with = root.join("boot").join(format!("{name}-{version}"));
        same_bytes(&esp.join(file.esp_path(root_uuid)), &with).unwrap_or(false)
    };
    let modules = root.join("usr/lib/modules").join(version);
    same(BootFile::Kernel, "vmlinuz")
        && same(BootFile::Initrd, "initrd.img")
        && fs::symlink_metadata(modules).is_ok_and(|meta| meta.is_dir())
}

/// One kernel pair of the ESP against `root`'s `/boot/vmlinuz<link_suffix>` and
/// `/boot/initrd.img<link_suffix>`, and its modules. Gives the kernel's version.
fn check_pair(
    esp: &Path,
    root: &Path,
    root_uuid: &str,
    link_suffix: &str,
    kernel: BootFile,
    initrd: BootFile,
) -> Result<String, CheckFailure> {
    let boot = root.join("boot");
    // Only the link's last part is used, so the file is looked up in `root`'s own `boot`.
    let linked = |name: &str| {
        let target = fs::read_link(boot.join(format!("{name}{link_suffix}"))).ok()?;
        let file = target.file_name()?.to_str()?.to_owned();
        let version = file.strip_prefix(name)?.strip_prefix('-')?;
        is_kernel_version(version).then(|| (boot.join(&file), version.to_owned()))
    };
    let same = |file: BootFile, with: &Path| {
        same_bytes(&esp.join(file.esp_path(root_uuid)), with).unwrap_or(false)
    };

    let (kernel_file, version) = linked("vmlinuz").ok_or(CheckFailure::NoKernelLink)?;
    if !same(kernel, &kernel_file) {
        return Err(CheckFailure::KernelDiffers);
    }
    if !linked("initrd.img").is_some_and(|(initrd_file, _)| same(initrd, &initrd_file)) {
        return Err(CheckFailure::InitrdDiffers);
    }
    let modules = root.join("usr/lib/modules").join(&version);
    if !fs::symlink_metadata(modules).is_ok_and(|meta| meta.is_dir()) {
        return Err(CheckFailure::NoModules { version });
    }
    Ok(version)
}

/// The sizes of a kernel and its initrd, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSizes {
    pub kernel: u64,
    pub initrd: u64,
}

/// The sizes of the kernel pairs: on the ESP, or the ones a boot refresh would write there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EspSizes {
    pub current: BootSizes,
    /// `None`: one kernel installed.
    pub previous: Option<BootSizes>,
}

/// What the ESP must have free before a restore: `on_esp` is what's there now, `restored`
/// the snapshot's `/boot/vmlinuz` and `/boot/initrd.img` (current) and its `.old` links
/// (previous). That's what each of the four files grows by when the boot refresh writes the
/// restored one over it, room for a put-back's temporary copy of the largest file, and a
/// margin.
///
/// Nothing of this is kept in `request.json`: the needs and the free space (a `statvfs` of
/// the ESP) are both read live, when preparing and again at "Restart now" (PLAN 6b.4).
#[must_use]
pub fn esp_needs(on_esp: EspSizes, restored: EspSizes) -> u64 {
    let none = BootSizes {
        kernel: 0,
        initrd: 0,
    };
    let pairs = [
        (on_esp.current, restored.current),
        (
            on_esp.previous.unwrap_or(none),
            restored.previous.unwrap_or(none),
        ),
    ];
    let growth = pairs.iter().fold(0_u64, |sum, (old, new)| {
        sum.saturating_add(new.kernel.saturating_sub(old.kernel))
            .saturating_add(new.initrd.saturating_sub(old.initrd))
    });
    let largest = pairs
        .iter()
        .map(|(old, _)| old.kernel.max(old.initrd))
        .max()
        .unwrap_or(0);
    growth.saturating_add(largest).saturating_add(ESP_MARGIN)
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
pub(in crate::restore) mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::super::file::tests::temp_dir;
    use super::*;

    pub(in crate::restore) const UUID: &str = "11111111-2222-3333-4444-555555555555";
    pub(in crate::restore) const NEW: &str = "6.9.3-76060903-generic";
    pub(in crate::restore) const OLD: &str = "6.8.0-76060800-generic";
    pub(in crate::restore) const OLDER: &str = "6.6.10-76060610-generic";

    /// The previous kernel's two files and its entry: there together or not at all.
    const PREVIOUS: [BootFile; 3] = [
        BootFile::PreviousKernel,
        BootFile::PreviousInitrd,
        BootFile::OldkernEntry,
    ];

    /// What else a Pop!_OS ESP holds. Never backed up, never written.
    const BYSTANDERS: [&str; 7] = [
        "EFI/BOOT/BOOTX64.EFI",
        "EFI/systemd/systemd-bootx64.efi",
        "EFI/Recovery-ABCD-1234/vmlinuz.efi",
        "loader/entries/Recovery-ABCD-1234.conf",
        "loader/loader.conf",
        "loader/random-seed",
        "loader/entries.srel",
    ];

    /// What kernel-install leaves on a Pop!_OS ESP (apsis-test, check 0.1): a folder per
    /// kernel under the machine id (a placeholder here) and `EFI/Linux`, all empty. Never
    /// read, backed up or written, and never an error.
    const KERNEL_INSTALL: [&str; 3] = [
        "0123456789abcdef0123456789abcdef/6.9.3-76060903-generic",
        "0123456789abcdef0123456789abcdef/6.8.0-76060800-generic",
        "EFI/Linux",
    ];

    /// A machine as apsis-test: the ESP boots copies of the kernel and initrd, and `/boot`
    /// links to the files they were copied from.
    pub(in crate::restore) struct Lab {
        pub esp: PathBuf,
        pub root: PathBuf,
        pub state: PathBuf,
    }

    impl Lab {
        pub fn esp_file(&self, file: BootFile) -> PathBuf {
            self.esp.join(file.esp_path(UUID))
        }

        fn backup_file(&self, file: BootFile) -> PathBuf {
            self.state.join(BACKUP_DIR).join(file.name())
        }

        /// A kernel in `/boot` and its modules, as a package installs them.
        pub fn install_kernel(&self, version: &str) {
            let boot = self.root.join("boot");
            fs::write(boot.join(format!("vmlinuz-{version}")), kernel(version)).unwrap();
            fs::write(boot.join(format!("initrd.img-{version}")), initrd(version)).unwrap();
            fs::create_dir_all(self.root.join("usr/lib/modules").join(version)).unwrap();
        }

        /// `/boot/vmlinuz<suffix>` and `/boot/initrd.img<suffix>` point to `version`.
        fn link(&self, suffix: &str, version: &str) {
            let boot = self.root.join("boot");
            for name in ["vmlinuz", "initrd.img"] {
                let link = boot.join(format!("{name}{suffix}"));
                let _ = fs::remove_file(&link);
                std::os::unix::fs::symlink(format!("{name}-{version}"), link).unwrap();
            }
        }

        /// `/boot/vmlinuz` and `/boot/initrd.img` point to `version`.
        pub fn link_kernel(&self, version: &str) {
            self.link("", version);
        }

        /// `/boot/vmlinuz.old` and `/boot/initrd.img.old` point to `version`.
        pub fn link_previous(&self, version: &str) {
            self.link(".old", version);
        }

        /// What kernelstub does: the linked kernel and initrd copied to the ESP.
        pub fn kernelstub(&self, version: &str) {
            fs::write(self.esp_file(BootFile::Kernel), kernel(version)).unwrap();
            fs::write(self.esp_file(BootFile::Initrd), initrd(version)).unwrap();
            fs::write(self.esp_file(BootFile::Cmdline), cmdline(version)).unwrap();
            fs::write(self.esp_file(BootFile::CurrentEntry), entry(version)).unwrap();
        }

        /// And for the `.old` links: the previous pair and the oldkern entry.
        pub fn kernelstub_previous(&self, version: &str) {
            fs::write(self.esp_file(BootFile::PreviousKernel), kernel(version)).unwrap();
            fs::write(self.esp_file(BootFile::PreviousInitrd), initrd(version)).unwrap();
            fs::write(self.esp_file(BootFile::OldkernEntry), entry(version)).unwrap();
        }

        /// A machine with one kernel installed: no previous pair, no oldkern entry.
        pub fn one_kernel(&self) {
            for file in PREVIOUS {
                fs::remove_file(self.esp_file(file)).unwrap();
            }
        }

        /// Every file on the ESP, from its top folder, with what it holds.
        pub fn esp_tree(&self) -> Vec<(String, String)> {
            fn walk(top: &Path, dir: &Path, found: &mut Vec<(String, String)>) {
                for entry in fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(top, &path, found);
                    } else {
                        let name = path.strip_prefix(top).unwrap().to_str().unwrap();
                        found.push((name.to_owned(), fs::read_to_string(&path).unwrap()));
                    }
                }
            }
            let mut found = Vec::new();
            walk(&self.esp, &self.esp, &mut found);
            found.sort();
            found
        }
    }

    pub(in crate::restore) fn cmdline(version: &str) -> String {
        format!("root=UUID={UUID} ro quiet splash # {version}\n")
    }

    pub(in crate::restore) fn kernel(version: &str) -> String {
        format!("kernel {version}\n").repeat(4000)
    }

    pub(in crate::restore) fn initrd(version: &str) -> String {
        format!("initrd {version}\n").repeat(9000)
    }

    pub(in crate::restore) fn entry(version: &str) -> String {
        format!("title Pop!_OS\nlinux /EFI/Pop_OS-{UUID}/vmlinuz.efi\n# {version}\n")
    }

    /// Running `NEW`, booted from the ESP, with `OLD` as the previous kernel.
    pub(in crate::restore) fn lab(name: &str) -> Lab {
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
        lab.install_kernel(OLD);
        lab.link_kernel(NEW);
        lab.link_previous(OLD);
        lab.kernelstub(NEW);
        lab.kernelstub_previous(OLD);
        for path in BYSTANDERS {
            let path = lab.esp.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("not ours: {}\n", path.display())).unwrap();
        }
        for path in KERNEL_INSTALL {
            fs::create_dir_all(lab.esp.join(path)).unwrap();
        }
        lab
    }

    fn checked(version: &str, previous: Previous) -> Result<Checked, CheckFailure> {
        Ok(Checked {
            version: version.to_owned(),
            previous,
        })
    }

    fn good(version: &str) -> Previous {
        Previous::Good {
            version: version.to_owned(),
        }
    }

    fn read(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_boot_files_are_the_seven_of_the_plan() {
        let paths: Vec<_> = BootFile::ALL
            .iter()
            .map(|file| file.esp_path(UUID).to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            paths,
            [
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/vmlinuz.efi",
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/initrd.img",
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/cmdline",
                "loader/entries/Pop_OS-current.conf",
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/vmlinuz-previous.efi",
                "EFI/Pop_OS-11111111-2222-3333-4444-555555555555/initrd.img-previous",
                "loader/entries/Pop_OS-oldkern.conf",
            ]
        );
    }

    /// The list is one table, looked up by the file: each row is its own file's.
    #[test]
    fn each_boot_file_is_its_own_row_of_the_list() {
        assert_eq!(BootFile::ALL.len(), BootFile::COUNT);
        for (index, file) in BootFile::ALL.into_iter().enumerate() {
            assert_eq!(file as usize, index, "{file:?}");
        }
        let optional: Vec<_> = BootFile::ALL
            .into_iter()
            .filter(|file| !file.is_required())
            .collect();
        assert_eq!(optional, PREVIOUS);
    }

    /// PLAN 6b.6 step 4: the recovery entry and its folder, the loader's own files and the
    /// other loaders are never in the list.
    #[test]
    fn nothing_else_on_the_esp_is_in_the_list() {
        for file in BootFile::ALL {
            let path = file.esp_path(UUID);
            let path = path.to_str().unwrap();
            assert!(
                path.starts_with(&format!("EFI/Pop_OS-{UUID}/"))
                    || path.starts_with("loader/entries/Pop_OS-"),
                "{path}"
            );
            assert!(!BYSTANDERS.contains(&path), "{path}");
        }
    }

    #[test]
    fn a_backup_copies_the_seven_files_into_the_state_folder() {
        let lab = lab("esp-backup");
        let before = lab.esp_tree();
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        for file in BootFile::ALL {
            assert_eq!(
                read(lab.backup_file(file)),
                read(lab.esp_file(file)),
                "{file:?}"
            );
        }
        let mut names: Vec<_> = fs::read_dir(lab.state.join(BACKUP_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Pop_OS-current.conf",
                "Pop_OS-oldkern.conf",
                "cmdline",
                "initrd.img",
                "initrd.img-previous",
                "manifest.json",
                "vmlinuz-previous.efi",
                "vmlinuz.efi",
            ]
        );
        assert_eq!(lab.esp_tree(), before);
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
            files: [entry(1), entry(2), entry(3), entry(4), None, None, None],
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
  "cmdline": {{
    "size": 3,
    "sha256": "{hash}"
  }},
  "Pop_OS-current.conf": {{
    "size": 4,
    "sha256": "{hash}"
  }},
  "vmlinuz-previous.efi": null,
  "initrd.img-previous": null,
  "Pop_OS-oldkern.conf": null
}}
"#
            )
        );
        assert_eq!(
            Manifest::parse(&manifest.to_text().unwrap()).unwrap(),
            manifest
        );
        let two_kernels = Manifest {
            files: [1, 2, 3, 4, 5, 6, 7].map(entry),
            ..manifest
        };
        assert_eq!(
            Manifest::parse(&two_kernels.to_text().unwrap()).unwrap(),
            two_kernels
        );
    }

    /// The previous pair and the oldkern entry come and go together.
    #[test]
    fn a_manifest_with_part_of_the_previous_kernel_is_refused() {
        let entry = Some(Entry {
            size: 1,
            sha256: "ab".repeat(32),
        });
        let some = || entry.clone();
        for previous in [
            [some(), None, None],
            [some(), some(), None],
            [None, None, some()],
        ] {
            let [kernel, initrd, oldkern] = previous;
            let manifest = Manifest {
                root_uuid: UUID.to_owned(),
                files: [some(), some(), some(), some(), kernel, initrd, oldkern],
            };
            match manifest.to_text() {
                Err(FileError::Invalid(reason)) => assert_eq!(
                    reason,
                    "\"vmlinuz-previous.efi\", \"initrd.img-previous\", \
                     \"Pop_OS-oldkern.conf\" aren't all there or all null"
                ),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_manifest_that_fails_validation_is_refused() {
        let entry = Some(Entry {
            size: 1,
            sha256: "ab".repeat(32),
        });
        let some = || entry.clone();
        let good = Manifest {
            root_uuid: UUID.to_owned(),
            files: [some(), some(), some(), some(), None, None, None],
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
        for missing in [0, 2] {
            let mut files = good.files.clone();
            files[missing] = None;
            let manifest = Manifest {
                files,
                ..good.clone()
            };
            assert!(matches!(manifest.to_text(), Err(FileError::Invalid(_))));
        }
        let mut files = good.files.clone();
        files[0] = Some(Entry {
            size: 1,
            sha256: "AB".repeat(32),
        });
        let bad_hash = Manifest { files, ..good };
        assert!(matches!(bad_hash.to_text(), Err(FileError::Invalid(_))));
    }

    /// A machine with one kernel has no previous pair and no oldkern entry.
    #[test]
    fn a_machine_with_one_kernel_is_backed_up_without_the_previous_files() {
        let lab = lab("esp-one-kernel");
        lab.one_kernel();
        let manifest = back_up(&lab.esp, &lab.state, UUID).unwrap();
        for file in PREVIOUS {
            assert_eq!(manifest.entry(file), None, "{file:?}");
            assert!(!lab.backup_file(file).exists(), "{file:?}");
        }
        verify(&lab.state).unwrap();
    }

    /// The three are optional together: an ESP with only some of them isn't one kernelstub
    /// left, and a put-back couldn't make the oldkern entry whole.
    #[test]
    fn part_of_the_previous_kernel_fails_the_backup_and_leaves_none_behind() {
        for (index, gone) in PREVIOUS.into_iter().enumerate() {
            let lab = lab(&format!("esp-part-previous-{index}"));
            fs::remove_file(lab.esp_file(gone)).unwrap();
            assert!(
                matches!(
                    back_up(&lab.esp, &lab.state, UUID),
                    Err(EspError::PreviousIncomplete)
                ),
                "{gone:?}"
            );
            assert!(!lab.state.join(BACKUP_DIR).exists());
        }
    }

    #[test]
    fn a_missing_required_file_fails_the_backup_and_leaves_none_behind() {
        for (index, gone) in [
            BootFile::Kernel,
            BootFile::Initrd,
            BootFile::Cmdline,
            BootFile::CurrentEntry,
        ]
        .into_iter()
        .enumerate()
        {
            let lab = lab(&format!("esp-no-required-{index}"));
            fs::remove_file(lab.esp_file(gone)).unwrap();
            match back_up(&lab.esp, &lab.state, UUID) {
                Err(EspError::Missing(name)) => assert_eq!(name, gone.name()),
                other => panic!("{other:?}"),
            }
            assert!(!lab.state.join(BACKUP_DIR).exists());
        }
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
        assert_eq!(check(&lab.esp, &lab.root, UUID), checked(NEW, good(OLD)));
    }

    /// A power cut during the backup leaves a folder with no manifest. Only that is removed:
    /// a real folder at the backup's own name, never through a link, never a whole backup.
    #[test]
    fn only_a_backup_folder_without_a_manifest_is_removed_as_partial() {
        let lab = lab("esp-partial");
        // No folder.
        assert!(!remove_partial(&lab.state).unwrap());
        // A whole backup.
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        assert!(!remove_partial(&lab.state).unwrap());
        verify(&lab.state).unwrap();
        // A partial one.
        fs::remove_file(lab.state.join(BACKUP_DIR).join(MANIFEST_FILE)).unwrap();
        assert!(remove_partial(&lab.state).unwrap());
        assert!(!lab.state.join(BACKUP_DIR).exists());
        // A link at the name, to a folder without a manifest.
        let elsewhere = lab.root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::write(elsewhere.join("vmlinuz.efi"), "not ours").unwrap();
        std::os::unix::fs::symlink(&elsewhere, lab.state.join(BACKUP_DIR)).unwrap();
        assert!(!remove_partial(&lab.state).unwrap());
        assert!(elsewhere.join("vmlinuz.efi").is_file());
        assert!(fs::symlink_metadata(lab.state.join(BACKUP_DIR)).is_ok());
        // A manifest that's there but not readable as one isn't "no manifest".
        fs::remove_file(lab.state.join(BACKUP_DIR)).unwrap();
        fs::create_dir(lab.state.join(BACKUP_DIR)).unwrap();
        fs::write(lab.state.join(BACKUP_DIR).join(MANIFEST_FILE), "{").unwrap();
        assert!(!remove_partial(&lab.state).unwrap());
    }

    /// A put-back that a power cut stopped leaves its temporary file on the ESP.
    #[test]
    fn temporary_files_of_a_put_back_are_cleared_and_nothing_else() {
        let lab = lab("esp-clear-tmp");
        let before = lab.esp_tree();
        clear_temporaries(&lab.esp, UUID).unwrap();
        assert_eq!(lab.esp_tree(), before);
        let folder = lab.esp.join(format!("EFI/Pop_OS-{UUID}"));
        fs::write(folder.join("initrd.img.apsis-tmp"), "half").unwrap();
        fs::write(
            lab.esp.join("loader/entries/Pop_OS-current.conf.apsis-tmp"),
            "h",
        )
        .unwrap();
        // Not a boot file's temporary name: not Apsis's.
        fs::write(lab.esp.join("loader/loader.conf.apsis-tmp"), "theirs").unwrap();
        clear_temporaries(&lab.esp, UUID).unwrap();
        let mut expected = before;
        expected.push((
            "loader/loader.conf.apsis-tmp".to_owned(),
            "theirs".to_owned(),
        ));
        expected.sort();
        assert_eq!(lab.esp_tree(), expected);
    }

    /// What a put-back claims, and what tells an untouched ESP from a refreshed one.
    #[test]
    fn the_esp_boots_the_kernel_whose_boot_files_it_holds() {
        let lab = lab("esp-boots");
        assert!(boots_kernel(&lab.esp, &lab.root, UUID, NEW));
        assert!(!boots_kernel(&lab.esp, &lab.root, UUID, OLD));
        lab.kernelstub(OLD);
        assert!(boots_kernel(&lab.esp, &lab.root, UUID, OLD));
        assert!(!boots_kernel(&lab.esp, &lab.root, UUID, NEW));
        // The links don't matter: it's asked of one version's own files.
        lab.link_kernel(OLDER);
        assert!(boots_kernel(&lab.esp, &lab.root, UUID, OLD));
        fs::write(lab.esp_file(BootFile::Initrd), initrd(NEW)).unwrap();
        assert!(!boots_kernel(&lab.esp, &lab.root, UUID, OLD));
        lab.kernelstub(OLD);
        fs::remove_dir(lab.root.join("usr/lib/modules").join(OLD)).unwrap();
        assert!(!boots_kernel(&lab.esp, &lab.root, UUID, OLD));
        assert!(!boots_kernel(&lab.esp, &lab.root, UUID, "../x"));
    }

    #[test]
    fn a_check_failure_says_what_is_wrong() {
        assert_eq!(
            CheckFailure::NoModules {
                version: NEW.to_owned()
            }
            .to_string(),
            format!("there are no modules for kernel {NEW}")
        );
        assert_eq!(
            CheckFailure::KernelDiffers.to_string(),
            "the kernel on the ESP isn't the one /boot links to"
        );
    }

    #[test]
    fn the_check_passes_on_a_machine_with_one_kernel() {
        let lab = lab("esp-check-one-kernel");
        lab.one_kernel();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            checked(NEW, Previous::Absent)
        );
    }

    /// The previous pair is checked like the current one, against the `.old` links. What's
    /// wrong with it is reported, and the check still passes: the current pair boots.
    #[test]
    fn a_previous_pair_that_isnt_the_old_links_is_reported_not_failed() {
        let lab = lab("esp-check-previous");
        let wrong = |failure| checked(NEW, Previous::Wrong(failure));

        fs::write(lab.esp_file(BootFile::PreviousKernel), kernel(OLDER)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            wrong(CheckFailure::KernelDiffers)
        );
        lab.kernelstub_previous(OLD);
        fs::write(lab.esp_file(BootFile::PreviousInitrd), initrd(OLDER)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            wrong(CheckFailure::InitrdDiffers)
        );
        lab.kernelstub_previous(OLD);

        fs::remove_dir(lab.root.join("usr/lib/modules").join(OLD)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            wrong(CheckFailure::NoModules {
                version: OLD.to_owned()
            })
        );
        lab.install_kernel(OLD);

        fs::remove_file(lab.root.join("boot/vmlinuz.old")).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            wrong(CheckFailure::NoKernelLink)
        );
        lab.link_previous(OLD);
        assert_eq!(check(&lab.esp, &lab.root, UUID), checked(NEW, good(OLD)));
    }

    #[test]
    fn part_of_the_previous_kernel_is_reported_not_failed() {
        for (index, gone) in PREVIOUS.into_iter().enumerate() {
            let lab = lab(&format!("esp-check-part-previous-{index}"));
            fs::remove_file(lab.esp_file(gone)).unwrap();
            assert_eq!(
                check(&lab.esp, &lab.root, UUID),
                checked(NEW, Previous::Wrong(CheckFailure::PreviousIncomplete)),
                "{gone:?}"
            );
        }
    }

    /// PLAN 6b.7: before arming, the same check runs against the live system. A current pair
    /// that isn't `/boot`'s refuses; a previous pair that isn't doesn't.
    #[test]
    fn before_arming_a_previous_pair_that_differs_doesnt_refuse() {
        let lab = lab("esp-check-live");
        fs::write(lab.esp_file(BootFile::PreviousKernel), kernel(OLDER)).unwrap();
        assert_eq!(
            check_before_arming(&lab.esp, &lab.root, UUID),
            Ok(Checked {
                version: NEW.to_owned(),
                previous: Previous::Wrong(CheckFailure::KernelDiffers)
            })
        );
        fs::write(lab.esp_file(BootFile::Initrd), initrd(OLD)).unwrap();
        assert_eq!(
            check_before_arming(&lab.esp, &lab.root, UUID),
            Err(Refusal::BootFiles(CheckFailure::InitrdDiffers))
        );
    }

    /// Only part of the optional three would fail the backup in the apply, after the copy.
    /// So it refuses before arming.
    #[test]
    fn part_of_the_previous_kernel_is_refused_before_arming() {
        for (index, gone) in PREVIOUS.into_iter().enumerate() {
            let lab = lab(&format!("esp-arm-part-previous-{index}"));
            fs::remove_file(lab.esp_file(gone)).unwrap();
            assert_eq!(
                check_before_arming(&lab.esp, &lab.root, UUID),
                Err(Refusal::BootFiles(CheckFailure::PreviousIncomplete)),
                "{gone:?}"
            );
        }
    }

    #[test]
    fn the_whole_previous_kernel_or_none_passes_before_arming() {
        let lab = lab("esp-arm-previous");
        assert_eq!(
            check_before_arming(&lab.esp, &lab.root, UUID),
            Ok(Checked {
                version: NEW.to_owned(),
                previous: good(OLD)
            })
        );
        lab.one_kernel();
        assert_eq!(
            check_before_arming(&lab.esp, &lab.root, UUID),
            Ok(Checked {
                version: NEW.to_owned(),
                previous: Previous::Absent
            })
        );
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
        assert_eq!(check(&lab.esp, &lab.root, UUID), checked(NEW, good(OLD)));
    }

    /// PLAN 6b.6 steps 4 to 6 for a snapshot from before a kernel update, with a boot
    /// refresh that goes wrong: the ESP gets back the kernel it booted, whose `/boot` files
    /// and modules the filter's rule 10 kept.
    #[test]
    fn a_failed_boot_refresh_is_put_back_to_the_kernel_that_was_running() {
        let lab = lab("esp-put-back");
        let before = lab.esp_tree();
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        // The copy: the snapshot's older kernels arrive, the running one's files stay.
        lab.install_kernel(OLDER);
        lab.link_kernel(OLD);
        lab.link_previous(OLDER);
        // kernelstub moved the previous pair on, wrote the entry and a broken kernel, then
        // failed.
        lab.kernelstub_previous(OLDER);
        fs::write(lab.esp_file(BootFile::Kernel), "half a kernel").unwrap();
        fs::write(lab.esp_file(BootFile::Initrd), initrd(OLD)).unwrap();
        fs::write(lab.esp_file(BootFile::Cmdline), cmdline(OLD)).unwrap();
        fs::write(lab.esp_file(BootFile::CurrentEntry), entry(OLD)).unwrap();
        fs::remove_file(lab.esp_file(BootFile::OldkernEntry)).unwrap();
        assert_eq!(
            check(&lab.esp, &lab.root, UUID),
            Err(CheckFailure::KernelDiffers)
        );

        put_back(&lab.esp, &lab.state, UUID).unwrap();

        assert_eq!(read(lab.esp_file(BootFile::Kernel)), kernel(NEW));
        assert_eq!(read(lab.esp_file(BootFile::Initrd)), initrd(NEW));
        assert_eq!(read(lab.esp_file(BootFile::Cmdline)), cmdline(NEW));
        assert_eq!(read(lab.esp_file(BootFile::CurrentEntry)), entry(NEW));
        assert_eq!(read(lab.esp_file(BootFile::PreviousKernel)), kernel(OLD));
        assert_eq!(read(lab.esp_file(BootFile::PreviousInitrd)), initrd(OLD));
        assert_eq!(read(lab.esp_file(BootFile::OldkernEntry)), entry(OLD));
        assert_eq!(lab.esp_tree(), before);
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
        lab.install_kernel(OLDER);
        lab.link_kernel(OLD);
        lab.link_previous(OLDER);
        lab.kernelstub(OLD);
        lab.kernelstub_previous(OLDER);
        assert_eq!(check(&lab.esp, &lab.root, UUID), checked(OLD, good(OLDER)));
    }

    /// PLAN 6b.6 step 4: the recovery entry and folder, `loader.conf`, the random seed,
    /// `entries.srel`, `EFI/BOOT` and `EFI/systemd` are the same files after a put-back, not
    /// rewritten ones.
    #[test]
    fn putting_back_leaves_the_rest_of_the_esp_alone() {
        use std::os::unix::fs::MetadataExt;

        let lab = lab("esp-put-back-bystanders");
        let stat = |path: &str| {
            let meta = fs::symlink_metadata(lab.esp.join(path)).unwrap();
            (meta.ino(), meta.mtime(), meta.mtime_nsec(), meta.len())
        };
        let before: Vec<_> = BYSTANDERS
            .iter()
            .map(|path| (stat(path), read(lab.esp.join(path))))
            .collect();
        let folders = ["EFI", "EFI/BOOT", "EFI/systemd", "EFI/Recovery-ABCD-1234"];
        let folders_before: Vec<_> = folders.iter().map(|path| stat(path)).collect();

        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.kernelstub(OLD);
        lab.kernelstub_previous(OLDER);
        put_back(&lab.esp, &lab.state, UUID).unwrap();

        for (path, was) in BYSTANDERS.iter().zip(before) {
            assert_eq!((stat(path), read(lab.esp.join(path))), was, "{path}");
        }
        for (path, was) in folders.iter().zip(folders_before) {
            assert_eq!(stat(path), was, "{path}");
        }
    }

    /// Check 0.1 on apsis-test: the ESP also holds kernel-install's `<machine-id>/<version>/`
    /// folders and `EFI/Linux/`, all empty. They aren't in the list: the check, the backup,
    /// the put-back and the clearing all pass with them there, nothing of them is backed up,
    /// and they're the same empty folders afterwards.
    #[test]
    fn kernel_installs_empty_folders_are_ignored_and_left_alone() {
        use std::os::unix::fs::MetadataExt;

        let lab = lab("esp-kernel-install");
        let folders = || -> Vec<_> {
            KERNEL_INSTALL
                .iter()
                .map(|path| {
                    let path = lab.esp.join(path);
                    let meta = fs::symlink_metadata(&path).unwrap();
                    assert!(meta.is_dir(), "{path:?}");
                    let empty = fs::read_dir(&path).unwrap().next().is_none();
                    (path, meta.ino(), meta.mtime(), meta.mtime_nsec(), empty)
                })
                .collect()
        };
        let before = folders();
        assert!(before.iter().all(|folder| folder.4), "{before:?}");

        assert_eq!(
            check_before_arming(&lab.esp, &lab.root, UUID),
            Ok(Checked {
                version: NEW.to_owned(),
                previous: good(OLD),
            })
        );
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        verify(&lab.state).unwrap();
        // The seven and the manifest: no folder of the ESP came along.
        assert_eq!(
            fs::read_dir(lab.state.join(BACKUP_DIR)).unwrap().count(),
            BootFile::COUNT + 1
        );
        assert_eq!(folders(), before);

        // The boot refresh, checked, and then put back.
        lab.link_kernel(OLD);
        lab.kernelstub(OLD);
        assert_eq!(check(&lab.esp, &lab.root, UUID), checked(OLD, good(OLD)));
        put_back(&lab.esp, &lab.state, UUID).unwrap();
        clear_temporaries(&lab.esp, UUID).unwrap();
        assert_eq!(folders(), before);
        assert!(boots_kernel(&lab.esp, &lab.root, UUID, NEW));
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
        assert_eq!(
            names,
            [
                "cmdline",
                "initrd.img",
                "initrd.img-previous",
                "vmlinuz-previous.efi",
                "vmlinuz.efi"
            ]
        );
    }

    /// A previous kernel that wasn't there at the backup isn't Apsis's to remove.
    #[test]
    fn putting_back_leaves_alone_what_wasnt_backed_up() {
        let lab = lab("esp-put-back-extra");
        lab.one_kernel();
        back_up(&lab.esp, &lab.state, UUID).unwrap();
        lab.kernelstub_previous(OLDER);
        put_back(&lab.esp, &lab.state, UUID).unwrap();
        assert_eq!(read(lab.esp_file(BootFile::PreviousKernel)), kernel(OLDER));
        assert_eq!(read(lab.esp_file(BootFile::PreviousInitrd)), initrd(OLDER));
        assert_eq!(read(lab.esp_file(BootFile::OldkernEntry)), entry(OLDER));
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

    fn one(current: BootSizes) -> EspSizes {
        EspSizes {
            current,
            previous: None,
        }
    }

    fn two(current: BootSizes, previous: BootSizes) -> EspSizes {
        EspSizes {
            current,
            previous: Some(previous),
        }
    }

    /// Growth of each file, room for the put-back's temporary copy of the largest one, and
    /// the margin.
    #[test]
    fn the_esp_needs_the_growth_a_put_back_copy_and_a_margin() {
        let on_esp = one(sizes(14 * MIB, 150 * MIB));
        // The same sizes: only the put-back copy and the margin.
        assert_eq!(esp_needs(on_esp, on_esp), (150 + 16) * MIB);
        // A larger kernel and initrd.
        assert_eq!(
            esp_needs(on_esp, one(sizes(15 * MIB, 170 * MIB))),
            (1 + 20 + 150 + 16) * MIB
        );
        // Smaller ones free nothing in advance.
        assert_eq!(
            esp_needs(on_esp, one(sizes(10 * MIB, 100 * MIB))),
            (150 + 16) * MIB
        );
        assert_eq!(
            esp_needs(one(sizes(u64::MAX, 0)), one(sizes(0, u64::MAX))),
            u64::MAX
        );
    }

    /// The boot refresh writes the previous pair too.
    #[test]
    fn the_previous_pair_counts_like_the_current_one() {
        let on_esp = two(sizes(14 * MIB, 150 * MIB), sizes(13 * MIB, 160 * MIB));
        // The put-back copy is of the largest file on the ESP, here the previous initrd.
        assert_eq!(esp_needs(on_esp, on_esp), (160 + 16) * MIB);
        // Each of the four grows by itself: a pair that shrinks makes no room for the other.
        assert_eq!(
            esp_needs(
                on_esp,
                two(sizes(10 * MIB, 100 * MIB), sizes(15 * MIB, 165 * MIB))
            ),
            (2 + 5 + 160 + 16) * MIB
        );
        // The snapshot has one kernel: nothing more to write.
        assert_eq!(
            esp_needs(on_esp, one(sizes(14 * MIB, 150 * MIB))),
            (160 + 16) * MIB
        );
        // The ESP has one kernel, the snapshot two: the previous pair is all growth.
        assert_eq!(
            esp_needs(one(sizes(14 * MIB, 150 * MIB)), on_esp),
            (13 + 160 + 150 + 16) * MIB
        );
    }

    /// The owner's machine on 2026-10-01 (Pop!_OS 24.04, kernels 7.1.5 and 7.0.11): the
    /// sizes in `EFI/Pop_OS-<root uuid>/`, and a 1020M ESP with 361M free.
    const REAL: EspSizes = EspSizes {
        current: BootSizes {
            kernel: 17_273_344,
            initrd: 214_307_600,
        },
        previous: Some(BootSizes {
            kernel: 17_056_256,
            initrd: 212_169_876,
        }),
    };
    const REAL_FREE: u64 = 361 * MIB;

    #[test]
    fn the_real_esp_has_room_for_a_restore_to_the_same_kernels() {
        let needs = esp_needs(REAL, REAL);
        assert_eq!(needs, 214_307_600 + 16 * MIB);
        assert_eq!(needs, 231_084_816);
        assert_eq!(check_esp_space(needs, REAL_FREE), Ok(()));
    }

    /// The snapshot from before the kernel update: 7.0.11 is the current pair again, and the
    /// kernel before it (taken as the same size) the previous one.
    #[test]
    fn the_real_esp_has_room_for_a_kernel_rollback() {
        let old = REAL.previous.unwrap();
        let needs = esp_needs(REAL, two(old, old));
        assert_eq!(needs, 231_084_816);
        assert_eq!(check_esp_space(needs, REAL_FREE), Ok(()));
        // And for a snapshot that has 7.0.11 alone.
        assert_eq!(esp_needs(REAL, one(old)), 231_084_816);
    }

    /// 361M free leaves 147_451_120 bytes for the files to grow by. One byte more refuses.
    #[test]
    fn the_real_esp_is_refused_one_byte_past_its_free_space() {
        let room = REAL_FREE - 231_084_816;
        assert_eq!(room, 147_451_120);
        let grown = |by: u64| {
            let mut restored = REAL;
            restored.current.initrd += by;
            esp_needs(REAL, restored)
        };
        assert_eq!(check_esp_space(grown(room), REAL_FREE), Ok(()));
        assert_eq!(
            check_esp_space(grown(room + 1), REAL_FREE),
            Err(Refusal::BootSpace {
                needs: REAL_FREE + 1,
                free: REAL_FREE
            })
        );
    }

    /// The same machine with one kernel on the ESP (the previous pair's bytes free too),
    /// restored to a snapshot with both: the previous pair is written new, and it fits.
    #[test]
    fn the_real_esp_with_one_kernel_has_room_for_a_second() {
        let previous = REAL.previous.unwrap();
        let free = REAL_FREE + previous.kernel + previous.initrd;
        let needs = esp_needs(one(REAL.current), REAL);
        assert_eq!(needs, 17_056_256 + 212_169_876 + 214_307_600 + 16 * MIB);
        assert_eq!(check_esp_space(needs, free), Ok(()));
        // Not in what's free today: the refresh has to fit before anything is removed.
        assert!(check_esp_space(needs, REAL_FREE).is_err());
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
