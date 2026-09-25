// SPDX-License-Identifier: GPL-3.0-only

//! File-level restore against real files and a real rsync.
//!
//! Every scenario runs in a folder under `target/tmp/`, and again on a loop-mounted ext4 image
//! when `APSIS_EXT4_MNT` names its mount point (it must be ext4 on `/dev/loop*`). Nothing here
//! needs root: the "caller" is the tester, and the running system is a folder of the test's.

use std::ffi::OsString;
use std::fs::{self, File, FileTimes};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use apsis_core::restore::{
    Action, Caller, Destination, Kind, Live, Names, Request, Restore, RsyncRunner, SnapPath,
    browse, open_snapshot,
};
use apsis_core::{Error, RunOutput, Runner};

const SNAPSHOT: &str = "2026-09-25_03-00-01";

/// A snapshot repository, a running system and a home, all folders of the test's.
struct Lab {
    kind: &'static str,
    repo: PathBuf,
    live: PathBuf,
    home: PathBuf,
    dry_target: PathBuf,
}

impl Lab {
    fn localhost(&self) -> PathBuf {
        self.repo
            .join("timeshift/snapshots")
            .join(SNAPSHOT)
            .join("localhost")
    }

    fn caller(&self) -> Caller {
        Caller {
            uid: rustix::process::geteuid().as_raw(),
            gid: rustix::process::getegid().as_raw(),
            home: self.home.clone(),
        }
    }

    fn restore<'a, R: Runner>(&'a self, runner: &'a R) -> Restore<'a, R> {
        Restore {
            repo: &self.repo,
            live_root: &self.live,
            backup_dev: None,
            dry_run_target: &self.dry_target,
            runner,
        }
    }

    fn run(
        &self,
        paths: &[&str],
        destination: Destination,
        dry_run: bool,
    ) -> apsis_core::Result<apsis_core::restore::Plan> {
        let request = Request {
            snapshot: SNAPSHOT.to_owned(),
            paths: paths.iter().map(|p| (*p).to_owned()).collect(),
            destination,
            dry_run,
        };
        self.restore(&rsync()).run(&request, &self.caller())
    }

    fn restored(&self, n: &str) -> PathBuf {
        self.home.join("Apsis-restored").join(n)
    }
}

fn rsync() -> RsyncRunner {
    RsyncRunner::new(std::env::var_os("PATH").unwrap_or_default())
}

/// A fresh lab per place the tests run: `target/tmp/`, and the ext4 image if it's mounted.
fn labs(test: &str) -> Vec<Lab> {
    let mut bases = vec![(
        "plain",
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("restore"),
    )];
    if let Some(mount) = std::env::var_os("APSIS_EXT4_MNT") {
        let mount = PathBuf::from(mount);
        assert_loop_ext4(&mount);
        bases.push(("ext4 loop", mount.join("apsis-restore-tests")));
    }
    bases
        .into_iter()
        .map(|(kind, base)| {
            let root = base.join(test);
            match fs::remove_dir_all(&root) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => panic!("{e}"),
                _ => {}
            }
            fs::create_dir_all(&root).unwrap();
            // The destination is opened component by component with O_NOFOLLOW: no symlinks
            // in the lab's own path.
            let root = root.canonicalize().unwrap();
            let lab = Lab {
                kind,
                repo: root.join("repo"),
                live: root.join("live"),
                home: root.join("home"),
                dry_target: root.join("dry-run-target"),
            };
            fs::create_dir_all(lab.localhost()).unwrap();
            fs::create_dir_all(&lab.live).unwrap();
            fs::create_dir_all(&lab.home).unwrap();
            lab
        })
        .collect()
}

/// `APSIS_EXT4_MNT` really is an ext4 filesystem on a loop device (see `tests/native.rs`).
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

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn set_mtime(path: &Path, secs: u64) {
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(time))
        .unwrap();
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn mkfifo(path: &Path) {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
}

/// `/etc` in the snapshot: files, a setuid file, a folder, a symlink, a FIFO.
fn small_etc(lab: &Lab) {
    let etc = lab.localhost().join("etc");
    write(&etc.join("hosts"), "127.0.0.1 snapshot\n");
    write(&etc.join("fstab"), "# fstab\n");
    write(&etc.join("nm/a.conf"), "a\n");
    fs::set_permissions(etc.join("fstab"), fs::Permissions::from_mode(0o4755)).unwrap();
    symlink("../usr/bin/vim.basic", etc.join("vi")).unwrap();
    mkfifo(&etc.join("nm/fifo"));
}

fn refused<T: std::fmt::Debug>(result: apsis_core::Result<T>) -> String {
    match result {
        Err(Error::InvalidInput(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn folder_mode_copies_into_a_new_folder_as_the_caller() {
    for lab in labs("folder") {
        small_etc(&lab);
        let plan = lab
            .run(
                &["/etc/hosts", "/etc/fstab", "/etc/nm", "/etc/vi"],
                Destination::Folder,
                false,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", lab.kind));
        let first = lab.restored(SNAPSHOT);
        assert_eq!(plan.target, first.display().to_string());
        assert_eq!(
            fs::read_to_string(first.join("etc/hosts")).unwrap(),
            "127.0.0.1 snapshot\n"
        );
        assert_eq!(
            fs::read_to_string(first.join("etc/nm/a.conf")).unwrap(),
            "a\n"
        );
        // Symlinks stay symlinks, with the same text.
        assert_eq!(
            fs::read_link(first.join("etc/vi")).unwrap(),
            Path::new("../usr/bin/vim.basic")
        );
        // No setuid bit on a caller-owned copy.
        assert_eq!(mode(&first.join("etc/fstab")), 0o755, "{}", lab.kind);
        // The folder is handed over.
        assert_eq!(mode(&first), 0o755);
        assert_eq!(fs::metadata(&first).unwrap().uid(), lab.caller().uid);
        assert!(
            plan.items
                .iter()
                .any(|i| i.action == Action::Link && i.path == "/etc/vi")
        );

        // A second restore never touches the first: it gets its own folder.
        fs::write(first.join("etc/hosts"), "edited by the user\n").unwrap();
        lab.run(&["/etc/hosts"], Destination::Folder, false)
            .unwrap();
        assert_eq!(
            fs::read_to_string(first.join("etc/hosts")).unwrap(),
            "edited by the user\n"
        );
        let second = lab.restored(&format!("{SNAPSHOT}-2"));
        assert_eq!(
            fs::read_to_string(second.join("etc/hosts")).unwrap(),
            "127.0.0.1 snapshot\n"
        );
    }
}

#[test]
fn folder_mode_never_creates_fifos_or_device_nodes() {
    for lab in labs("specials") {
        small_etc(&lab);
        assert!(
            fs::symlink_metadata(lab.localhost().join("etc/nm/fifo"))
                .unwrap()
                .file_type()
                .is_fifo()
        );
        let plan = lab.run(&["/etc/nm"], Destination::Folder, false).unwrap();
        let restored = lab.restored(SNAPSHOT).join("etc/nm");
        assert!(restored.join("a.conf").exists());
        assert!(
            fs::symlink_metadata(restored.join("fifo")).is_err(),
            "{}: a FIFO was created",
            lab.kind
        );
        assert!(
            plan.notes.iter().any(|n| n.contains("fifo")),
            "{:?}",
            plan.notes
        );
        // Named on its own, too.
        lab.run(&["/etc/nm/fifo"], Destination::Folder, false)
            .unwrap();
        assert!(
            fs::symlink_metadata(lab.restored(&format!("{SNAPSHOT}-2")).join("etc/nm/fifo"))
                .is_err()
        );
    }
}

#[test]
fn a_dry_run_writes_nothing() {
    for lab in labs("dry-run") {
        small_etc(&lab);
        write(&lab.live.join("etc/hosts"), "live\n");
        let plan = lab
            .run(&["/etc/hosts", "/etc/nm"], Destination::Folder, true)
            .unwrap();
        assert!(plan.dry_run);
        assert!(!lab.home.join("Apsis-restored").exists(), "{}", lab.kind);
        assert!(!lab.dry_target.exists());
        let paths: Vec<_> = plan.items.iter().map(|i| i.path.as_str()).collect();
        assert!(
            paths.contains(&"/etc/hosts") && paths.contains(&"/etc/nm/a.conf"),
            "{paths:?}"
        );
        assert!(plan.commands[0].contains(&"--dry-run".to_owned()));

        let plan = lab
            .run(&["/etc/hosts"], Destination::Original, true)
            .unwrap();
        assert_eq!(
            fs::read_to_string(lab.live.join("etc/hosts")).unwrap(),
            "live\n"
        );
        assert!(
            !lab.live
                .join(format!("etc/hosts.apsis-before-{SNAPSHOT}"))
                .exists()
        );
        assert_eq!(plan.replaced(), 1);
        assert_eq!(
            plan.items[0].backup.as_deref(),
            Some(format!("/etc/hosts.apsis-before-{SNAPSHOT}").as_str())
        );
        assert!(
            plan.to_string()
                .contains("warning: /etc/hosts: /etc is live system files")
        );
    }
}

/// A snapshot symlink to a file outside the destination, restored in folder mode: the target
/// keeps its owner, mode and times (nothing is chowned or chmodded through the link).
#[test]
fn folder_mode_hand_over_never_follows_a_symlink() {
    for lab in labs("hand-over") {
        let outside = lab.home.parent().unwrap().join("outside-target");
        write(&outside, "not yours\n");
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        set_mtime(&outside, 1_000_000_000);
        let before = fs::metadata(&outside).unwrap();
        let etc = lab.localhost().join("etc");
        fs::create_dir_all(&etc).unwrap();
        symlink(&outside, etc.join("evil")).unwrap();
        // And a link to a root-owned system file, when there is one to point at.
        let system = Path::new("/etc/hostname");
        let system_before = fs::metadata(system).ok();
        symlink(system, etc.join("hostname-link")).unwrap();

        lab.run(&["/etc"], Destination::Folder, false).unwrap();
        lab.run(&["/etc/evil"], Destination::Folder, false).unwrap();
        let after = fs::metadata(&outside).unwrap();
        assert_eq!(
            (after.uid(), after.gid(), after.mode(), after.mtime()),
            (before.uid(), before.gid(), before.mode(), before.mtime()),
            "{}",
            lab.kind
        );
        assert_eq!(fs::read_to_string(&outside).unwrap(), "not yours\n");
        if let Some(system_before) = system_before {
            let system_after = fs::metadata(system).unwrap();
            assert_eq!(
                (system_after.uid(), system_after.mode()),
                (system_before.uid(), system_before.mode())
            );
        }
        let restored = lab.restored(SNAPSHOT).join("etc/evil");
        assert!(
            fs::symlink_metadata(&restored)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&restored).unwrap(), outside);
    }
}

/// Swaps `~/Apsis-restored` for a symlink to a decoy right before rsync starts, as a user
/// racing root would. rsync must still write into the folder made for it.
struct SwapBeforeRun<'a> {
    home: &'a Path,
    decoy: &'a Path,
}

impl Runner for SwapBeforeRun<'_> {
    fn run(&self, argv: &[OsString]) -> io::Result<RunOutput> {
        let restored = self.home.join("Apsis-restored");
        fs::rename(&restored, self.home.join("moved-away"))?;
        symlink(self.decoy, &restored)?;
        rsync().run(argv)
    }
}

#[test]
fn rsync_writes_through_the_held_folder_not_the_path() {
    for lab in labs("pinned") {
        small_etc(&lab);
        let decoy = lab.home.parent().unwrap().join("decoy");
        fs::create_dir_all(&decoy).unwrap();
        let runner = SwapBeforeRun {
            home: &lab.home,
            decoy: &decoy,
        };
        let request = Request {
            snapshot: SNAPSHOT.to_owned(),
            paths: vec!["/etc/hosts".to_owned()],
            destination: Destination::Folder,
            dry_run: false,
        };
        let plan = lab.restore(&runner).run(&request, &lab.caller()).unwrap();
        assert!(
            plan.commands[0]
                .last()
                .unwrap()
                .starts_with("/proc/self/fd/")
        );
        assert_eq!(
            fs::read_dir(&decoy).unwrap().count(),
            0,
            "{}: wrote through the swapped path",
            lab.kind
        );
        let landed = lab.home.join("moved-away").join(SNAPSHOT).join("etc/hosts");
        assert_eq!(fs::read_to_string(landed).unwrap(), "127.0.0.1 snapshot\n");
    }
}

#[test]
fn original_mode_backs_up_what_it_replaces() {
    for lab in labs("original") {
        small_etc(&lab);
        let hosts = lab.localhost().join("etc/hosts");
        set_mtime(&hosts, 1_700_000_000);
        write(&lab.live.join("etc/hosts"), "127.0.0.1 live, changed\n");
        // Identical: same content, size and mtime.
        write(&lab.live.join("etc/nm/a.conf"), "a\n");
        set_mtime(&lab.localhost().join("etc/nm/a.conf"), 1_600_000_000);
        set_mtime(&lab.live.join("etc/nm/a.conf"), 1_600_000_000);

        let plan = lab
            .run(
                &["/etc/hosts", "/etc/nm/a.conf"],
                Destination::Original,
                false,
            )
            .unwrap();
        let backup = lab.live.join(format!("etc/hosts.apsis-before-{SNAPSHOT}"));
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "127.0.0.1 live, changed\n",
            "{}",
            lab.kind
        );
        assert_eq!(
            fs::read_to_string(lab.live.join("etc/hosts")).unwrap(),
            "127.0.0.1 snapshot\n"
        );
        assert_eq!(
            fs::metadata(lab.live.join("etc/hosts")).unwrap().mtime(),
            1_700_000_000
        );
        // The identical file was skipped, so nothing backed it up.
        assert!(
            !lab.live
                .join(format!("etc/nm/a.conf.apsis-before-{SNAPSHOT}"))
                .exists()
        );
        assert_eq!(plan.replaced(), 1);

        // The file changed again: a second restore would overwrite that backup. Refused.
        write(&lab.live.join("etc/hosts"), "changed again\n");
        let reason = refused(lab.run(&["/etc/hosts"], Destination::Original, false));
        assert!(reason.contains("in the way"), "{reason}");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "127.0.0.1 live, changed\n"
        );
        assert_eq!(
            fs::read_to_string(lab.live.join("etc/hosts")).unwrap(),
            "changed again\n"
        );

        // A folder that's gone on the running system comes back whole (FIFO included: original
        // mode puts back what was there).
        fs::remove_dir_all(lab.live.join("etc/nm")).unwrap();
        lab.run(&["/etc/nm"], Destination::Original, false).unwrap();
        assert_eq!(
            fs::read_to_string(lab.live.join("etc/nm/a.conf")).unwrap(),
            "a\n"
        );
        assert!(
            fs::symlink_metadata(lab.live.join("etc/nm/fifo"))
                .unwrap()
                .file_type()
                .is_fifo()
        );
    }
}

#[test]
fn original_mode_refuses_what_it_must_not_touch() {
    for lab in labs("original-refused") {
        small_etc(&lab);
        write(&lab.localhost().join("proc/x"), "x");
        write(&lab.localhost().join("boot/vmlinuz"), "k");
        write(&lab.localhost().join("opt/app/conf"), "c");
        write(&lab.localhost().join("var/run/x"), "x");
        for path in ["/proc/x", "/boot/vmlinuz"] {
            let reason = refused(lab.run(&[path], Destination::Original, true));
            assert!(reason.contains("never restored"), "{reason}");
        }
        // The live parent is missing.
        let reason = refused(lab.run(&["/opt/app/conf"], Destination::Original, true));
        assert!(
            reason.contains("doesn't exist in the running system; restore the folder above it"),
            "{reason}"
        );
        // The live parent is a symlink (like /var/run -> /run).
        fs::create_dir_all(lab.live.join("run")).unwrap();
        fs::create_dir_all(lab.live.join("var")).unwrap();
        symlink("../run", lab.live.join("var/run")).unwrap();
        let reason = refused(lab.run(&["/var/run/x"], Destination::Original, false));
        assert!(reason.contains("symlink"), "{reason}");
        assert!(!lab.live.join("run/x").exists());
        // On the backup device's own filesystem.
        fs::create_dir_all(lab.live.join("etc")).unwrap();
        let dev = fs::metadata(&lab.live).unwrap().dev();
        let runner = rsync();
        let restore = Restore {
            backup_dev: Some(dev),
            ..lab.restore(&runner)
        };
        let request = Request {
            snapshot: SNAPSHOT.to_owned(),
            paths: vec!["/etc/hosts".to_owned()],
            destination: Destination::Original,
            dry_run: true,
        };
        let reason = refused(restore.run(&request, &lab.caller()));
        assert!(reason.contains("backup device"), "{reason}");
    }
}

#[test]
fn paths_never_escape_the_snapshot() {
    for lab in labs("escape") {
        small_etc(&lab);
        let outside = lab.home.parent().unwrap().join("secret");
        write(&outside.join("shadow"), "secret\n");
        let etc = lab.localhost().join("etc");
        // A folder symlink in the middle of a path: never followed.
        symlink(&outside, etc.join("linked-dir")).unwrap();
        symlink("../../../../../../../../secret", etc.join("up-dir")).unwrap();
        // A final symlink whose relative target climbs out.
        symlink(
            "../../../../../../../../../../secret/shadow",
            etc.join("escape"),
        )
        .unwrap();
        for (path, why) in [
            ("/etc/linked-dir/shadow", "symlink"),
            ("/etc/up-dir/shadow", "symlink"),
            ("/etc/escape", "outside the snapshot"),
            ("/etc/../../secret/shadow", "`..`"),
            ("/etc/./hosts", "`..`"),
            ("etc/hosts", "absolute"),
            ("/etc/missing", "isn't in the snapshot"),
        ] {
            for destination in [Destination::Folder, Destination::Original] {
                let reason = refused(lab.run(&[path], destination, true));
                assert!(reason.contains(why), "{path}: {reason}");
            }
        }
        assert!(!lab.home.join("Apsis-restored").exists());
        // Browsing into a symlink or through `..` is refused too.
        let root = open_snapshot(&lab.repo, SNAPSHOT).unwrap();
        let names = Names::default();
        for path in ["/etc/linked-dir", "/etc/up-dir"] {
            let path = SnapPath::parse(path).unwrap();
            let reason = refused(browse(&root, &path, &lab.live, &names));
            assert!(reason.contains("symlink"), "{reason}");
        }
        assert!(SnapPath::parse("/etc/..").is_err());
    }
}

#[test]
fn snapshots_are_found_only_by_name_and_without_symlinks() {
    for lab in labs("snapshot-name") {
        small_etc(&lab);
        assert!(matches!(
            open_snapshot(&lab.repo, "../../etc"),
            Err(Error::InvalidSnapshotName(_))
        ));
        assert!(matches!(
            open_snapshot(&lab.repo, "2001-01-01_00-00-00"),
            Err(Error::NoSuchSnapshot(_))
        ));
        let snapshots = lab.repo.join("timeshift/snapshots");
        symlink(
            snapshots.join(SNAPSHOT),
            snapshots.join("2026-09-26_00-00-00"),
        )
        .unwrap();
        let reason = refused(open_snapshot(&lab.repo, "2026-09-26_00-00-00"));
        assert!(reason.contains("symlink"), "{reason}");
    }
}

#[test]
fn browse_compares_each_entry_with_the_running_system() {
    for lab in labs("browse") {
        small_etc(&lab);
        let snap_etc = lab.localhost().join("etc");
        let live_etc = lab.live.join("etc");
        write(&live_etc.join("hosts"), "127.0.0.1 snapshot\n");
        set_mtime(&snap_etc.join("hosts"), 1_600_000_000);
        set_mtime(&live_etc.join("hosts"), 1_600_000_000);
        write(&live_etc.join("fstab"), "# changed fstab\n");
        fs::create_dir_all(live_etc.join("nm")).unwrap();
        symlink("elsewhere", live_etc.join("vi")).unwrap();

        let root = open_snapshot(&lab.repo, SNAPSHOT).unwrap();
        let listing = browse(
            &root,
            &SnapPath::parse("/etc").unwrap(),
            &lab.live,
            &Names::default(),
        )
        .unwrap();
        let got: Vec<_> = listing
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind, e.live))
            .collect();
        assert_eq!(
            got,
            [
                ("fstab", Kind::File, Live::Changed),
                ("hosts", Kind::File, Live::Same),
                ("nm", Kind::Dir, Live::Present),
                ("vi", Kind::Link, Live::Changed),
            ],
            "{}",
            lab.kind
        );
        let vi = &listing.entries[3];
        assert_eq!(vi.target, "../usr/bin/vim.basic");
        assert!(!listing.truncated);
        let nm = browse(
            &root,
            &SnapPath::parse("/etc/nm").unwrap(),
            &lab.live,
            &Names::default(),
        )
        .unwrap();
        let got: Vec<_> = nm
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind, e.live))
            .collect();
        assert_eq!(
            got,
            [
                ("a.conf", Kind::File, Live::Missing),
                ("fifo", Kind::Other, Live::Missing)
            ]
        );
        // `/` lists the snapshot's root; a file isn't a folder.
        let top = browse(&root, &SnapPath::root(), &lab.live, &Names::default()).unwrap();
        assert_eq!(top.entries[0].name, "etc");
        let reason = refused(browse(
            &root,
            &SnapPath::parse("/etc/hosts").unwrap(),
            &lab.live,
            &Names::default(),
        ));
        assert!(reason.contains("not a folder"), "{reason}");
    }
}
