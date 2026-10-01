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

use apsis_core::native::{
    self, Cancel, CreatePlan, NativeConfig, NativeRsync, QuietRunner, exclude,
};
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
        exclude: exclude::for_backup(&[], false, false, "", &[]),
        dry_run,
    }
}

type Log = Arc<Mutex<Vec<String>>>;

/// A backend on `lab` with the test clock and a log the test can read.
fn backend(lab: &Lab, dry_run: bool) -> (NativeRsync<QuietRunner>, Log) {
    let log: Log = Arc::default();
    let sink = Arc::clone(&log);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let backend = NativeRsync::new(config(lab, dry_run), QuietRunner::new(path).low_priority())
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
fn create_reports_rsync_progress_up_to_the_end() {
    for lab in labs("progress") {
        let kind = lab.kind;
        populate(&lab.source);
        let seen: Arc<Mutex<Vec<apsis_core::Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let (backend, _) = backend(&lab, false);
        let backend = backend.with_progress(move |p| sink.lock().unwrap().push(p));
        backend.create("progress").unwrap();
        let seen = seen.lock().unwrap();
        // rsync ends `--info=progress2` with a line for the whole transfer.
        let last = seen.last().unwrap_or_else(|| panic!("{kind}: no progress"));
        assert_eq!(last.percent, Some(100.0), "{kind}: {seen:?}");
        assert_eq!(last.eta_seconds, Some(0), "{kind}");
        // The snapshot itself is as without progress.
        assert!(
            localhost(&lab, FIRST).join("etc/changes").exists(),
            "{kind}"
        );
    }
}

/// An extended attribute of `path`, or `None` if it has none by that name.
fn xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut buffer = [0_u8; 256];
    match rustix::fs::getxattr(path, name, &mut buffer[..]) {
        Ok(len) => Some(buffer[..len].to_vec()),
        Err(rustix::io::Errno::NODATA) => None,
        Err(e) => panic!("{}: {name}: {e}", path.display()),
    }
}

/// The file's POSIX ACL as the kernel stores it.
const ACL_XATTR: &str = "system.posix_acl_access";

/// Since 0.4.1: `-A -X`. Both can be set without root on the tester's own files. Where the
/// filesystem under the lab doesn't store them, that part is skipped with a message.
#[test]
fn acls_and_extended_attributes_are_kept() {
    for lab in labs("acls-xattrs") {
        let kind = lab.kind;
        populate(&lab.source);
        let tool = lab.source.join("usr/bin/tool");
        let same = lab.source.join("etc/same");

        let has_xattr = match rustix::fs::setxattr(
            &tool,
            "user.apsis-test",
            b"kept",
            rustix::fs::XattrFlags::empty(),
        ) {
            Ok(()) => true,
            Err(rustix::io::Errno::NOTSUP) => {
                eprintln!("{kind}: no user xattrs on this filesystem, skipping that part");
                false
            }
            Err(e) => panic!("{kind}: setxattr: {e}"),
        };
        // A named entry for the tester's own uid, by number: it needs no root, and the uid
        // exists in any user namespace the tests run in.
        let entry = format!("u:{}:r--", rustix::process::getuid().as_raw());
        let has_acl = match Command::new("setfacl")
            .args(["-m", &entry])
            .arg(&same)
            .output()
        {
            Ok(output) if output.status.success() => true,
            Ok(output) => {
                eprintln!(
                    "{kind}: no ACLs on this filesystem, skipping that part: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                false
            }
            Err(e) => {
                eprintln!("{kind}: setfacl didn't run, skipping the ACL part: {e}");
                false
            }
        };
        let live_acl = has_acl.then(|| xattr(&same, ACL_XATTR).expect("the ACL just set"));

        let (backend, _) = backend(&lab, false);
        backend.create("first").unwrap();
        let first = localhost(&lab, FIRST);
        if has_xattr {
            assert_eq!(
                xattr(&first.join("usr/bin/tool"), "user.apsis-test").as_deref(),
                Some(&b"kept"[..]),
                "{kind}"
            );
        }
        if has_acl {
            assert_eq!(
                xattr(&first.join("etc/same"), ACL_XATTR),
                live_acl,
                "{kind}"
            );
        }
        // A file with neither gets neither.
        assert_eq!(xattr(&first.join("etc/changes"), "user.apsis-test"), None);
        assert_eq!(xattr(&first.join("etc/changes"), ACL_XATTR), None, "{kind}");

        let text = fs::read_to_string(snapshot_dir(&lab, FIRST).join("info.json")).unwrap();
        assert!(
            !native::Info::parse(&text).unwrap().is_old_format(),
            "{kind}"
        );

        // `--link-dest` still links them: rsync finds the same ACL and xattrs in the first.
        backend.create("second").unwrap();
        let second = localhost(&lab, SECOND);
        for file in ["usr/bin/tool", "etc/same"] {
            assert_eq!(
                inode(&first.join(file)),
                inode(&second.join(file)),
                "{kind}: {file}"
            );
        }

        // An xattr that changed is a change: the file is copied again, and each snapshot keeps
        // its own.
        if has_xattr {
            rustix::fs::setxattr(
                &tool,
                "user.apsis-test",
                b"changed",
                rustix::fs::XattrFlags::empty(),
            )
            .unwrap();
            backend.create("third").unwrap();
            let third = localhost(&lab, THIRD);
            assert_ne!(
                inode(&second.join("usr/bin/tool")),
                inode(&third.join("usr/bin/tool")),
                "{kind}"
            );
            assert_eq!(
                xattr(&third.join("usr/bin/tool"), "user.apsis-test").as_deref(),
                Some(&b"changed"[..]),
                "{kind}"
            );
            assert_eq!(
                xattr(&second.join("usr/bin/tool"), "user.apsis-test").as_deref(),
                Some(&b"kept"[..]),
                "{kind}"
            );
        }
    }
}

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
        assert_eq!(
            s.exclude_list,
            exclude::for_backup(&[], false, false, "", &[])
        );
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
             \"rsync\",\n  \"apsis-rsync-flags\" : \"-aAX --numeric-ids\"\n}}",
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
    assert!(info.is_old_format(), "Timeshift's own snapshot");
    assert_eq!(info.to_text(), text);
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
                "run (argv, no shell): rsync -aii -A -X --numeric-ids --recursive --verbose --delete --force \
                 --stats --sparse --delete-excluded --info=progress2 --link-dest={}/ --log-file={}/rsync-log \
                 --exclude-from={}/exclude.list {}/ {}/localhost/",
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

/// Two snapshots made by the backend, for the delete tests.
fn two_snapshots(lab: &Lab) -> NativeRsync<QuietRunner> {
    populate(&lab.source);
    let (backend, _) = backend(lab, false);
    backend.create("").unwrap();
    backend.create("").unwrap();
    backend
}

#[test]
fn delete_refuses_names_that_arent_snapshot_names() {
    for lab in labs("delete-names") {
        let backend = two_snapshots(&lab);
        for bad in [
            "",
            ".",
            "..",
            "snapshots",
            "../snapshots",
            "2026-09-25_11-28-53/..",
            "*",
        ] {
            assert!(
                matches!(backend.delete(bad), Err(Error::InvalidSnapshotName(_))),
                "{bad:?}"
            );
        }
        assert_eq!(names(&backend), [FIRST, SECOND], "{}", lab.kind);
    }
}

#[test]
fn delete_refuses_a_symlinked_snapshot_or_snapshots_folder() {
    for lab in labs("delete-symlinked") {
        let kind = lab.kind;
        let backend = two_snapshots(&lab);
        // snapshots/<name> is a link to a real snapshot kept elsewhere.
        let elsewhere = lab.repo.parent().unwrap().join("elsewhere");
        fs::rename(snapshot_dir(&lab, FIRST), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, snapshot_dir(&lab, FIRST)).unwrap();
        let error = backend.delete(FIRST).unwrap_err();
        assert!(
            matches!(error, Error::InvalidInput(ref m) if m.contains("symlink")),
            "{kind}: {error:?}"
        );
        assert!(elsewhere.join("info.json").exists(), "{kind}");

        // snapshots/ itself is a link.
        let moved = lab.repo.join("timeshift/moved-snapshots");
        fs::rename(lab.repo.join("timeshift/snapshots"), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, lab.repo.join("timeshift/snapshots")).unwrap();
        let error = backend.delete(SECOND).unwrap_err();
        assert!(
            matches!(error, Error::InvalidInput(ref m) if m.contains("timeshift/snapshots/ is a symlink")),
            "{kind}: {error:?}"
        );
        assert!(moved.join(SECOND).join("info.json").exists(), "{kind}");
    }
}

#[test]
fn delete_refuses_a_folder_without_info_json() {
    for lab in labs("delete-no-info") {
        let kind = lab.kind;
        let backend = two_snapshots(&lab);
        fs::remove_file(snapshot_dir(&lab, FIRST).join("info.json")).unwrap();
        let error = backend.delete(FIRST).unwrap_err();
        assert!(
            matches!(error, Error::InvalidInput(ref m) if m.contains("no info.json")),
            "{kind}: {error:?}"
        );
        assert!(
            snapshot_dir(&lab, FIRST).join("localhost").exists(),
            "{kind}"
        );
        // A symlinked info.json doesn't count either.
        std::os::unix::fs::symlink(
            snapshot_dir(&lab, SECOND).join("info.json"),
            snapshot_dir(&lab, FIRST).join("info.json"),
        )
        .unwrap();
        assert!(
            matches!(backend.delete(FIRST), Err(Error::InvalidInput(_))),
            "{kind}"
        );
        assert!(snapshot_dir(&lab, FIRST).exists(), "{kind}");
    }
}

#[test]
fn delete_never_follows_symlinks_out_of_the_snapshot() {
    for lab in labs("delete-links-out") {
        let kind = lab.kind;
        let backend = two_snapshots(&lab);
        let outside = lab.repo.parent().unwrap().join("outside");
        fs::create_dir_all(outside.join("dir")).unwrap();
        fs::write(outside.join("dir/precious"), "keep").unwrap();
        let localhost = localhost(&lab, FIRST);
        std::os::unix::fs::symlink(outside.join("dir/precious"), localhost.join("to-file"))
            .unwrap();
        std::os::unix::fs::symlink(outside.join("dir"), localhost.join("to-dir")).unwrap();
        std::os::unix::fs::symlink("/", localhost.join("to-root")).unwrap();
        backend.delete(FIRST).unwrap();
        assert!(!snapshot_dir(&lab, FIRST).exists(), "{kind}");
        assert_eq!(
            fs::read_to_string(outside.join("dir/precious")).unwrap(),
            "keep",
            "{kind}"
        );
    }
}

#[test]
fn delete_refuses_a_snapshot_with_something_mounted_inside() {
    for lab in labs("delete-mounted") {
        let kind = lab.kind;
        populate(&lab.source);
        let inside = fs::canonicalize(&lab.repo)
            .unwrap()
            .join("timeshift/snapshots")
            .join(FIRST)
            .join("localhost/mnt");
        let line = format!(
            "99 1 7:0 / {} rw,relatime shared:9 - ext4 /dev/loop9 rw\n",
            inside.display()
        );
        let path = std::env::var_os("PATH").unwrap_or_default();
        let backend = NativeRsync::new(config(&lab, false), QuietRunner::new(path))
            .with_clock(clock())
            .with_mountinfo(move || Ok(line.clone()));
        backend.create("").unwrap();
        let error = backend.delete(FIRST).unwrap_err();
        assert!(
            matches!(error, Error::InvalidInput(ref m) if m.contains("mounted inside")),
            "{kind}: {error:?}"
        );
        // Refused before anything was deleted.
        assert!(localhost(&lab, FIRST).join("etc/same").exists(), "{kind}");
    }
}

#[test]
fn delete_removes_only_its_own_tag_links() {
    for lab in labs("delete-tag-links") {
        let kind = lab.kind;
        let backend = two_snapshots(&lab);
        let tag_dir = |tag: &str| lab.repo.join(format!("timeshift/snapshots-{tag}"));
        // A link Timeshift's schedule made, and something that isn't a link.
        std::os::unix::fs::symlink(
            format!("../snapshots/{FIRST}"),
            tag_dir("daily").join(FIRST),
        )
        .unwrap();
        fs::write(tag_dir("hourly").join(FIRST), "not a link").unwrap();
        backend.delete(FIRST).unwrap();
        for tag in ["ondemand", "daily"] {
            assert!(
                fs::symlink_metadata(tag_dir(tag).join(FIRST)).is_err(),
                "{kind}: {tag}"
            );
        }
        assert!(tag_dir("hourly").join(FIRST).is_file(), "{kind}");
        assert!(tag_dir("ondemand").join(SECOND).is_symlink(), "{kind}");
    }
}

/// A real bind mount and a real loop mount inside a snapshot, set up by the user with
/// `just nested-mount` (mounting needs root). Opt-in: `APSIS_NESTED_MOUNT` names the prepared
/// repository. Both mounted folders, and the snapshot, must survive the delete.
#[test]
fn delete_leaves_real_mounts_inside_alone() {
    let Some(repo) = std::env::var_os("APSIS_NESTED_MOUNT").map(PathBuf::from) else {
        eprintln!("nested mount test skipped; see `just nested-mount`");
        return;
    };
    let snapshot = repo.join("timeshift/snapshots").join(NESTED_NAME);
    for inside in ["bind", "loop"] {
        let point = snapshot.join("localhost").join(inside);
        let listed = Command::new("findmnt")
            .args(["--noheadings", "--output", "TARGET", "--mountpoint"])
            .arg(&point)
            .output()
            .unwrap();
        assert!(
            !listed.stdout.is_empty(),
            "{} isn't mounted; run `just nested-mount` first",
            point.display()
        );
    }
    // Regular files readable by the tester: the ext4 image's `lost+found` is root's.
    let readable = |dir: &Path| -> BTreeMap<PathBuf, (u64, u64)> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    out.insert(path.strip_prefix(dir).unwrap().to_owned(), inode(&path));
                }
            }
        }
        out
    };
    let before: Vec<_> = ["bind", "loop"]
        .iter()
        .map(|m| readable(&snapshot.join("localhost").join(m)))
        .collect();
    let config = NativeConfig {
        repo: repo.clone(),
        device: None,
        device_uuid: None,
        source: repo.clone(),
        sys_uuid: SYS_UUID.to_owned(),
        sys_distro: String::new(),
        exclude: Vec::new(),
        dry_run: false,
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let backend = NativeRsync::new(config, QuietRunner::new(path));
    let error = backend.delete(NESTED_NAME).unwrap_err();
    assert!(
        matches!(error, Error::InvalidInput(ref m) if m.contains("mounted inside")),
        "{error:?}"
    );
    assert!(snapshot.join("info.json").exists());
    let after: Vec<_> = ["bind", "loop"]
        .iter()
        .map(|m| readable(&snapshot.join("localhost").join(m)))
        .collect();
    assert_eq!(before, after);
    assert!(!before[0].is_empty() && !before[1].is_empty());

    // The walk alone (without the mount check) stops at the loop mount, another filesystem,
    // before deleting anything in it.
    let error = native::prune::remove_tree(&snapshot.join("localhost"), "loop").unwrap_err();
    assert!(error.to_string().contains("another filesystem"), "{error}");
    assert_eq!(readable(&snapshot.join("localhost/loop")), before[1]);
}

/// The snapshot name `just nested-mount` makes.
const NESTED_NAME: &str = "2026-01-01_00-00-00";

const LEFTOVER: &str = "2026-09-01_00-00-00";

#[test]
fn leftovers_are_listed_then_removed_by_the_next_create() {
    for lab in labs("staging-leftover") {
        let kind = lab.kind;
        populate(&lab.source);
        let staging = lab.repo.join("timeshift/apsis-staging");
        write(&staging.join(LEFTOVER).join("localhost/etc/hosts"), "half");
        // Not Apsis's: reported, never touched.
        write(&staging.join("notes.txt"), "mine");
        let (backend, log) = backend(&lab, false);
        let list = backend.list().unwrap();
        assert!(list.snapshots.is_empty(), "{kind}");
        assert_eq!(list.leftovers, [LEFTOVER], "{kind}");
        assert_eq!(list.warnings.len(), 1, "{kind}: {:?}", list.warnings);
        assert!(list.warnings[0].contains("notes.txt"), "{kind}");
        assert!(
            !list.warnings[0].contains('/'),
            "{kind}: no path in the pane"
        );

        backend.create("after a crash").unwrap();
        assert!(!staging.join(LEFTOVER).exists(), "{kind}");
        assert!(staging.join("notes.txt").exists(), "{kind}");
        let log = log.lock().unwrap().join("\n");
        assert!(
            log.contains("an interrupted create, started 2026-09-01 00:00:00"),
            "{kind}: {log}"
        );
        assert!(backend.list().unwrap().leftovers.is_empty(), "{kind}");
    }
}

#[test]
fn delete_removes_a_leftover_through_the_same_rules() {
    for lab in labs("staging-delete") {
        let kind = lab.kind;
        let staging = lab.repo.join("timeshift/apsis-staging");
        write(&staging.join(LEFTOVER).join("localhost/a"), "half");
        let outside = lab.repo.join("outside");
        write(&outside.join("keep"), "keep");
        std::os::unix::fs::symlink(&outside, staging.join(LEFTOVER).join("localhost/link"))
            .unwrap();
        let (backend, _) = backend(&lab, false);
        backend.delete(LEFTOVER).unwrap();
        assert!(
            !staging.exists(),
            "{kind}: the empty staging folder goes too"
        );
        assert!(
            outside.join("keep").exists(),
            "{kind}: symlinks aren't followed"
        );
        // Gone: now it's neither a leftover nor a snapshot.
        assert!(backend.delete(LEFTOVER).is_err(), "{kind}");
        assert!(
            matches!(backend.delete("../x"), Err(Error::InvalidSnapshotName(_))),
            "{kind}"
        );
    }
}

#[test]
fn a_symlinked_staging_folder_is_never_removed_through() {
    for lab in labs("staging-symlink") {
        let kind = lab.kind;
        let elsewhere = lab.repo.join("elsewhere");
        write(&elsewhere.join(LEFTOVER).join("x"), "x");
        fs::create_dir_all(lab.repo.join("timeshift")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, lab.repo.join("timeshift/apsis-staging")).unwrap();
        let (backend, _) = backend(&lab, false);
        let error = backend.remove_staging(LEFTOVER).unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)), "{kind}: {error:?}");
        assert!(elsewhere.join(LEFTOVER).join("x").exists(), "{kind}");
    }
}

#[test]
fn a_mount_inside_a_leftover_refuses_its_removal() {
    for lab in labs("staging-mount") {
        let kind = lab.kind;
        let leftover = lab.repo.join("timeshift/apsis-staging").join(LEFTOVER);
        write(&leftover.join("localhost/mnt/x"), "x");
        let mounted = fs::canonicalize(leftover.join("localhost/mnt")).unwrap();
        let line = format!(
            "36 35 98:0 / {} rw - ext4 /dev/sdz1 rw\n",
            mounted.display()
        );
        let (backend, _) = backend(&lab, false);
        let backend = backend.with_mountinfo(move || Ok(line.clone()));
        let error = backend.remove_staging(LEFTOVER).unwrap_err();
        assert!(
            error.to_string().contains("mounted inside"),
            "{kind}: {error}"
        );
        assert!(leftover.join("localhost/mnt/x").exists(), "{kind}");
    }
}

/// A folder holding one program, `rsync`: the shell `script`, for a runner's fixed `PATH`.
fn fake_rsync(lab: &Lab, script: &str) -> OsString {
    let bin = lab.repo.parent().unwrap().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let path = bin.join("rsync");
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let mut search = bin.into_os_string();
    search.push(":/usr/bin:/bin");
    search
}

/// Starts a create on another thread, waits until rsync is running, stops it; how long the
/// stop took and what the create returned.
fn stop_while_copying(lab: &Lab, script: &str, grace: Duration) -> (Duration, Error) {
    let path = fake_rsync(lab, script);
    let cancel = Cancel::with_grace(grace);
    let backend = NativeRsync::new(config(lab, false), QuietRunner::new(path))
        .with_clock(clock())
        .with_cancel(Arc::clone(&cancel));
    let staging = lab.repo.join("timeshift/apsis-staging");
    let create = std::thread::spawn(move || backend.create("stopped").unwrap_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !staging.join("2026-09-25_11-28-53/exclude.list").exists() {
        assert!(std::time::Instant::now() < deadline, "rsync never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    // rsync has been started by now (the list is written right before).
    std::thread::sleep(Duration::from_millis(300));
    let asked = std::time::Instant::now();
    cancel.request().unwrap();
    let error = create.join().unwrap();
    (asked.elapsed(), error)
}

#[test]
fn stop_ends_rsyncs_whole_group_and_removes_its_copy() {
    for lab in labs("stop-term") {
        let kind = lab.kind;
        populate(&lab.source);
        // A child in the same group, as rsync's receiver is: both must go.
        let (took, error) = stop_while_copying(&lab, "sleep 60 & wait", Duration::from_secs(30));
        assert!(matches!(error, Error::Stopped), "{kind}: {error:?}");
        assert!(
            took < Duration::from_secs(10),
            "{kind}: SIGTERM was enough ({took:?})"
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
fn stop_kills_what_ignores_sigterm_after_the_grace_period() {
    for lab in labs("stop-kill") {
        let kind = lab.kind;
        populate(&lab.source);
        let grace = Duration::from_millis(400);
        let script = "trap '' TERM\nwhile :; do sleep 0.1; done";
        let (took, error) = stop_while_copying(&lab, script, grace);
        assert!(matches!(error, Error::Stopped), "{kind}: {error:?}");
        assert!(took >= grace, "{kind}: ended before SIGKILL ({took:?})");
        assert!(took < Duration::from_secs(10), "{kind}: {took:?}");
        assert!(!lab.repo.join("timeshift/apsis-staging").exists(), "{kind}");
    }
}

#[test]
fn a_stop_before_rsync_starts_copies_nothing() {
    for lab in labs("stop-early") {
        let kind = lab.kind;
        populate(&lab.source);
        let cancel = Cancel::new();
        cancel.request().unwrap();
        let (backend, _) = backend(&lab, false);
        let backend = backend.with_cancel(cancel);
        assert!(matches!(backend.create("x"), Err(Error::Stopped)), "{kind}");
        assert!(!lab.repo.join("timeshift/apsis-staging").exists(), "{kind}");
    }
}

#[test]
fn a_create_names_its_snapshot_as_soon_as_its_planned() {
    for lab in labs("create-named") {
        populate(&lab.source);
        let seen: Log = Arc::default();
        let sink = Arc::clone(&seen);
        let (backend, _) = backend(&lab, false);
        let backend = backend.with_named(move |name| sink.lock().unwrap().push(name.to_owned()));
        backend.create("").unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            ["2026-09-25_11-28-53"],
            "{}",
            lab.kind
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
        let error = backend.delete(FIRST).unwrap_err();
        assert!(
            matches!(error, Error::InvalidInput(ref m) if m.contains("timeshift/ is a symlink")),
            "{kind}: {error:?}"
        );
    }
}
