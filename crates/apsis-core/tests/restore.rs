// SPDX-License-Identifier: GPL-3.0-only

//! The restore's own rsync argv and filter against a real rsync:
//! from a temp "snapshot" tree onto a temp "live root" tree, as the tester, without root.
//!
//! The argv is [`argv::rsync`] and [`argv::rsync_dry_run`], the filter is [`filter::rules`],
//! the exit mapping is [`Copied::end`] and the size is [`space::dry_run_size`]: nothing here
//! is a copy of what core builds. The filter's rules are anchored at the transfer root, so
//! they work unchanged with a temp folder as `/`.
//!
//! Each test skips with a message if rsync isn't installed. What a run without root can't
//! show is in the ignored tests at the end, each with its reason.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hasher};
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime};

use apsis_core::native::Info;
use apsis_core::restore::apply::{self, Copied, CopyEnd, SnapshotFound};
use apsis_core::restore::filter::{self, Home};
use apsis_core::restore::plan::Plan;
use apsis_core::restore::refusal::Refusal;
use apsis_core::restore::{argv, space};
use rustix::process::{Pid, Signal};

/// The kernel the live system runs, and the one the snapshot has.
const NEW: &str = "6.9.3-76060903-generic";
const OLD: &str = "6.8.0-76060800-generic";

/// A typical Pop!_OS machine, with the backup disk mounted for the restore and another
/// disk at `/srv/data`. Devices are placeholders.
const MOUNTINFO: &str = "\
24 1 259:3 / / rw,relatime shared:1 - ext4 /dev/sdX3 rw
25 24 0:5 / /dev rw,nosuid shared:2 - devtmpfs udev rw
26 24 0:22 / /proc rw,nosuid shared:3 - proc proc rw
27 24 0:23 / /sys rw,nosuid shared:4 - sysfs sysfs rw
28 24 0:24 / /run rw shared:5 - tmpfs tmpfs rw
30 24 259:1 / /boot/efi rw,relatime shared:6 - vfat /dev/sdX1 rw
31 24 259:2 / /recovery rw shared:7 - vfat /dev/sdX2 rw
32 24 8:33 / /srv/data rw shared:8 - ext4 /dev/sdY1 rw
33 28 8:17 / /run/apsis/backup ro,nosuid,nodev,noexec - ext4 /dev/sdZ1 ro
";

/// The same machine with `/home` on its own partition.
fn mountinfo_with_home() -> String {
    format!("{MOUNTINFO}34 24 8:49 / /home rw shared:9 - ext4 /dev/sdW1 rw\n")
}

/// The snapshot's own `exclude.list`: what it didn't save stays as it is (rule 11).
const EXCLUDE_LIST: &str = "\
- /root/***
- /var/tmp/***
- /home/*/.cache/***
+ /home/**
";

/// Where a test runs.
struct Lab {
    /// The snapshot's `localhost/`.
    snapshot: PathBuf,
    /// The fake `/`.
    live: PathBuf,
    /// The live root's `/var/lib/apsis/restore`: the filter and rsync's log are here, as for
    /// real.
    state: PathBuf,
    /// Not part of either tree: what a link must never be followed to.
    outside: PathBuf,
}

/// A fresh, filled lab under `target/tmp/restore/`. `None`: rsync isn't installed, and the
/// test is skipped.
fn lab(name: &str) -> Option<Lab> {
    if Command::new("rsync").arg("--version").output().is_err() {
        eprintln!("{name}: skipped, rsync isn't installed");
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("restore")
        .join(name);
    if dir.exists() {
        // An earlier run may have left folders that can't be entered.
        make_writable(&dir);
        fs::remove_dir_all(&dir).unwrap();
    }
    let lab = Lab {
        snapshot: dir.join("backup/timeshift/snapshots/2026-09-25_11-28-00/localhost"),
        live: dir.join("live"),
        state: dir.join("live/var/lib/apsis/restore"),
        outside: dir.join("outside"),
    };
    fill(&lab);
    Some(lab)
}

fn make_writable(dir: &Path) {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return;
    };
    if meta.is_dir() {
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
        for entry in fs::read_dir(dir).unwrap() {
            make_writable(&entry.unwrap().path());
        }
    } else if meta.is_file() {
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o644));
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn link(root: &Path, path: &str, target: impl AsRef<Path>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(target, path).unwrap();
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn exists(root: &Path, path: &str) -> bool {
    fs::symlink_metadata(root.join(path)).is_ok()
}

/// A kernel as its package installs it: the rule 10 files.
fn kernel(root: &Path, version: &str, mark: &str) {
    for name in ["vmlinuz", "initrd.img", "config", "System.map"] {
        write(
            root,
            &format!("boot/{name}-{version}"),
            &format!("{name} {version} {mark}"),
        );
    }
    write(
        root,
        &format!("usr/lib/modules/{version}/kernel/fs/ext4.ko"),
        &format!("module {version} {mark}"),
    );
    link(root, "boot/vmlinuz", format!("vmlinuz-{version}"));
    link(root, "boot/initrd.img", format!("initrd.img-{version}"));
}

/// The two trees. Every file the restore should bring back differs in size from the live
/// one, so the test doesn't hang on two files written in the same instant.
fn fill(lab: &Lab) {
    let Lab {
        snapshot: s,
        live: l,
        outside,
        ..
    } = lab;
    write(outside, "canary", "outside the trees");
    write(outside, "dir/theirs", "outside the trees");

    // ---- the snapshot ----
    write(s, "etc/hostname", "snapshot\n");
    write(s, "etc/fstab", "the snapshot's fstab, of other disks\n");
    write(s, "etc/crypttab", "the snapshot's crypttab\n");
    write(s, "etc/apsis/config.toml", "the snapshot's Apsis config\n");
    // A drop-in leaked into a snapshot: the live, protected one stays.
    write(
        s,
        "etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf",
        "a leaked drop-in\n",
    );
    write(s, "etc/only-in-snapshot", "comes back\n");
    write(s, "usr/bin/tool", "tool 1.0\n");
    write(s, "usr/bin/dash", "a shell\n");
    write(s, "usr/bin/real", "a real file in the snapshot\n");
    write(s, "opt/same/file", "the same on both sides\n");
    write(s, "var/lib/dpkg/status", "Package: apsis\n");
    write(s, "var/cache/app/data", "the snapshot's cache\n");
    write(
        s,
        "var/lib/apsis/restore/request.json",
        "a snapshot's own\n",
    );
    // Links: absolute, relative, to a folder, and one that points out of the tree.
    link(s, "usr/bin/vi", "/etc/alternatives/vi");
    link(s, "usr/bin/sh", "dash");
    link(s, "lib", "usr/lib");
    link(s, "etc/out", outside.join("canary"));
    kernel(s, OLD, "from the snapshot");
    // Snapshots hold a full copy of the ESP, and may hold anything else.
    write(
        s,
        "boot/efi/EFI/Pop_OS/vmlinuz.efi",
        "the snapshot's ESP copy\n",
    );
    write(s, "recovery/from-snapshot", "x\n");
    for dir in [
        "dev", "proc", "sys", "run", "tmp", "mnt", "media", "srv/data",
    ] {
        write(s, &format!("{dir}/from-snapshot"), "never copied\n");
    }
    write(s, "timeshift/from-snapshot", "never copied\n");
    write(s, "var/log/journal/from-snapshot", "never copied\n");
    write(s, "swapfile", "the snapshot's\n");
    write(s, "system-update", "the snapshot's\n");
    write(s, "home/user/doc.txt", "the old document\n");
    write(s, "home/user/.config/app.conf", "old settings\n");
    write(s, "home/user/only-in-snapshot", "comes back with home\n");
    let info = Info {
        created: CREATED,
        sys_uuid: ROOT_UUID.to_owned(),
        sys_distro: "Pop 24.04 (noble)".to_owned(),
        app_version: "apsis 0.5.0".to_owned(),
        file_count: 40,
        tags: vec!["ondemand".to_owned()],
        comments: String::new(),
        live: false,
        kind: "rsync".to_owned(),
        rsync_flags: Some("-aAX --numeric-ids".to_owned()),
    };
    fs::write(s.with_file_name("info.json"), info.to_text()).unwrap();
    fs::write(s.with_file_name("exclude.list"), EXCLUDE_LIST).unwrap();
    age(s);

    // ---- the live root ----
    write(l, "etc/hostname", "live, renamed since\n");
    write(l, "etc/fstab", "the live fstab\n");
    write(l, "etc/crypttab", "the live crypttab\n");
    write(l, "etc/apsis/config.toml", "the live Apsis config\n");
    write(l, "usr/bin/tool", "tool 2.0, updated since\n");
    write(l, "usr/bin/dash", "a newer shell\n");
    write(l, "usr/bin/newer", "installed after the snapshot\n");
    write(
        l,
        "opt/app/bin/run",
        "a folder installed after the snapshot\n",
    );
    write(l, "opt/app/share/data", "x\n");
    write(
        l,
        "var/lib/dpkg/status",
        "Package: apsis\nPackage: cowsay\n",
    );
    write(l, "opt/same/file", "the same on both sides\n");
    // A file where the snapshot has a link, and links where it has a file and a folder.
    write(l, "usr/bin/sh", "a file, not a link\n");
    link(l, "usr/bin/real", outside.join("canary"));
    link(l, "var/cache/app", outside.join("dir"));
    kernel(l, NEW, "running");
    // The protect list.
    link(l, "system-update", "/var/lib/apsis/restore");
    write(l, "etc/systemd/system/apsis-restore.service", "[Unit]\n");
    link(
        l,
        "etc/systemd/system/system-update.target.wants/apsis-restore.service",
        "../apsis-restore.service",
    );
    write(
        l,
        "etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf",
        "[Unit]\nConditionPathExists=!/system-update/apsis-helper\n",
    );
    for name in [
        "request.json",
        "state.json",
        "result.json",
        "apsis-helper",
        // The note beside the filter and the pair an arm keeps. The filter
        // itself is written by each run (`Lab::restore`).
        "restore.note",
        "last-restore.filter",
        "last-restore.note",
    ] {
        write(l, &format!("var/lib/apsis/restore/{name}"), name);
    }
    write(
        l,
        "var/lib/apsis/restore/esp-backup/vmlinuz.efi",
        "backup\n",
    );
    write(l, "var/lib/apsis/restore/esp-backup/manifest.json", "{}\n");
    // The ESP, the recovery partition, and the runtime and other-disk paths.
    write(
        l,
        "boot/efi/EFI/Pop_OS/vmlinuz.efi",
        "the live ESP's kernel\n",
    );
    write(l, "boot/efi/loader/loader.conf", "default Pop_OS-current\n");
    write(l, "recovery/casper/vmlinuz.efi", "recovery\n");
    write(l, "dev/a-device", "x\n");
    write(l, "proc/1/status", "x\n");
    write(l, "sys/kernel/x", "x\n");
    write(l, "run/user/1000/x", "x\n");
    write(l, "tmp/a-temporary-file", "x\n");
    write(l, "mnt/disk/x", "x\n");
    write(l, "media/user/Backup/timeshift/x", "x\n");
    write(l, "lost+found/x", "x\n");
    write(l, "swapfile", "swap\n");
    write(
        l,
        "timeshift/snapshots/2026-01-01_00-00-00/info.json",
        "{}\n",
    );
    write(
        l,
        "var/log/journal/0123/system.journal",
        "the restore's own log\n",
    );
    write(l, "srv/data/on-another-disk", "x\n");
    // What the snapshot's exclude.list left out.
    write(l, "root/.bash_history", "x\n");
    write(l, "var/tmp/x", "x\n");
    write(l, "home/user/.cache/thumbnails", "x\n");
    write(
        l,
        "home/user/doc.txt",
        "the document, edited since the snapshot\n",
    );
    write(
        l,
        "home/user/.config/app.conf",
        "new settings, changed since\n",
    );
    write(l, "home/user/new.txt", "written after the snapshot\n");
    fs::set_permissions(l.join("home"), fs::Permissions::from_mode(0o751)).unwrap();
    // The same file on both sides is the same to rsync: size and time.
    let then = fs::metadata(s.join("opt/same/file"))
        .unwrap()
        .modified()
        .unwrap();
    File::options()
        .write(true)
        .open(l.join("opt/same/file"))
        .unwrap()
        .set_modified(then)
        .unwrap();
    fs::create_dir_all(&lab.state).unwrap();
}

/// Sets every file and folder of the snapshot to the time it was taken.
fn age(dir: &Path) {
    let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.is_dir() {
            age(&path);
        }
        if !meta.is_symlink() {
            File::open(&path).unwrap().set_modified(then).unwrap();
        }
    }
}

/// How a restore is run.
#[derive(Clone, Copy)]
struct Run<'a> {
    home: Home,
    mountinfo: &'a str,
    protected_kernel: Option<&'a str>,
    old_format: bool,
}

impl Default for Run<'_> {
    /// Pass 1 as the apply runs it: home kept, the running kernel protected.
    fn default() -> Self {
        Self {
            home: Home::Keep,
            mountinfo: MOUNTINFO,
            protected_kernel: Some(NEW),
            old_format: false,
        }
    }
}

impl Lab {
    /// Writes core's filter for `run` to the state folder, and gives its path.
    fn write_filter(&self, run: Run<'_>) -> PathBuf {
        let rules = filter::rules(&filter::Request {
            mountinfo: run.mountinfo,
            home: run.home,
            protected_kernel: run.protected_kernel,
            snapshot_excludes: EXCLUDE_LIST,
            restore_root: true,
        })
        .unwrap();
        let path = self.state.join("restore.filter");
        fs::write(&path, filter::to_text(&rules)).unwrap();
        path
    }

    /// Core's argv for the restore.
    fn argv(&self, run: Run<'_>) -> Vec<OsString> {
        let filter = self.write_filter(run);
        argv::rsync(
            &self.snapshot,
            &self.live,
            &filter,
            &self.state.join("rsync-log"),
            run.old_format,
        )
    }

    /// Runs the restore to its end.
    fn restore(&self, run: Run<'_>) -> Output {
        command(&self.argv(run)).output().unwrap()
    }

    /// Runs it and expects exit 0.
    fn restore_ok(&self, run: Run<'_>) {
        let output = self.restore(run);
        assert!(output.status.success(), "{}", text(&output));
    }

    /// Starts the restore in its own process group, with nothing to read.
    fn spawn(&self, run: Run<'_>) -> Child {
        command(&self.argv(run))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap()
    }
}

/// An argv as a command, in the C locale and with nothing else from the tester's environment.
fn command(argv: &[OsString]) -> Command {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env(argv::LOCALE.0, argv::LOCALE.1)
        .stdin(Stdio::null());
    command
}

fn text(output: &Output) -> String {
    format!(
        "{:?}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A finished rsync as core takes it: the exit code, all of the standard output, the errors.
fn copied(output: &Output) -> Copied {
    Copied::new(
        output.status.code(),
        &String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The snapshot's `created`, in its `info.json` and in the plan.
const CREATED: i64 = 1_789_990_080;
const ROOT_UUID: &str = "11111111-2222-3333-4444-555555555555";

fn plan() -> Plan {
    Plan {
        snapshot: "2026-09-25_11-28-00".to_owned(),
        snapshot_created: CREATED,
        backup_uuid: "99999999-8888-7777-6666-555555555555".to_owned(),
        home: Home::Keep,
        old_format: false,
        safety_snapshot: None,
        root_uuid: ROOT_UUID.to_owned(),
        running_kernel: NEW.to_owned(),
        root_needs: 1,
        separate_home: None,
        starter_uid: 1000,
        prepared_at: 1_790_000_000,
    }
}

/// What the runner reads at the snapshot's place before a copy, from the lab's disk.
fn find_snapshot(lab: &Lab) -> SnapshotFound {
    SnapshotFound {
        has_localhost: fs::symlink_metadata(&lab.snapshot).is_ok_and(|meta| meta.is_dir()),
        info: fs::read_to_string(lab.snapshot.with_file_name("info.json")).ok(),
    }
}

/// One thing in a tree as it is on disk, for "untouched": the same file, not a rewritten one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    File {
        inode: u64,
        mtime: (i64, i64),
        mode: u32,
        bytes: Vec<u8>,
    },
    Dir {
        inode: u64,
        mode: u32,
    },
    Link {
        inode: u64,
        target: PathBuf,
    },
    Other {
        inode: u64,
    },
}

/// `path` under `root` and everything in it, never through a link. Empty if it isn't there.
fn nodes(root: &Path, path: &str) -> BTreeMap<String, Node> {
    fn walk(root: &Path, path: &Path, found: &mut BTreeMap<String, Node>) {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        let inode = meta.ino();
        let kind = meta.file_type();
        let node = if kind.is_symlink() {
            Node::Link {
                inode,
                target: fs::read_link(path).unwrap(),
            }
        } else if kind.is_dir() {
            Node::Dir {
                inode,
                mode: meta.mode(),
            }
        } else if kind.is_file() {
            Node::File {
                inode,
                mtime: (meta.mtime(), meta.mtime_nsec()),
                mode: meta.mode(),
                bytes: fs::read(path).unwrap(),
            }
        } else {
            Node::Other { inode }
        };
        let name = path.strip_prefix(root).unwrap().to_str().unwrap();
        found.insert(name.to_owned(), node);
        if kind.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), found);
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(root, &root.join(path), &mut found);
    found
}

/// The nodes of several paths, each of which must be there.
fn nodes_of(root: &Path, paths: &[&str]) -> BTreeMap<String, Node> {
    let mut found = BTreeMap::new();
    for path in paths {
        let part = nodes(root, path);
        assert!(!part.is_empty(), "{path} isn't in the lab");
        found.extend(part);
    }
    // rsync's own log is written into the state folder while it runs.
    found.remove("var/lib/apsis/restore/rsync-log");
    found.remove("var/lib/apsis/restore/restore.filter");
    found
}

/// One thing in a tree as a restore leaves it, for "the same tree": no inodes, no times.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shape {
    File { mode: u32, len: u64, hash: u64 },
    Dir { mode: u32 },
    Link { target: PathBuf },
    Other,
}

fn shape(root: &Path) -> BTreeMap<String, Shape> {
    fn hash(path: &Path) -> u64 {
        let mut file = File::open(path).unwrap();
        let mut hasher = DefaultHasher::new();
        let mut buffer = vec![0; 1 << 20];
        loop {
            match file.read(&mut buffer).unwrap() {
                0 => return hasher.finish(),
                read => hasher.write(&buffer[..read]),
            }
        }
    }
    fn walk(root: &Path, path: &Path, found: &mut BTreeMap<String, Shape>) {
        let meta = fs::symlink_metadata(path).unwrap();
        let kind = meta.file_type();
        let shape = if kind.is_symlink() {
            Shape::Link {
                target: fs::read_link(path).unwrap(),
            }
        } else if kind.is_dir() {
            Shape::Dir { mode: meta.mode() }
        } else if kind.is_file() {
            Shape::File {
                mode: meta.mode(),
                len: meta.len(),
                hash: hash(path),
            }
        } else {
            Shape::Other
        };
        let name = path.strip_prefix(root).unwrap().to_str().unwrap();
        found.insert(name.to_owned(), shape);
        if kind.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), found);
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(root, root, &mut found);
    found.remove("var/lib/apsis/restore/rsync-log");
    found
}

/// The excluded paths that aren't on the protect list.
const EXCLUDED: [&str; 16] = [
    "boot/efi",
    "recovery",
    "etc/fstab",
    "etc/crypttab",
    "dev",
    "proc",
    "sys",
    "run",
    "tmp",
    "mnt",
    "media",
    "lost+found",
    "swapfile",
    "timeshift",
    "var/log/journal",
    "srv/data",
];

// ---- 1. protected and excluded paths ----

/// Whatever the protect list holds: each path is in the lab, and is the same file after the
/// restore (inode, time, bytes), although the snapshot has other files at some of them.
#[test]
fn everything_on_the_protect_list_is_untouched() {
    let Some(lab) = lab("protected") else { return };
    let paths: Vec<&str> = filter::PROTECTED
        .iter()
        .map(|pattern| pattern.trim_end_matches("/***").trim_start_matches('/'))
        .collect();
    assert_eq!(paths.len(), 6);
    let before = nodes_of(&lab.live, &paths);
    for file in [
        "system-update",
        "etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf",
        "var/lib/apsis/restore/request.json",
        "var/lib/apsis/restore/state.json",
        "var/lib/apsis/restore/result.json",
        "var/lib/apsis/restore/restore.note",
        "var/lib/apsis/restore/last-restore.filter",
        "var/lib/apsis/restore/last-restore.note",
        "var/lib/apsis/restore/esp-backup/manifest.json",
        "etc/apsis/config.toml",
    ] {
        assert!(before.contains_key(file), "{file}");
    }

    lab.restore_ok(Run::default());

    assert_eq!(nodes_of(&lab.live, &paths), before);
    // The snapshot's own files at those names didn't come along.
    assert_eq!(
        read(&lab.live, "etc/apsis/config.toml"),
        "the live Apsis config\n"
    );
    assert_eq!(
        read(&lab.live, "var/lib/apsis/restore/request.json"),
        "request.json"
    );
    // The four files of the filter's lifetime are under the same rule: the
    // note and the kept pair are as they were, and the filter the run read is still there
    // though the snapshot has none.
    for name in ["restore.note", "last-restore.filter", "last-restore.note"] {
        assert_eq!(
            read(&lab.live, &format!("var/lib/apsis/restore/{name}")),
            name
        );
    }
    assert!(exists(&lab.live, "var/lib/apsis/restore/restore.filter"));
    assert_eq!(
        read(
            &lab.live,
            "etc/systemd/system/pop-upgrade-init.service.d/50-apsis.conf"
        ),
        "[Unit]\nConditionPathExists=!/system-update/apsis-helper\n",
        "the live drop-in, not the snapshot's leaked one"
    );
    assert!(
        fs::symlink_metadata(lab.live.join("system-update"))
            .unwrap()
            .is_symlink()
    );
}

/// The ESP, the recovery partition, fstab and crypttab, the runtime paths, the journal and
/// other disks: not copied from the snapshot, and not deleted for not being in it.
#[test]
fn excluded_paths_are_neither_copied_nor_deleted() {
    let Some(lab) = lab("excluded") else { return };
    let before = nodes_of(&lab.live, &EXCLUDED);
    lab.restore_ok(Run::default());
    assert_eq!(nodes_of(&lab.live, &EXCLUDED), before);
    for dir in [
        "dev",
        "proc",
        "sys",
        "run",
        "tmp",
        "mnt",
        "media",
        "srv/data",
        "timeshift",
        "recovery",
        "var/log/journal",
    ] {
        assert!(!exists(&lab.live, &format!("{dir}/from-snapshot")), "{dir}");
    }
    assert_eq!(read(&lab.live, "etc/fstab"), "the live fstab\n");
    assert_eq!(
        read(&lab.live, "boot/efi/EFI/Pop_OS/vmlinuz.efi"),
        "the live ESP's kernel\n"
    );
    assert_eq!(read(&lab.live, "swapfile"), "swap\n");
}

/// The fixed rules don't lean on the mounts: with nothing mounted but `/` (the ESP not
/// mounted, the runtime folders plain folders), the same paths are still left alone.
#[test]
fn the_fixed_exclusions_hold_without_any_mount() {
    let Some(lab) = lab("excluded-no-mounts") else {
        return;
    };
    let fixed: Vec<&str> = EXCLUDED
        .into_iter()
        .filter(|path| *path != "srv/data")
        .collect();
    let before = nodes_of(&lab.live, &fixed);
    lab.restore_ok(Run {
        mountinfo: "24 1 259:3 / / rw,relatime shared:1 - ext4 /dev/sdX3 rw\n",
        ..Run::default()
    });
    assert_eq!(nodes_of(&lab.live, &fixed), before);
    // Another disk's folder is only known from its mount: unmounted, it's part of `/`.
    assert!(exists(&lab.live, "srv/data/from-snapshot"));
    assert!(!exists(&lab.live, "srv/data/on-another-disk"));
}

/// A machine id as kernel-install names its ESP folder. A placeholder.
const MACHINE_ID: &str = "0123456789abcdef0123456789abcdef";

/// The snapshot's `boot/efi` is a full copy of the ESP as it was
/// (the kernels, kernel-install's empty `<machine-id>/<version>/` and `EFI/Linux/`), and the
/// live ESP has moved on since. Nothing of the snapshot's copy is transferred, and nothing
/// the snapshot lacks is deleted, on the ESP or in `/recovery`: with the two mounted, and
/// with neither (the fixed rules alone).
#[test]
fn the_snapshots_esp_copy_is_neither_transferred_nor_deleted_against() {
    let no_mounts = "24 1 259:3 / / rw,relatime shared:1 - ext4 /dev/sdX3 rw\n";
    for (name, mountinfo) in [("esp-mounted", MOUNTINFO), ("esp-not-mounted", no_mounts)] {
        let Some(lab) = lab(name) else { return };
        let (s, l) = (&lab.snapshot, &lab.live);
        let folder = format!("boot/efi/EFI/Pop_OS-{ROOT_UUID}");
        // The snapshot's copy: the kernels of then, and what the live ESP doesn't have.
        write(s, &format!("{folder}/vmlinuz.efi"), "the kernel of then\n");
        write(s, &format!("{folder}/initrd.img"), "the initrd of then\n");
        write(s, "boot/efi/loader/entries/Pop_OS-current.conf", "then\n");
        write(s, "boot/efi/loader/entries/Gone-since.conf", "then\n");
        fs::create_dir_all(s.join(format!("boot/efi/{MACHINE_ID}/{OLD}"))).unwrap();
        fs::create_dir_all(s.join("boot/efi/EFI/Linux")).unwrap();
        write(s, "recovery/casper/vmlinuz.efi", "the recovery of then\n");
        age(s);
        // The live ESP: other kernels, and files and folders the snapshot never had.
        write(l, &format!("{folder}/vmlinuz.efi"), "the live kernel\n");
        write(l, &format!("{folder}/initrd.img"), "the live initrd\n");
        write(l, &format!("{folder}/vmlinuz-previous.efi"), "previous\n");
        write(l, "boot/efi/loader/entries/Pop_OS-current.conf", "live\n");
        write(l, "boot/efi/loader/entries/Recovery-ABCD-1234.conf", "x\n");
        fs::create_dir_all(l.join(format!("boot/efi/{MACHINE_ID}/{NEW}"))).unwrap();
        write(l, "recovery/casper/initrd.gz", "only on the live one\n");

        let paths = ["boot/efi", "recovery"];
        let before = nodes_of(l, &paths);
        lab.restore_ok(Run {
            mountinfo,
            ..Run::default()
        });

        assert_eq!(nodes_of(l, &paths), before, "{name}");
        // Said once more by name: not transferred,
        for path in [
            "boot/efi/loader/entries/Gone-since.conf".to_owned(),
            format!("boot/efi/{MACHINE_ID}/{OLD}"),
            "boot/efi/EFI/Linux".to_owned(),
            "recovery/from-snapshot".to_owned(),
        ] {
            assert!(!exists(l, &path), "{name}: {path}");
        }
        assert_eq!(
            read(l, &format!("{folder}/vmlinuz.efi")),
            "the live kernel\n"
        );
        assert_eq!(
            read(l, "recovery/casper/vmlinuz.efi"),
            "recovery\n",
            "{name}"
        );
        // and not deleted.
        for path in [
            format!("{folder}/vmlinuz-previous.efi"),
            "boot/efi/loader/entries/Recovery-ABCD-1234.conf".to_owned(),
            format!("boot/efi/{MACHINE_ID}/{NEW}"),
            "recovery/casper/initrd.gz".to_owned(),
        ] {
            assert!(exists(l, &path), "{name}: {path}");
        }
        // The rest of the restore ran.
        assert_eq!(read(l, "etc/hostname"), "snapshot\n", "{name}");
    }
}

/// The snapshot's own `exclude.list` (rule 11): what it didn't save isn't deleted.
#[test]
fn what_the_snapshot_didnt_save_stays() {
    let Some(lab) = lab("rule-11") else { return };
    let kept = ["root", "var/tmp"];
    let before = nodes_of(&lab.live, &kept);
    lab.restore_ok(Run::default());
    assert_eq!(nodes_of(&lab.live, &kept), before);
}

// ---- 2. deleted, changed, links ----

#[test]
fn what_the_snapshot_lacks_is_deleted_and_changed_files_come_back_byte_for_byte() {
    let Some(lab) = lab("restored") else { return };
    let same = nodes(&lab.live, "opt/same");
    lab.restore_ok(Run::default());

    // Deleted: a file and a whole folder that came after the snapshot.
    assert!(!exists(&lab.live, "usr/bin/newer"));
    assert!(!exists(&lab.live, "opt/app"));
    // Changed files, and one the live system had lost.
    for file in [
        "etc/hostname",
        "usr/bin/tool",
        "usr/bin/dash",
        "var/lib/dpkg/status",
        "etc/only-in-snapshot",
    ] {
        assert_eq!(
            fs::read(lab.live.join(file)).unwrap(),
            fs::read(lab.snapshot.join(file)).unwrap(),
            "{file}"
        );
        let time = |root: &Path| fs::metadata(root.join(file)).unwrap().modified().unwrap();
        assert_eq!(time(&lab.live), time(&lab.snapshot), "{file}");
    }
    // A file that was already the same isn't written again.
    assert_eq!(nodes(&lab.live, "opt/same"), same);
}

#[test]
fn links_come_back_as_links_and_are_never_followed() {
    let Some(lab) = lab("links") else { return };
    let outside = nodes(&lab.outside, "");
    lab.restore_ok(Run::default());

    let target = |path: &str| fs::read_link(lab.live.join(path)).unwrap();
    assert_eq!(target("usr/bin/vi"), Path::new("/etc/alternatives/vi"));
    assert_eq!(target("lib"), Path::new("usr/lib"));
    // A live file where the snapshot has a link becomes the link.
    assert_eq!(target("usr/bin/sh"), Path::new("dash"));
    // A link out of the tree comes back as that link.
    assert_eq!(target("etc/out"), lab.outside.join("canary"));
    // A live link where the snapshot has a file: the link is replaced, its target isn't
    // written through.
    let real = fs::symlink_metadata(lab.live.join("usr/bin/real")).unwrap();
    assert!(real.is_file());
    assert_eq!(
        read(&lab.live, "usr/bin/real"),
        "a real file in the snapshot\n"
    );
    // A live link to a folder where the snapshot has a folder: the same.
    let app = fs::symlink_metadata(lab.live.join("var/cache/app")).unwrap();
    assert!(app.is_dir());
    assert_eq!(
        read(&lab.live, "var/cache/app/data"),
        "the snapshot's cache\n"
    );
    // Nothing outside the tree was written, deleted or touched.
    assert_eq!(nodes(&lab.outside, ""), outside);
}

/// `-a` brings FIFOs back as FIFOs. (Device nodes need root: see the ignored tests.)
#[test]
fn a_fifo_comes_back_as_a_fifo() {
    let Some(lab) = lab("fifo") else { return };
    let fifo = lab.snapshot.join("var/lib/app.fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    lab.restore_ok(Run::default());
    let restored = fs::symlink_metadata(lab.live.join("var/lib/app.fifo")).unwrap();
    assert!(restored.file_type().is_fifo());
}

// ---- 3. home ----

/// Keep: `/home` is fully excluded. Nothing in it is compared, written or deleted, and
/// `/home` itself keeps its mode and its time.
#[test]
fn a_kept_home_is_untouched() {
    for (name, mountinfo) in [
        ("home-kept", MOUNTINFO.to_owned()),
        ("home-kept-separate", mountinfo_with_home()),
    ] {
        let Some(lab) = lab(name) else { return };
        let run = Run {
            mountinfo: &mountinfo,
            ..Run::default()
        };
        let before = nodes(&lab.live, "home");
        let home = |lab: &Lab| {
            let meta = fs::metadata(lab.live.join("home")).unwrap();
            (meta.mode() & 0o7777, meta.mtime(), meta.mtime_nsec())
        };
        let home_before = home(&lab);
        assert_eq!(home_before.0, 0o751);

        lab.restore_ok(run);

        assert_eq!(nodes(&lab.live, "home"), before, "{name}");
        assert_eq!(home(&lab), home_before, "{name}");
        // The rest was restored all the same.
        assert_eq!(read(&lab.live, "etc/hostname"), "snapshot\n");
    }
}

/// Restore: `/home` becomes what the snapshot holds, also when it's a partition of its own.
/// What the snapshot's `exclude.list` left out (the caches) stays.
#[test]
fn a_restored_home_becomes_the_snapshots() {
    for (name, mountinfo) in [
        ("home-restored", MOUNTINFO.to_owned()),
        ("home-restored-separate", mountinfo_with_home()),
    ] {
        let Some(lab) = lab(name) else { return };
        let cache = nodes(&lab.live, "home/user/.cache");
        lab.restore_ok(Run {
            home: Home::Restore,
            mountinfo: &mountinfo,
            ..Run::default()
        });
        assert_eq!(read(&lab.live, "home/user/doc.txt"), "the old document\n");
        assert_eq!(
            read(&lab.live, "home/user/.config/app.conf"),
            "old settings\n"
        );
        assert_eq!(
            read(&lab.live, "home/user/only-in-snapshot"),
            "comes back with home\n"
        );
        // Written after the snapshot: gone.
        assert!(!exists(&lab.live, "home/user/new.txt"), "{name}");
        // `/home` itself is the snapshot's again.
        let mode = |root: &Path| fs::metadata(root.join("home")).unwrap().mode();
        assert_eq!(mode(&lab.live), mode(&lab.snapshot), "{name}");
        assert_eq!(nodes(&lab.live, "home/user/.cache"), cache, "{name}");
    }
}

/// What tells the two apart for a separate `/home`: rule 5's line for its mount point.
#[test]
fn a_separate_home_has_its_mount_rule_only_when_its_kept() {
    let rules = |home| {
        filter::rules(&filter::Request {
            mountinfo: &mountinfo_with_home(),
            home,
            protected_kernel: None,
            snapshot_excludes: "",
            restore_root: true,
        })
        .unwrap()
    };
    assert!(rules(Home::Keep).contains(&"- /home".to_owned()));
    assert!(rules(Home::Keep).contains(&"- /home/***".to_owned()));
    let restored = rules(Home::Restore);
    assert!(!restored.iter().any(|rule| rule.starts_with("- /home")));
}

// ---- 4. the kernel guard ----

/// Rule 10, pass 1: the kernel the ESP boots keeps its modules folder and its `/boot` files,
/// the same files, while the rest of `/boot` and the modules become the snapshot's.
#[test]
fn the_booted_kernels_files_survive_pass_1() {
    let Some(lab) = lab("kernel-guard") else {
        return;
    };
    let guarded = [
        format!("usr/lib/modules/{NEW}"),
        format!("boot/vmlinuz-{NEW}"),
        format!("boot/initrd.img-{NEW}"),
        format!("boot/config-{NEW}"),
        format!("boot/System.map-{NEW}"),
    ];
    let guarded: Vec<&str> = guarded.iter().map(String::as_str).collect();
    let before = nodes_of(&lab.live, &guarded);

    lab.restore_ok(Run::default());

    assert_eq!(nodes_of(&lab.live, &guarded), before);
    // The snapshot's kernel arrived, and `/boot`'s links name it.
    assert_eq!(
        read(&lab.live, &format!("boot/vmlinuz-{OLD}")),
        format!("vmlinuz {OLD} from the snapshot")
    );
    assert!(exists(
        &lab.live,
        &format!("usr/lib/modules/{OLD}/kernel/fs/ext4.ko")
    ));
    assert_eq!(
        fs::read_link(lab.live.join("boot/vmlinuz")).unwrap(),
        Path::new(&format!("vmlinuz-{OLD}"))
    );
    assert_eq!(
        fs::read_link(lab.live.join("boot/initrd.img")).unwrap(),
        Path::new(&format!("initrd.img-{OLD}"))
    );
}

/// The rule is what keeps them: without it, a kernel the snapshot doesn't have is deleted
/// like anything else.
#[test]
fn without_the_guard_a_kernel_the_snapshot_lacks_is_deleted() {
    let Some(lab) = lab("kernel-unguarded") else {
        return;
    };
    lab.restore_ok(Run {
        protected_kernel: None,
        ..Run::default()
    });
    assert!(!exists(&lab.live, &format!("usr/lib/modules/{NEW}")));
    for name in ["vmlinuz", "initrd.img", "config", "System.map"] {
        assert!(!exists(&lab.live, &format!("boot/{name}-{NEW}")), "{name}");
    }
}

// ---- 5. a copy cut partway ----

/// A file large enough that rsync is still writing it when the test acts: 128 MiB of
/// nothing, between files that sort before it and files that sort after it.
fn add_large_file(lab: &Lab) {
    let path = lab.snapshot.join("usr/lib/large.bin");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    File::create(&path).unwrap().set_len(128 << 20).unwrap();
    age(&lab.snapshot);
}

/// Waits until rsync is writing the large file: its temporary file is in the live root.
fn wait_for_the_large_file(lab: &Lab, child: &mut Child) -> PathBuf {
    let dir = lab.live.join("usr/lib");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let temporary = fs::read_dir(&dir).ok().and_then(|entries| {
            entries.map(|entry| entry.unwrap().path()).find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".large.bin."))
            })
        });
        if let Some(path) = temporary {
            return path;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "rsync ended before it got to the large file"
        );
        assert!(
            Instant::now() < deadline,
            "rsync never got to the large file"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Kills rsync's whole group, as a power cut would stop it, and waits until it's gone.
fn kill_group(child: &mut Child) {
    let group = Pid::from_child(child);
    rustix::process::kill_process_group(group, Signal::KILL).unwrap();
    child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while rustix::process::test_kill_process_group(group).is_ok() {
        assert!(Instant::now() < deadline, "rsync's processes never ended");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The copy is killed while it writes a file in the middle of the tree. Run again, it ends
/// with the same tree as a copy that was never cut, and the protected paths are still the
/// same files.
#[test]
fn a_copy_cut_partway_and_run_again_ends_like_an_uncut_one() {
    let (Some(cut), Some(uncut)) = (lab("cut"), lab("uncut")) else {
        return;
    };
    add_large_file(&cut);
    add_large_file(&uncut);
    uncut.restore_ok(Run::default());

    let protected = ["var/lib/apsis", "etc/apsis", "boot/efi", "system-update"];
    let before = nodes_of(&cut.live, &protected);
    let mut child = cut.spawn(Run::default());
    let temporary = wait_for_the_large_file(&cut, &mut child);
    kill_group(&mut child);

    // Really cut partway: what sorts before the large file is done, what sorts after it
    // isn't, and rsync's temporary file is left behind.
    assert_eq!(read(&cut.live, "etc/hostname"), "snapshot\n");
    assert!(temporary.exists());
    assert!(!exists(&cut.live, "usr/lib/large.bin"));
    assert_eq!(
        read(&cut.live, "var/lib/dpkg/status"),
        "Package: apsis\nPackage: cowsay\n"
    );
    assert_ne!(shape(&cut.live), comparable(&uncut, &cut));

    cut.restore_ok(Run::default());

    assert!(!temporary.exists());
    assert_eq!(shape(&cut.live), comparable(&uncut, &cut));
    assert_eq!(nodes_of(&cut.live, &protected), before);
}

/// The uncut lab's tree as the cut lab's would be: the two labs are in different folders, so
/// the links that point out of the tree name different places.
fn comparable(uncut: &Lab, cut: &Lab) -> BTreeMap<String, Shape> {
    let mut tree = shape(&uncut.live);
    for node in tree.values_mut() {
        if let Shape::Link { target } = node
            && let Ok(inside) = target.strip_prefix(&uncut.outside)
        {
            *target = cut.outside.join(inside);
        }
    }
    tree
}

// ---- 6. exit codes ----

#[test]
fn a_clean_copy_exits_0_and_ends() {
    let Some(lab) = lab("exit-0") else { return };
    let output = lab.restore(Run::default());
    assert_eq!(output.status.code(), Some(0), "{}", text(&output));
    assert_eq!(copied(&output).end(), CopyEnd::Ended { problems: false });
}

/// Plain exit 23: a file of the snapshot that can't be read. The rest is copied, and what
/// the snapshot lacks is still deleted: a restore with problems.
#[test]
fn a_file_that_cant_be_read_exits_23_and_ends_with_problems() {
    let Some(lab) = lab("exit-23") else { return };
    if rustix::process::getuid().is_root() {
        eprintln!("exit-23: skipped, root reads every file");
        return;
    }
    let locked = lab.snapshot.join("usr/bin/tool");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let output = lab.restore(Run::default());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(output.status.code(), Some(23), "{}", text(&output));
    // Plain 23: rsync went on deleting, so this is a restore with problems.
    assert!(!copied(&output).deletions_skipped, "{}", text(&output));
    assert_eq!(copied(&output).end(), CopyEnd::Ended { problems: true });
    let tail = copied(&output).tail;
    assert!(tail.contains("Permission denied (13)"), "{tail}");
    assert!(tail.contains("(code 23)"), "{tail}");
    // What the journal gets, one line each: the file rsync couldn't read, by name
    // and with the reason, then rsync's closing line. No empty line among them.
    let copied = copied(&output);
    let lines: Vec<&str> = copied.error_lines().collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("rsync: [sender] send_files failed to open ")
            && lines[0].contains("usr/bin/tool")
            && lines[0].ends_with(": Permission denied (13)"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("rsync error: some files/attrs were not transferred")
            && lines[1].contains("(code 23)"),
        "{lines:?}"
    );
    // The file stays as it was; everything else is restored.
    assert_eq!(read(&lab.live, "usr/bin/tool"), "tool 2.0, updated since\n");
    assert_eq!(read(&lab.live, "etc/hostname"), "snapshot\n");
    assert!(!exists(&lab.live, "usr/bin/newer"));
}

/// Exit 23 is also what rsync gives when a whole folder of the snapshot can't be read, as
/// when the backup disk goes away under it. From there on it deletes nothing ("IO error
/// encountered -- skipping file deletion"), so the tree isn't the snapshot's: what came
/// after the snapshot stays. Core takes 23 with that line as a copy that broke: no boot
/// refresh, and another attempt.
#[test]
fn a_folder_that_cant_be_read_exits_23_skips_deletions_and_is_a_copy_that_broke() {
    let Some(lab) = lab("exit-23-folder") else {
        return;
    };
    if rustix::process::getuid().is_root() {
        eprintln!("exit-23-folder: skipped, root reads every folder");
        return;
    }
    // The first folder rsync gets to.
    write(&lab.snapshot, "a-first/file", "x\n");
    age(&lab.snapshot);
    let locked = lab.snapshot.join("a-first");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let output = lab.restore(Run::default());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    // rsync made the folder in the live root with the same mode: don't leave one behind
    // that `rm -r target` can't enter.
    let _ = fs::set_permissions(lab.live.join("a-first"), fs::Permissions::from_mode(0o755));

    assert_eq!(output.status.code(), Some(23), "{}", text(&output));
    // rsync says so on its standard output, not with the errors, and core reads it there.
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains(apply::DELETIONS_SKIPPED), "{}", text(&output));
    assert!(!copied(&output).tail.contains(apply::DELETIONS_SKIPPED));
    assert!(copied(&output).deletions_skipped);
    assert_eq!(copied(&output).end(), CopyEnd::Broke);
    // Copied, but nothing deleted.
    assert_eq!(read(&lab.live, "etc/hostname"), "snapshot\n");
    assert!(exists(&lab.live, "usr/bin/newer"));
    assert!(exists(&lab.live, "opt/app/bin/run"));
}

/// Plain exit 23 is also what a snapshot that isn't there at all gives: nothing is copied,
/// nothing deleted, and no line about deletions. rsync can't tell that from a restore with
/// problems, so the apply checks the snapshot before every copy (`check_snapshot`): held
/// here against the same lab, it passes while the snapshot is there and refuses once it's
/// gone.
#[test]
fn a_snapshot_that_is_gone_exits_plain_23_and_is_what_the_check_before_the_copy_catches() {
    let Some(lab) = lab("exit-23-gone") else {
        return;
    };
    assert_eq!(apply::check_snapshot(&plan(), &find_snapshot(&lab)), Ok(()));
    let before = shape(&lab.live);
    fs::rename(&lab.snapshot, lab.snapshot.with_file_name("gone")).unwrap();

    assert_eq!(
        apply::check_snapshot(&plan(), &find_snapshot(&lab)),
        Err("the snapshot's folder isn't on the backup disk".to_owned())
    );
    // What rsync does if it's run all the same.
    let output = lab.restore(Run::default());
    assert_eq!(output.status.code(), Some(23), "{}", text(&output));
    assert!(!copied(&output).deletions_skipped, "{}", text(&output));
    assert_eq!(copied(&output).end(), CopyEnd::Ended { problems: true });
    let mut after = shape(&lab.live);
    after.remove("var/lib/apsis/restore/restore.filter");
    assert_eq!(after, before);
}

/// The check holds the snapshot's own `info.json` against the plan: another snapshot at the
/// same name (another creation time) isn't restored.
#[test]
fn another_snapshot_at_the_same_name_fails_the_check_before_the_copy() {
    let Some(lab) = lab("check-other") else {
        return;
    };
    let other = Plan {
        snapshot_created: CREATED + 3600,
        ..plan()
    };
    assert_eq!(
        apply::check_snapshot(&other, &find_snapshot(&lab)),
        Err(
            "the snapshot on the backup disk isn't the one this restore was prepared for"
                .to_owned()
        )
    );
    fs::remove_file(lab.snapshot.with_file_name("info.json")).unwrap();
    assert_eq!(
        apply::check_snapshot(&plan(), &find_snapshot(&lab)),
        Err("the snapshot's info.json can't be read".to_owned())
    );
}

/// Exit 24: a file that was in the snapshot when rsync listed it and is gone when it gets
/// there.
#[test]
fn a_file_that_vanishes_exits_24_and_ends() {
    let Some(lab) = lab("exit-24") else { return };
    add_large_file(&lab);
    let argv = lab.argv(Run::default());
    let mut child = command(&argv)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_the_large_file(&lab, &mut child);
    // Listed already, and after the large file in rsync's order.
    fs::remove_file(lab.snapshot.join("var/lib/dpkg/status")).unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(24), "{}", text(&output));
    assert_eq!(copied(&output).end(), CopyEnd::Ended { problems: false });
    assert!(copied(&output).tail.contains("vanished"));
    assert_eq!(read(&lab.live, "etc/hostname"), "snapshot\n");
    assert!(exists(&lab.live, "usr/lib/large.bin"));
}

/// A hard failure: the filter file is missing. Exit 11, before anything is copied, and a
/// copy that broke.
#[test]
fn a_filter_that_cant_be_read_exits_11_and_broke() {
    let Some(lab) = lab("exit-11") else { return };
    let before = shape(&lab.live);
    let argv = lab.argv(Run::default());
    fs::remove_file(lab.state.join("restore.filter")).unwrap();
    let output = command(&argv).output().unwrap();
    assert_eq!(output.status.code(), Some(11), "{}", text(&output));
    assert_eq!(copied(&output).end(), CopyEnd::Broke);
    assert_eq!(shape(&lab.live), before);
}

/// SIGTERM, as a shutdown sends it: rsync exits 20, a copy that broke.
#[test]
fn a_terminated_copy_exits_20_and_broke() {
    let Some(lab) = lab("exit-20") else { return };
    add_large_file(&lab);
    let argv = lab.argv(Run::default());
    let mut child = command(&argv)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_the_large_file(&lab, &mut child);
    rustix::process::kill_process(Pid::from_child(&child), Signal::TERM).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(20), "{}", text(&output));
    assert_eq!(copied(&output).end(), CopyEnd::Broke);
}

/// SIGKILL: no exit code at all, and a copy that broke.
#[test]
fn a_killed_copy_has_no_exit_code_and_broke() {
    let Some(lab) = lab("exit-none") else { return };
    add_large_file(&lab);
    let mut child = lab.spawn(Run::default());
    wait_for_the_large_file(&lab, &mut child);
    let group = Pid::from_child(&child);
    rustix::process::kill_process_group(group, Signal::KILL).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), None);
    let broke = Copied::new(status.code(), "", String::new());
    assert_eq!(broke.end(), CopyEnd::Broke);
}

// ---- 7. the dry run ----

/// Core's dry run against real output: the size is read by core's own parser, it's what
/// the files to copy add up to, the real run's `--stats` says the same, and nothing was
/// written.
#[test]
fn the_dry_runs_size_is_read_from_real_output_and_nothing_is_written() {
    let Some(lab) = lab("dry-run") else { return };
    let filter = lab.write_filter(Run::default());
    let before = shape(&lab.live);
    let dry = argv::rsync_dry_run(&lab.snapshot, &lab.live, &filter, false);
    let output = command(&dry).output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let stats = String::from_utf8(output.stdout).unwrap();

    // Nothing written: no file changed, and no log.
    assert_eq!(shape(&lab.live), before);
    assert!(!lab.state.join("rsync-log").exists());

    // What the restore would copy: every file of the snapshot that's new or changed.
    let expected: u64 = [
        "etc/hostname",
        "etc/only-in-snapshot",
        "usr/bin/tool",
        "usr/bin/dash",
        "usr/bin/real",
        "var/lib/dpkg/status",
        "var/cache/app/data",
        &format!("boot/vmlinuz-{OLD}"),
        &format!("boot/initrd.img-{OLD}"),
        &format!("boot/config-{OLD}"),
        &format!("boot/System.map-{OLD}"),
        &format!("usr/lib/modules/{OLD}/kernel/fs/ext4.ko"),
    ]
    .iter()
    .map(|file| fs::metadata(lab.snapshot.join(file)).unwrap().len())
    .sum();
    assert_eq!(space::dry_run_size(&stats), Ok(expected), "{stats}");
    assert_eq!(space::transfer_size(&stats), Some(expected));
    // `--no-human-readable`: a plain count, and the line core reads.
    assert!(
        stats.contains(&format!("Total transferred file size: {expected} bytes")),
        "{stats}"
    );
    assert!(stats.contains("(DRY RUN)"), "{stats}");

    // The restore itself copies that much.
    let output = lab.restore(Run::default());
    assert!(output.status.success(), "{}", text(&output));
    let real = String::from_utf8(output.stdout).unwrap();
    assert_eq!(space::transfer_size(&real), Some(expected), "{real}");
}

/// A size core can't read refuses, and is never taken as zero: real output with the size in
/// rsync's human-readable form, and the output of a dry run that failed.
#[test]
fn real_output_without_a_readable_size_refuses() {
    let Some(lab) = lab("dry-run-refused") else {
        return;
    };
    add_large_file(&lab);
    let filter = lab.write_filter(Run::default());
    let mut dry = argv::rsync_dry_run(&lab.snapshot, &lab.live, &filter, false);

    // The run itself is fine and has a size.
    let output = command(&dry).output().unwrap();
    let stats = String::from_utf8(output.stdout).unwrap();
    assert!(space::dry_run_size(&stats).is_ok_and(|size| size > 128 << 20));

    // Human-readable: "134.22M bytes" (128 MiB, in units of 1000) isn't a count.
    for arg in &mut dry {
        if arg == "--no-human-readable" {
            *arg = "--human-readable".into();
        }
    }
    let output = command(&dry).output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    let stats = String::from_utf8(output.stdout).unwrap();
    assert!(
        stats.contains("Total transferred file size: 134.22M bytes"),
        "{stats}"
    );
    assert_eq!(space::transfer_size(&stats), None, "{stats}");
    assert_eq!(space::dry_run_size(&stats), Err(Refusal::SizeUnknown));

    // A dry run that couldn't start prints no stats.
    fs::remove_file(&filter).unwrap();
    let output = command(&dry).output().unwrap();
    assert_eq!(output.status.code(), Some(11));
    let stats = String::from_utf8(output.stdout).unwrap();
    assert_eq!(space::dry_run_size(&stats), Err(Refusal::SizeUnknown));
}

/// The numbers are read as text, so the locale matters: the tester's own may print them
/// another way, and the C locale is what every run here uses.
#[test]
fn every_run_is_in_the_c_locale() {
    let Some(lab) = lab("locale") else { return };
    let filter = lab.write_filter(Run::default());
    let dry = argv::rsync_dry_run(&lab.snapshot, &lab.live, &filter, false);
    let run = command(&dry);
    let locale: Vec<_> = run
        .get_envs()
        .filter(|(name, _)| name.to_string_lossy().starts_with("LC_") || *name == "LANG")
        .collect();
    assert_eq!(
        locale,
        [(
            std::ffi::OsStr::new("LC_ALL"),
            Some(std::ffi::OsStr::new("C"))
        )]
    );
}

// ---- 8. ACLs and extended attributes, where the filesystem has them ----

/// Sets a `user.*` attribute, if the filesystem under the lab stores them.
fn set_user_xattr(path: &Path, value: &[u8]) -> bool {
    match rustix::fs::setxattr(
        path,
        "user.apsis-test",
        value,
        rustix::fs::XattrFlags::empty(),
    ) {
        Ok(()) => true,
        Err(rustix::io::Errno::NOTSUP) => false,
        Err(error) => panic!("setxattr: {error}"),
    }
}

fn user_xattr(path: &Path) -> Option<Vec<u8>> {
    let mut value = [0_u8; 64];
    rustix::fs::getxattr(path, "user.apsis-test", &mut value[..])
        .ok()
        .map(|length| value[..length].to_vec())
}

/// A new-format snapshot is restored with `-A -X`: a `user.*` attribute and an ACL come
/// back. Both can be set without root on the tester's own files.
#[test]
fn a_new_format_restore_brings_back_xattrs_and_acls() {
    let Some(lab) = lab("xattrs") else { return };
    let source = lab.snapshot.join("usr/bin/tool");
    let restored = lab.live.join("usr/bin/tool");
    let has_xattr = set_user_xattr(&source, b"from the snapshot");
    let entry = format!("u:{}:r--", rustix::process::getuid().as_raw());
    let has_acl = Command::new("setfacl")
        .args(["-m", &entry])
        .arg(&source)
        .output()
        .is_ok_and(|output| output.status.success());
    if !has_xattr && !has_acl {
        eprintln!("xattrs: skipped, this filesystem stores neither user xattrs nor ACLs");
        return;
    }
    age(&lab.snapshot);
    lab.restore_ok(Run::default());
    if has_xattr {
        assert_eq!(
            user_xattr(&restored).as_deref(),
            Some(&b"from the snapshot"[..])
        );
    } else {
        eprintln!("xattrs: the user xattr part is skipped, this filesystem has none");
    }
    if has_acl {
        let acl = |path: &Path| {
            let mut value = [0_u8; 256];
            rustix::fs::getxattr(path, "system.posix_acl_access", &mut value[..])
                .ok()
                .map(|length| value[..length].to_vec())
        };
        assert!(acl(&source).is_some());
        assert_eq!(acl(&restored), acl(&source));
    } else {
        eprintln!("xattrs: the ACL part is skipped, setfacl didn't work here");
    }
}

/// An old-format snapshot never stored extended attributes. Restored with `-X`, rsync would
/// strip the live ones to match it, also from files that didn't change; without `-X` (what
/// core's argv does for it) they stay. Both are run, so the test shows why the flag is left
/// out.
#[test]
fn an_old_format_restore_doesnt_strip_live_xattrs() {
    for (name, old_format) in [("xattrs-old-format", true), ("xattrs-new-format", false)] {
        let Some(lab) = lab(name) else { return };
        // A file that's the same on both sides, with an attribute only the live one has.
        let unchanged = lab.live.join("opt/same/file");
        if !set_user_xattr(&unchanged, b"live only") {
            eprintln!("{name}: skipped, this filesystem stores no user xattrs");
            return;
        }
        lab.restore_ok(Run {
            old_format,
            ..Run::default()
        });
        let after = user_xattr(&unchanged);
        if old_format {
            assert_eq!(after.as_deref(), Some(&b"live only"[..]));
        } else {
            assert_eq!(after, None, "with -X the snapshot's (no) attributes win");
        }
    }
}

// ---- what a run without root can't show ----
//
// These need root to set up (another owner, a device node, a file capability), and the
// tests never run anything as root. They're ignored here, with the reason.
// `sudo cargo test -- --ignored` would run them; nobody is asked to.

/// `--numeric-ids`: owners come back by number, whatever the user database says.
#[test]
#[ignore = "needs root: only root can give a file to another owner"]
fn owners_come_back_by_number() {
    let Some(lab) = lab("root-owners") else {
        return;
    };
    let file = lab.snapshot.join("usr/bin/tool");
    // A uid and gid that no user database on a test machine names.
    std::os::unix::fs::chown(&file, Some(54_321), Some(54_322)).unwrap();
    age(&lab.snapshot);
    lab.restore_ok(Run::default());
    let restored = fs::metadata(lab.live.join("usr/bin/tool")).unwrap();
    assert_eq!((restored.uid(), restored.gid()), (54_321, 54_322));
}

/// `-a` is `--devices --specials` too: a device node comes back as that device.
#[test]
#[ignore = "needs root: only root can make a device node"]
fn a_device_node_comes_back_as_a_device() {
    let Some(lab) = lab("root-device") else {
        return;
    };
    let node = lab.snapshot.join("var/lib/a-device");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &node,
        rustix::fs::FileType::CharacterDevice,
        rustix::fs::Mode::from_raw_mode(0o600),
        rustix::fs::makedev(1, 3),
    )
    .unwrap();
    lab.restore_ok(Run::default());
    let restored = fs::symlink_metadata(lab.live.join("var/lib/a-device")).unwrap();
    assert!(restored.file_type().is_char_device());
    assert_eq!(restored.rdev(), rustix::fs::makedev(1, 3));
}

/// `-X` as root copies `security.*` too: a file capability (`ping`'s) comes back.
#[test]
#[ignore = "needs root: security.capability needs CAP_SETFCAP"]
fn a_file_capability_comes_back() {
    let Some(lab) = lab("root-capability") else {
        return;
    };
    let file = lab.snapshot.join("usr/bin/tool");
    // cap_net_raw=ep, version 2, as `setcap` writes it.
    let capability = [
        1, 0, 0, 2, 0, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    rustix::fs::setxattr(
        &file,
        "security.capability",
        &capability,
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    age(&lab.snapshot);
    lab.restore_ok(Run::default());
    let mut value = [0_u8; 64];
    let length = rustix::fs::getxattr(
        lab.live.join("usr/bin/tool"),
        "security.capability",
        &mut value[..],
    )
    .unwrap();
    assert_eq!(value[..length], capability);
}
