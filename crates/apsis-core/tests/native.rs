// SPDX-License-Identifier: GPL-3.0-only

//! The native rsync backend against real files and a real rsync.
//!
//! Every scenario runs in a folder under `target/tmp/`, and again on a loop-mounted ext4 image
//! when `APSIS_EXT4_MNT` names its mount point (it must be ext4 on `/dev/loop*`, or the tests
//! fail). Nothing here needs root; the image is mounted beforehand by the user.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, FileTimes};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use apsis_core::native::{self, CreatePlan, NativeConfig, NativeRsync, QuietRunner, exclude};
use apsis_core::{Backend, Error, RunOutput, Runner, Tag};
use jiff::Zoned;
use jiff::tz::{Offset, TimeZone};
use serde_json::Value;

const SYS_UUID: &str = "00000000-0000-0000-0000-000000000000";
const OTHER_SYS_UUID: &str = "99999999-9999-9999-9999-999999999999";
const FIXTURE_REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/native-repo");
const FIXTURE_NAME: &str = "2026-09-20_10-00-00";

/// Where a scenario runs: a source tree standing in for `/`, and a repository standing in for
/// the backup device's mount.
struct Lab {
    kind: &'static str,
    source: PathBuf,
    repo: PathBuf,
}

/// A fresh lab per place the tests run: `target/tmp/`, and the ext4 image if it's mounted.
fn labs(test: &str) -> Vec<Lab> {
    let mut bases = vec![(
        "plain",
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native"),
    )];
    if let Some(mount) = std::env::var_os("APSIS_EXT4_MNT") {
        let mount = PathBuf::from(mount);
        assert_loop_ext4(&mount);
        bases.push(("ext4 loop", mount.join("apsis-tests")));
    }
    bases
        .into_iter()
        .map(|(kind, base)| {
            let root = base.join(test);
            match fs::remove_dir_all(&root) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => panic!("{e}"),
                _ => {}
            }
            let lab = Lab {
                kind,
                source: root.join("source"),
                repo: root.join("repo"),
            };
            fs::create_dir_all(&lab.source).unwrap();
            fs::create_dir_all(&lab.repo).unwrap();
            lab
        })
        .collect()
}

/// `APSIS_EXT4_MNT` really is an ext4 filesystem on a loop device. The same mount can be listed
/// more than once (stacked mounts, or a sandbox re-binding the tree), so every line must match.
fn assert_loop_ext4(mount: &Path) {
    let output = Command::new("findmnt")
        .args([
            "--noheadings",
            "--output",
            "FSTYPE,SOURCE,TARGET",
            "--mountpoint",
        ])
        .arg(mount)
        .output()
        .expect("findmnt");
    let text = String::from_utf8_lossy(&output.stdout);
    let is_loop_ext4 = |line: &str| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.len() == 3 && fields[0] == "ext4" && fields[1].starts_with("/dev/loop")
    };
    assert!(
        text.lines().next().is_some() && text.lines().all(is_loop_ext4),
        "APSIS_EXT4_MNT={} is not an ext4 loop mount: {text:?}",
        mount.display()
    );
}

/// A clock that starts at 2026-09-25 11:28:53 at UTC+2 and moves a minute per call.
fn clock() -> impl Fn() -> Zoned + Send + Sync + 'static {
    let tz = TimeZone::fixed(Offset::from_hours(2).unwrap());
    let start = jiff::civil::date(2026, 9, 25)
        .at(11, 28, 53, 0)
        .to_zoned(tz)
        .unwrap();
    let minutes = AtomicI64::new(0);
    move || {
        let n = minutes.fetch_add(1, Ordering::SeqCst);
        start.checked_add(jiff::Span::new().minutes(n)).unwrap()
    }
}

fn config(lab: &Lab, dry_run: bool) -> NativeConfig {
    NativeConfig {
        repo: lab.repo.clone(),
        device: Some("/dev/sdX1".to_owned()),
        device_uuid: Some("11111111-1111-1111-1111-111111111111".to_owned()),
        source: lab.source.clone(),
        sys_uuid: SYS_UUID.to_owned(),
        sys_distro: "Pop 24.04 (noble)".to_owned(),
        exclude: exclude::for_backup(&[], "", &[]),
        dry_run,
    }
}

type Log = Arc<Mutex<Vec<String>>>;

/// A backend on `lab` with the test clock and a log the test can read.
fn backend(lab: &Lab, dry_run: bool) -> (NativeRsync<QuietRunner>, Log) {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let backend = NativeRsync::new(config(lab, dry_run), QuietRunner::new(path))
        .with_clock(clock())
        .with_log(move |line| sink.lock().unwrap().push(line.to_owned()));
    (backend, log)
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A small system: files to keep, and some in folders Timeshift excludes.
fn populate(source: &Path) {
    write(&source.join("etc/same"), "never changes\n");
    write(&source.join("etc/changes"), "version 1\n");
    write(
        &source.join("etc/goes-away"),
        "deleted before the second snapshot\n",
    );
    write(&source.join("usr/bin/tool"), "#!/bin/sh\n");
    fs::set_permissions(
        source.join("usr/bin/tool"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    write(&source.join("tmp/scratch"), "excluded: /tmp/*\n");
    write(&source.join("proc/1/status"), "excluded: /proc/*\n");
    write(&source.join("home/user1/notes"), "excluded: /home/*/**\n");
    write(&source.join("home/user1/.cache/x"), "excluded\n");
    write(&source.join("root/.bashrc"), "excluded: /root/**\n");
    write(
        &source.join("timeshift/snapshots/old"),
        "excluded: /timeshift/*\n",
    );
}

fn snapshot_dir(lab: &Lab, name: &str) -> PathBuf {
    lab.repo.join("timeshift/snapshots").join(name)
}

fn localhost(lab: &Lab, name: &str) -> PathBuf {
    snapshot_dir(lab, name).join("localhost")
}

fn inode(path: &Path) -> (u64, u64) {
    let meta = fs::symlink_metadata(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (meta.dev(), meta.ino())
}

fn nlink(path: &Path) -> u64 {
    fs::symlink_metadata(path).unwrap().nlink()
}

fn names(backend: &impl Backend) -> Vec<String> {
    backend
        .list()
        .unwrap()
        .snapshots
        .into_iter()
        .map(|s| s.name)
        .collect()
}

/// Regular files under `dir`, relative, with their inodes.
fn files(dir: &Path) -> BTreeMap<PathBuf, (u64, u64)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.insert(path.strip_prefix(dir).unwrap().to_owned(), inode(&path));
            }
        }
    }
    out
}

/// Every path under `dir` with its inode, size and change time: equal before and after means
/// nothing was written.
fn state(dir: &Path) -> BTreeMap<PathBuf, (u64, u64, i64, i64)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(current) = stack.pop() {
        let meta = fs::symlink_metadata(&current).unwrap();
        out.insert(
            current.clone(),
            (meta.ino(), meta.size(), meta.ctime(), meta.ctime_nsec()),
        );
        if meta.is_dir() {
            for entry in fs::read_dir(&current).unwrap() {
                stack.push(entry.unwrap().path());
            }
        }
    }
    out
}

/// Names of the snapshots the test clock gives, in order.
const FIRST: &str = "2026-09-25_11-28-53";
const SECOND: &str = "2026-09-25_11-29-53";
const THIRD: &str = "2026-09-25_11-30-53";

#[test]
fn unchanged_files_share_inodes_changed_ones_do_not() {
    for lab in labs("hardlinks") {
        let kind = lab.kind;
        populate(&lab.source);
        let (backend, _) = backend(&lab, false);

        backend.create("first").unwrap();
        let first = localhost(&lab, FIRST);
        assert_eq!(
            fs::read_to_string(first.join("etc/changes")).unwrap(),
            "version 1\n"
        );
        assert!(first.join("etc/goes-away").exists(), "{kind}");
        // Excluded folders are there, empty: Timeshift's patterns end in `/*` or `/**`.
        for dir in ["tmp", "proc", "home/user1", "root", "timeshift"] {
            assert_eq!(
                fs::read_dir(first.join(dir)).unwrap().count(),
                0,
                "{kind}: {dir} should be empty"
            );
        }
        // The first snapshot is a full copy: nothing to link to.
        assert_eq!(nlink(&first.join("etc/same")), 1, "{kind}");

        write(&lab.source.join("etc/changes"), "version 2, longer\n");
        fs::remove_file(lab.source.join("etc/goes-away")).unwrap();
        write(
            &lab.source.join("etc/new"),
            "added before the second snapshot\n",
        );
        backend.create("second").unwrap();
        let second = localhost(&lab, SECOND);

        // Unchanged: the same inode in both snapshots.
        for file in ["etc/same", "usr/bin/tool"] {
            assert_eq!(
                inode(&first.join(file)),
                inode(&second.join(file)),
                "{kind}: {file}"
            );
            assert_eq!(nlink(&second.join(file)), 2, "{kind}: {file}");
        }
        // Changed: separate inodes, each snapshot keeps its own version.
        assert_ne!(
            inode(&first.join("etc/changes")),
            inode(&second.join("etc/changes")),
            "{kind}"
        );
        assert_eq!(
            fs::read_to_string(first.join("etc/changes")).unwrap(),
            "version 1\n"
        );
        assert_eq!(
            fs::read_to_string(second.join("etc/changes")).unwrap(),
            "version 2, longer\n"
        );
        assert_eq!(nlink(&second.join("etc/changes")), 1, "{kind}");
        // Deleted and added files.
        assert!(!second.join("etc/goes-away").exists(), "{kind}");
        assert!(first.join("etc/goes-away").exists(), "{kind}");
        assert_eq!(nlink(&second.join("etc/new")), 1, "{kind}");
        // Directories are never hard links: each snapshot has its own.
        assert_ne!(
            inode(&first.join("etc")),
            inode(&second.join("etc")),
            "{kind}"
        );

        // Nothing changed: every file of the third is the second's.
        backend.create("third").unwrap();
        let third = localhost(&lab, THIRD);
        assert_eq!(files(&second), files(&third), "{kind}");
        assert_eq!(nlink(&third.join("etc/same")), 3, "{kind}");

        assert_eq!(names(&backend), [FIRST, SECOND, THIRD], "{kind}");
        // Nothing is left in the staging folder.
        assert!(!lab.repo.join("timeshift/apsis-staging").exists(), "{kind}");
    }
}

/// A snapshot folder read the way Timeshift reads it, written here from Timeshift's source
/// independently of Apsis's own parser (`SnapshotRepo.load_snapshots`,
/// `Snapshot.read_control_file`, `Snapshot.read_exclude_list`).
#[derive(Debug)]
struct TimeshiftView {
    name: String,
    valid: bool,
    created: i64,
    sys_uuid: String,
    sys_distro: String,
    app_version: String,
    file_count: u64,
    tags: Vec<String>,
    description: String,
    live: bool,
    kind: String,
    exclude_list: Vec<String>,
}

fn timeshift_view(snapshots: &Path) -> Vec<TimeshiftView> {
    let mut out = Vec::new();
    for entry in fs::read_dir(snapshots).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name == ".sync" || !fs::metadata(&path).unwrap().is_dir() {
            continue;
        }
        let json: Option<serde_json::Map<String, Value>> =
            fs::read_to_string(path.join("info.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
        let exclude = fs::read_to_string(path.join("exclude.list")).ok();
        let get = |key: &str| {
            json.as_ref().and_then(|j| j.get(key)).map(|v| {
                v.as_str()
                    .expect("Timeshift reads every member as a string")
                    .to_owned()
            })
        };
        out.push(TimeshiftView {
            valid: json.is_some() && exclude.is_some(),
            created: get("created").map_or(0, |v| v.parse().unwrap()),
            sys_uuid: get("sys-uuid").unwrap_or_default(),
            sys_distro: get("sys-distro").unwrap_or_default(),
            app_version: get("app-version").unwrap_or_default(),
            file_count: get("file_count").map_or(0, |v| v.parse().unwrap()),
            tags: get("tags")
                .unwrap_or_default()
                .split(' ')
                .map(|t| t.trim().to_owned())
                .collect(),
            description: get("comments").unwrap_or_default(),
            live: get("live").is_some_and(|v| v == "true"),
            kind: get("type").unwrap_or_else(|| "rsync".to_owned()),
            exclude_list: exclude
                .unwrap_or_default()
                .split('\n')
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect(),
            name,
        });
    }
    out.sort_by_key(|s| s.created);
    out
}

#[test]
fn timeshift_can_read_a_native_snapshot() {
    for lab in labs("timeshift-reads-native") {
        let kind = lab.kind;
        populate(&lab.source);
        let (backend, _) = backend(&lab, false);
        backend.create("apsis: native, \"quoted\"").unwrap();

        let snapshots = lab.repo.join("timeshift/snapshots");
        let view = timeshift_view(&snapshots);
        assert_eq!(view.len(), 1, "{kind}");
        let s = &view[0];
        assert!(s.valid, "{kind}");
        assert_eq!(s.name, FIRST);
        // The name is the local time (UTC+2 here) of `created` (UTC).
        assert_eq!(s.created, 1_790_328_533, "{kind}");
        assert_eq!(s.sys_uuid, SYS_UUID);
        assert_eq!(s.sys_distro, "Pop 24.04 (noble)");
        assert_eq!(s.app_version, native::APP_VERSION);
        assert_eq!(s.tags, ["ondemand"]);
        assert_eq!(s.description, "apsis: native, \"quoted\"");
        assert!(!s.live);
        assert_eq!(s.kind, "rsync");
        assert_eq!(s.exclude_list, exclude::for_backup(&[], "", &[]));
        let log = fs::read(snapshots.join(FIRST).join("rsync-log")).unwrap();
        assert_eq!(
            s.file_count,
            log.iter().filter(|&&b| b == b'\n').count() as u64
        );
        assert!(s.file_count > 0, "{kind}");

        // Byte for byte as json-glib writes it (written out here by hand, not by Apsis).
        let expected = format!(
            "{{\n  \"created\" : \"1790328533\",\n  \"sys-uuid\" : \"{SYS_UUID}\",\n  \
             \"sys-distro\" : \"Pop 24.04 (noble)\",\n  \"app-version\" : \"{}\",\n  \
             \"file_count\" : \"{}\",\n  \"tags\" : \"ondemand\",\n  \"comments\" : \
             \"apsis: native, \\\"quoted\\\"\",\n  \"live\" : \"false\",\n  \"type\" : \
             \"rsync\"\n}}",
            native::APP_VERSION,
            s.file_count
        );
        let info = fs::read_to_string(snapshots.join(FIRST).join("info.json")).unwrap();
        assert_eq!(info, expected, "{kind}");

        // `create_symlinks`: a folder per tag, a relative link per tagged snapshot.
        let timeshift = lab.repo.join("timeshift");
        for tag in ["boot", "hourly", "daily", "weekly", "monthly"] {
            let dir = timeshift.join(format!("snapshots-{tag}"));
            assert_eq!(fs::read_dir(dir).unwrap().count(), 0, "{kind}: {tag}");
        }
        let link = timeshift.join("snapshots-ondemand").join(FIRST);
        assert_eq!(
            fs::read_link(&link).unwrap(),
            PathBuf::from(format!("../snapshots/{FIRST}"))
        );
        assert!(link.join("info.json").exists(), "{kind}: the link resolves");
    }
}

/// The fixture's `info.json` and `exclude.list` came from a snapshot Timeshift 24.01.1 made
/// (redacted: UUIDs zeroed, homes renamed to `/home/userN`). Apsis writes the same bytes.
#[test]
fn info_json_is_written_as_timeshift_writes_it() {
    let path = Path::new(FIXTURE_REPO)
        .join("timeshift/snapshots")
        .join(FIXTURE_NAME)
        .join("info.json");
    let text = fs::read_to_string(path).unwrap();
    let info = native::Info::parse(&text).unwrap();
    assert_eq!(info.created, 1_790_116_435);
    assert_eq!(info.file_count, 1_218_050);
    assert_eq!(info.to_text(), text);
}

/// The same `exclude.list` from the settings it was made with. Those are read back from the
/// list: the user filters are everything between the fixed extras and the home entries
/// (24.01.1's order), `/recovery/*` comes from Pop!_OS's fstab, and the two `+ /home/userN/**`
/// lines were two different homes before redaction (the builder drops exact duplicates).
#[test]
fn exclude_list_is_built_as_timeshift_builds_it() {
    let path = Path::new(FIXTURE_REPO)
        .join("timeshift/snapshots")
        .join(FIXTURE_NAME)
        .join("exclude.list");
    let expected = fs::read_to_string(path).unwrap();
    let user: Vec<String> = [
        "+ /root/**",
        "+ /home/user1/**",
        "/var/lib/libvirt/**",
        "+ /home/user2/**",
    ]
    .map(str::to_owned)
    .to_vec();
    let fstab = "UUID=0000 / ext4 noatime,errors=remount-ro 0 0\n\
        PARTUUID=0001 /boot/efi vfat umask=0077 0 0\n\
        PARTUUID=0002 /recovery vfat umask=0077 0 0\n\
        /dev/mapper/cryptswap none swap defaults 0 0\n";
    let passwd = "root:x:0:0:root:/root:/bin/bash\n\
        daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n\
        user1:x:1000:1000::/home/user1:/bin/bash\n\
        user2:x:1001:1001::/home/user2:/bin/bash\n\
        nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n";
    let users = exclude::home_users(passwd, Path::new("/nonexistent-apsis-root"));
    let text = exclude::to_text(&exclude::for_backup(&user, fstab, &users));
    let redacted = text
        .replace("/home/user1/", "/home/userN/")
        .replace("/home/user2/", "/home/userN/");
    assert_eq!(redacted, expected);
}

/// Copies the fixture tree (made to Timeshift's layout) into the lab's repository.
fn copy_fixture(lab: &Lab) {
    let status = Command::new("cp")
        .arg("-a")
        .arg(format!("{FIXTURE_REPO}/timeshift"))
        .arg(&lab.repo)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Makes `source/etc/apsis-fixture` identical (content, size, mtime, mode) to the fixture's
/// copy, so rsync counts it as unchanged.
fn match_fixture_file(lab: &Lab) {
    let fixture = localhost(lab, FIXTURE_NAME).join("etc/apsis-fixture");
    let target = lab.source.join("etc/apsis-fixture");
    fs::copy(&fixture, &target).unwrap();
    let meta = fs::metadata(&fixture).unwrap();
    fs::set_permissions(&target, meta.permissions()).unwrap();
    let times = FileTimes::new().set_modified(meta.modified().unwrap());
    File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(times)
        .unwrap();
    write(
        &lab.source.join("etc/apsis-changed"),
        "changed since the fixture\n",
    );
}

#[test]
fn native_reads_and_links_against_a_timeshift_snapshot() {
    for lab in labs("native-reads-timeshift") {
        let kind = lab.kind;
        copy_fixture(&lab);
        let (backend, _) = backend(&lab, false);

        let list = backend.list().unwrap();
        assert!(list.warnings.is_empty(), "{kind}: {:?}", list.warnings);
        assert_eq!(list.snapshots.len(), 1);
        let s = &list.snapshots[0];
        assert_eq!(s.name, FIXTURE_NAME);
        assert_eq!(s.tags, [Tag::OnDemand]);
        assert_eq!(s.comment, None);

        populate(&lab.source);
        match_fixture_file(&lab);
        // `plan` takes a minute of the test clock: the create is the second name.
        let plan = backend.plan("").unwrap();
        assert_eq!(
            plan.link_from,
            Some(localhost(&lab, FIXTURE_NAME)),
            "{kind}"
        );
        backend.create("").unwrap();

        let old = localhost(&lab, FIXTURE_NAME);
        let new = localhost(&lab, SECOND);
        assert_eq!(
            inode(&old.join("etc/apsis-fixture")),
            inode(&new.join("etc/apsis-fixture")),
            "{kind}: the unchanged file is linked to Timeshift's copy"
        );
        assert_ne!(
            inode(&old.join("etc/apsis-changed")),
            inode(&new.join("etc/apsis-changed")),
            "{kind}"
        );
        assert_eq!(names(&backend), [FIXTURE_NAME, SECOND], "{kind}");

        // Rebuilding the tag folders keeps Timeshift's snapshot's tags.
        let timeshift = lab.repo.join("timeshift");
        for (tag, name) in [("ondemand", FIXTURE_NAME), ("ondemand", SECOND)] {
            let link = timeshift.join(format!("snapshots-{tag}")).join(name);
            assert_eq!(
                fs::read_link(link).unwrap(),
                PathBuf::from(format!("../snapshots/{name}")),
                "{kind}"
            );
        }
        // No comment: Timeshift writes `""`.
        let view = timeshift_view(&timeshift.join("snapshots"));
        assert_eq!(view[1].description, "");
    }
}

#[test]
fn dry_run_logs_the_plan_and_writes_nothing() {
    for lab in labs("dry-run") {
        let kind = lab.kind;
        populate(&lab.source);
        // One clock for both, so the dry run gets the next name.
        let shared = Arc::new(clock());
        let tick = || {
            let shared = Arc::clone(&shared);
            move || shared()
        };
        let path = std::env::var_os("PATH").unwrap_or_default();
        // One real snapshot first, so the plan has something to link to.
        NativeRsync::new(config(&lab, false), QuietRunner::new(path.clone()))
            .with_clock(tick())
            .create("real")
            .unwrap();
        let before = state(&lab.repo);

        let log: Log = Arc::default();
        let sink = Arc::clone(&log);
        let dry = NativeRsync::new(config(&lab, true), QuietRunner::new(path))
            .with_clock(tick())
            .with_log(move |line| sink.lock().unwrap().push(line.to_owned()));
        dry.create("dry").unwrap();

        assert_eq!(
            state(&lab.repo),
            before,
            "{kind}: the dry run wrote something"
        );
        let log = log.lock().unwrap().join("\n");
        let staging = lab.repo.join("timeshift/apsis-staging").join(SECOND);
        assert!(
            log.starts_with(&format!("dry run: native create {SECOND}, nothing written")),
            "{log}"
        );
        assert!(
            log.contains(&format!(
                "run (argv, no shell): rsync -aii --recursive --verbose --delete --force \
                 --stats --sparse --delete-excluded --link-dest={}/ --log-file={}/rsync-log \
                 --exclude-from={}/exclude.list --delete-excluded {}/ {}/localhost/",
                localhost(&lab, FIRST).display(),
                staging.display(),
                staging.display(),
                lab.source.display(),
                staging.display(),
            )),
            "{kind}: {log}"
        );
        assert!(log.contains("  \"comments\" : \"dry\","), "{log}");
        assert!(log.contains("\n  /home/*/**\n"), "{log}");
        assert_eq!(names(&dry), [FIRST], "{kind}");
    }
}

/// A runner that fails like rsync does when the disk fills up.
struct FailingRsync;

impl Runner for FailingRsync {
    fn run(&self, _: &[OsString]) -> io::Result<RunOutput> {
        Ok(RunOutput {
            success: false,
            code: Some(11),
            stdout: String::new(),
            stderr: "rsync: write failed: No space left on device (28)".to_owned(),
        })
    }
}

#[test]
fn failed_rsync_leaves_no_snapshot_behind() {
    for lab in labs("rsync-fails") {
        let kind = lab.kind;
        populate(&lab.source);
        let backend = NativeRsync::new(config(&lab, false), FailingRsync).with_clock(clock());
        let error = backend.create("doomed").unwrap_err();
        assert!(
            matches!(&error, Error::Native(m) if m.contains("code 11") && m.contains("No space")),
            "{kind}: {error:?}"
        );
        assert!(!lab.repo.join("timeshift/apsis-staging").exists(), "{kind}");
        assert_eq!(
            fs::read_dir(lab.repo.join("timeshift/snapshots"))
                .unwrap()
                .count(),
            0,
            "{kind}"
        );
    }
}

#[test]
fn other_systems_and_incomplete_snapshots_are_not_linked() {
    for lab in labs("link-choice") {
        let kind = lab.kind;
        populate(&lab.source);
        let (backend, _) = backend(&lab, false);
        backend.create("").unwrap();

        // A newer snapshot of another system, and a newer incomplete one.
        let other = snapshot_dir(&lab, "2026-09-26_00-00-00");
        write(&other.join("exclude.list"), "/tmp/*\n");
        write(&other.join("localhost/etc/same"), "never changes\n");
        let info = fs::read_to_string(snapshot_dir(&lab, FIRST).join("info.json"))
            .unwrap()
            .replace(SYS_UUID, OTHER_SYS_UUID)
            .replace("\"1790328533\"", "\"1790380800\"");
        fs::write(other.join("info.json"), info).unwrap();
        let incomplete = snapshot_dir(&lab, "2026-09-27_00-00-00");
        fs::create_dir_all(incomplete.join("localhost")).unwrap();
        write(&incomplete.join("exclude.list"), "");

        let plan: CreatePlan = backend.plan("").unwrap();
        assert_eq!(plan.link_from, Some(localhost(&lab, FIRST)), "{kind}");

        let list = backend.list().unwrap();
        assert_eq!(list.snapshots.len(), 2, "{kind}");
        assert_eq!(
            list.warnings,
            ["2026-09-27_00-00-00: incomplete: no info.json"],
            "{kind}"
        );
    }
}

#[test]
fn delete_removes_the_tree_and_its_links_only() {
    for lab in labs("delete") {
        let kind = lab.kind;
        populate(&lab.source);
        let (backend, _) = backend(&lab, false);
        backend.create("").unwrap();
        backend.create("").unwrap();
        let second = localhost(&lab, SECOND);
        assert_eq!(nlink(&second.join("etc/same")), 2);

        backend.delete(FIRST).unwrap();
        assert!(!snapshot_dir(&lab, FIRST).exists(), "{kind}");
        assert!(
            fs::symlink_metadata(lab.repo.join("timeshift/snapshots-ondemand").join(FIRST))
                .is_err(),
            "{kind}"
        );
        // The remaining snapshot keeps its data; its hard links just lost a name.
        assert_eq!(nlink(&second.join("etc/same")), 1, "{kind}");
        assert_eq!(
            fs::read_to_string(second.join("etc/same")).unwrap(),
            "never changes\n"
        );
        assert_eq!(names(&backend), [SECOND]);

        assert!(matches!(
            backend.delete(FIRST),
            Err(Error::NoSuchSnapshot(_))
        ));
        assert!(matches!(
            backend.delete("../x"),
            Err(Error::InvalidSnapshotName(_))
        ));
    }
}

#[test]
fn a_leftover_staging_folder_is_reported() {
    for lab in labs("staging-leftover") {
        let kind = lab.kind;
        fs::create_dir_all(lab.repo.join("timeshift/apsis-staging/2026-09-01_00-00-00")).unwrap();
        let (backend, _) = backend(&lab, false);
        let list = backend.list().unwrap();
        assert!(list.snapshots.is_empty(), "{kind}");
        assert_eq!(list.warnings.len(), 1, "{kind}");
        assert!(
            list.warnings[0].contains("interrupted native create"),
            "{kind}"
        );
    }
}

#[test]
fn snapshot_name_taken_in_the_same_second_is_refused() {
    for lab in labs("same-second") {
        populate(&lab.source);
        let fixed = jiff::civil::date(2026, 9, 25)
            .at(11, 28, 53, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let backend = NativeRsync::new(config(&lab, false), QuietRunner::new(path))
            .with_clock(move || fixed.clone());
        backend.create("").unwrap();
        let error = backend.create("").unwrap_err();
        assert!(
            matches!(error, Error::Native(ref m) if m.contains("already exists")),
            "{error:?}"
        );
    }
}

#[test]
fn mtimes_and_modes_survive() {
    for lab in labs("attributes") {
        let kind = lab.kind;
        populate(&lab.source);
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let file = lab.source.join("etc/same");
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old))
            .unwrap();
        let (backend, _) = backend(&lab, false);
        backend.create("").unwrap();
        let copy = localhost(&lab, FIRST).join("etc/same");
        assert_eq!(
            fs::metadata(copy).unwrap().modified().unwrap(),
            old,
            "{kind}"
        );
        let tool = localhost(&lab, FIRST).join("usr/bin/tool");
        assert_eq!(
            fs::metadata(tool).unwrap().permissions().mode() & 0o777,
            0o755,
            "{kind}"
        );
    }
}

#[test]
fn symlinked_folders_are_not_written_through() {
    for lab in labs("symlinks") {
        let kind = lab.kind;
        populate(&lab.source);
        let elsewhere = lab.repo.parent().unwrap().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, lab.repo.join("timeshift")).unwrap();
        let (backend, _) = backend(&lab, false);
        let error = backend.create("").unwrap_err();
        assert!(
            matches!(error, Error::Native(ref m) if m.contains("symlink")),
            "{kind}: {error:?}"
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0, "{kind}");
        assert!(
            matches!(backend.delete(FIRST), Err(Error::Native(_))),
            "{kind}"
        );
    }
}
