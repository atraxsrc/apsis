// SPDX-License-Identifier: GPL-3.0-only

//! `request.json`: the plan of a prepared restore (PLAN 6b.9).
//!
//! Written when the preparation ends ("ready"), read at "Restart now" and by the apply in
//! the next boot. Format and rules: [`super::file`].

use std::path::Path;

use serde_json::{Map, Value, json};

use super::file::{self, FileError};
use super::filter::{Home, is_kernel_version};

pub const FILE: &str = "request.json";

/// How long a plan may wait on the ready prompt (PLAN 6b.5): 30 minutes.
pub const MAX_AGE_SECS: i64 = 30 * 60;

/// How far ahead of the clock `prepared_at` may be before the plan's age counts as unknown.
pub const MAX_FUTURE_SECS: i64 = 2 * 60;

/// `RestartToRestore`'s refusal (as `InvalidInput`) for a plan older than [`MAX_AGE_SECS`]
/// (PLAN 6b.5): the window says "The preparation is too old. Start the restore again."
pub const TOO_OLD: &str = "the preparation is too old";
/// `RestartToRestore`'s and `CancelRestore`'s refusal (as `InvalidInput`) when the helper has
/// no plan for the snapshot (PLAN 6b.9): "The preparation is gone. Start the restore again."
pub const GONE: &str = "the preparation is gone";

const KEYS: [&str; 12] = [
    "snapshot",
    "snapshot_created",
    "backup_uuid",
    "home",
    "old_format",
    "safety_snapshot",
    "root_uuid",
    "running_kernel",
    "root_needs",
    "separate_home",
    "starter_uid",
    "prepared_at",
];

/// Everything the restart check and the apply need to know about a prepared restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The snapshot to restore.
    pub snapshot: String,
    /// Its `info.json`'s `created` (Unix seconds) when the plan was made. `info.json` has no
    /// name in it, so this is what tells the apply that the snapshot at that name is still
    /// the same one ([`super::apply::check_snapshot`]).
    pub snapshot_created: i64,
    /// The backup disk's filesystem UUID.
    pub backup_uuid: String,
    pub home: Home,
    /// The snapshot was made without ACLs and extended attributes (`Info::is_old_format`).
    pub old_format: bool,
    /// The safety snapshot taken while preparing, if it was asked for.
    pub safety_snapshot: Option<String>,
    /// The UUID of the filesystem mounted at `/` when the plan was made.
    pub root_uuid: String,
    /// `uname -r` when the plan was made: the kernel the ESP boots.
    pub running_kernel: String,
    /// What `/` must have free, in bytes, margin included (`space::system_needs`).
    pub root_needs: u64,
    /// `/home`, when it's restored and is its own mount. `None`: kept, or on `/`.
    pub separate_home: Option<SeparateHome>,
    /// Who started the restore; they may restart or cancel without the password again.
    pub starter_uid: u32,
    /// When the preparation ended, in Unix seconds.
    pub prepared_at: i64,
}

/// A `/home` on its own filesystem that the restore writes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparateHome {
    /// Its filesystem UUID; it must be mounted at `/home` with it at apply time.
    pub uuid: String,
    /// What it must have free, in bytes, margin included.
    pub needs: u64,
}

impl Plan {
    /// # Errors
    ///
    /// [`FileError::Invalid`] if `text` isn't a plan this Apsis wrote, whole and in range.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let map = file::object(text, &KEYS)?;
        let starter_uid = map
            .get("starter_uid")
            .and_then(Value::as_u64)
            .and_then(|uid| u32::try_from(uid).ok())
            .ok_or_else(|| FileError::Invalid("\"starter_uid\" isn't a user id".to_owned()))?;
        let plan = Self {
            snapshot: file::text(&map, "snapshot")?.to_owned(),
            snapshot_created: file::time(&map, "snapshot_created")?,
            backup_uuid: file::text(&map, "backup_uuid")?.to_owned(),
            home: file::home(&map)?,
            old_format: file::flag(&map, "old_format")?,
            safety_snapshot: file::optional_text(&map, "safety_snapshot")?.map(str::to_owned),
            root_uuid: file::text(&map, "root_uuid")?.to_owned(),
            running_kernel: file::text(&map, "running_kernel")?.to_owned(),
            root_needs: file::bytes(&map, "root_needs")?,
            separate_home: separate_home(&map)?,
            starter_uid,
            prepared_at: file::time(&map, "prepared_at")?,
        };
        plan.validate()?;
        Ok(plan)
    }

    /// The file's text.
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] if the plan is one [`Plan::parse`] would refuse.
    pub fn to_text(&self) -> Result<String, FileError> {
        self.validate()?;
        let separate_home = self.separate_home.as_ref().map_or(
            Value::Null,
            |home| json!({"uuid": home.uuid, "needs": home.needs}),
        );
        Ok(file::to_text([
            ("snapshot", self.snapshot.as_str().into()),
            ("snapshot_created", self.snapshot_created.into()),
            ("backup_uuid", self.backup_uuid.as_str().into()),
            ("home", file::home_word(self.home).into()),
            ("old_format", self.old_format.into()),
            ("safety_snapshot", self.safety_snapshot.as_deref().into()),
            ("root_uuid", self.root_uuid.as_str().into()),
            ("running_kernel", self.running_kernel.as_str().into()),
            ("root_needs", self.root_needs.into()),
            ("separate_home", separate_home),
            ("starter_uid", self.starter_uid.into()),
            ("prepared_at", self.prepared_at.into()),
        ]))
    }

    /// Reads `dir/request.json`.
    ///
    /// # Errors
    ///
    /// [`FileError::Io`] if it's missing or unreadable, [`FileError::Invalid`] as for
    /// [`Plan::parse`].
    pub fn load(dir: &Path) -> Result<Self, FileError> {
        Self::parse(&file::load(dir, FILE)?).map_err(|error| error.within(FILE))
    }

    /// Writes `dir/request.json` in one step (temporary file, fsync, rename).
    ///
    /// # Errors
    ///
    /// [`FileError::Invalid`] as for [`Plan::to_text`], with nothing written;
    /// [`FileError::Io`] if the write fails, with the file as it was.
    pub fn save(&self, dir: &Path) -> Result<(), FileError> {
        file::save(dir, FILE, &self.to_text()?)
    }

    /// Whether the plan can't be used at `now` (Unix seconds): it has waited more than
    /// [`MAX_AGE_SECS`], or it was made more than [`MAX_FUTURE_SECS`] after `now` (the clock
    /// was set back, so its age is unknown). A little ahead is a corrected clock, and fine.
    #[must_use]
    pub fn is_too_old(&self, now: i64) -> bool {
        let age = now.saturating_sub(self.prepared_at);
        !(-MAX_FUTURE_SECS..=MAX_AGE_SECS).contains(&age)
    }

    fn validate(&self) -> Result<(), FileError> {
        file::check_snapshot("snapshot", &self.snapshot)?;
        file::check_time("snapshot_created", self.snapshot_created)?;
        file::check_uuid("backup_uuid", &self.backup_uuid)?;
        if let Some(safety) = &self.safety_snapshot {
            file::check_snapshot("safety_snapshot", safety)?;
        }
        file::check_uuid("root_uuid", &self.root_uuid)?;
        if !is_kernel_version(&self.running_kernel) {
            return Err(invalid("\"running_kernel\" isn't a kernel version"));
        }
        if let Some(home) = &self.separate_home {
            file::check_uuid("uuid", &home.uuid).map_err(|error| error.within(SEPARATE_HOME))?;
        }
        file::check_time("prepared_at", self.prepared_at)?;

        if self.safety_snapshot.as_ref() == Some(&self.snapshot) {
            return Err(invalid("the safety snapshot is the snapshot to restore"));
        }
        if let Some(home) = &self.separate_home {
            if self.home == Home::Keep {
                return Err(invalid("a separate /home is named but home is kept"));
            }
            if home.uuid == self.root_uuid {
                return Err(invalid("the separate /home is the root filesystem"));
            }
        }
        Ok(())
    }
}

/// In front of what's wrong inside the `separate_home` object.
const SEPARATE_HOME: &str = "\"separate_home\"";

fn invalid(reason: &str) -> FileError {
    FileError::Invalid(reason.to_owned())
}

fn separate_home(map: &Map<String, Value>) -> Result<Option<SeparateHome>, FileError> {
    let home = match map.get("separate_home") {
        Some(Value::Null) => return Ok(None),
        Some(Value::Object(home)) => home,
        _ => return Err(invalid("\"separate_home\" isn't an object or null")),
    };
    (|| {
        file::fields(home, &["uuid", "needs"])?;
        Ok(Some(SeparateHome {
            uuid: file::text(home, "uuid")?.to_owned(),
            needs: file::bytes(home, "needs")?,
        }))
    })()
    .map_err(|error: FileError| error.within(SEPARATE_HOME))
}

#[cfg(test)]
mod tests {
    use super::super::file::tests::temp_dir;
    use super::*;

    const SNAPSHOT: &str = "2026-09-25_11-28-00";
    const SAFETY: &str = "2026-10-01_09-15-42";
    const ROOT: &str = "11111111-2222-3333-4444-555555555555";
    const BACKUP: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    const HOME: &str = "99999999-8888-7777-6666-555555555555";
    const PREPARED: i64 = 1_790_000_000;

    fn plan() -> Plan {
        Plan {
            snapshot: SNAPSHOT.to_owned(),
            snapshot_created: 1_789_990_080,
            backup_uuid: BACKUP.to_owned(),
            home: Home::Keep,
            old_format: false,
            safety_snapshot: Some(SAFETY.to_owned()),
            root_uuid: ROOT.to_owned(),
            running_kernel: "6.9.3-76060903-generic".to_owned(),
            root_needs: 7_073_741_824,
            separate_home: None,
            starter_uid: 1000,
            prepared_at: PREPARED,
        }
    }

    fn with_separate_home() -> Plan {
        Plan {
            home: Home::Restore,
            separate_home: Some(SeparateHome {
                uuid: HOME.to_owned(),
                needs: 2_073_741_824,
            }),
            ..plan()
        }
    }

    const TEXT: &str = r#"{
  "version": 1,
  "snapshot": "2026-09-25_11-28-00",
  "snapshot_created": 1789990080,
  "backup_uuid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
  "home": "keep",
  "old_format": false,
  "safety_snapshot": "2026-10-01_09-15-42",
  "root_uuid": "11111111-2222-3333-4444-555555555555",
  "running_kernel": "6.9.3-76060903-generic",
  "root_needs": 7073741824,
  "separate_home": null,
  "starter_uid": 1000,
  "prepared_at": 1790000000
}
"#;

    /// The snapshot's `created` from its `info.json`, so the apply can tell it's the same
    /// snapshot at that name.
    #[test]
    fn the_snapshots_creation_time_must_be_a_time() {
        for value in ["0", "-5", "\"1789990080\"", "null"] {
            assert_eq!(
                invalid(Plan::parse(&text_with("snapshot_created", value))),
                "\"snapshot_created\" isn't a time",
                "{value}"
            );
        }
    }

    fn invalid(result: Result<Plan, FileError>) -> String {
        match result {
            Err(FileError::Invalid(reason)) => reason,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    /// `TEXT` with one field's line replaced.
    fn text_with(field: &str, value: &str) -> String {
        let mut found = false;
        let lines: Vec<String> = TEXT
            .lines()
            .map(|line| {
                if line.starts_with(&format!("  \"{field}\":")) {
                    found = true;
                    let comma = if line.ends_with(',') { "," } else { "" };
                    format!("  \"{field}\": {value}{comma}")
                } else {
                    line.to_owned()
                }
            })
            .collect();
        assert!(found, "{field}");
        lines.join("\n")
    }

    #[test]
    fn the_plan_is_written_field_by_field() {
        assert_eq!(plan().to_text().unwrap(), TEXT);
    }

    #[test]
    fn the_text_reads_back_as_the_plan() {
        assert_eq!(Plan::parse(TEXT).unwrap(), plan());
    }

    #[test]
    fn a_plan_without_a_safety_snapshot_round_trips() {
        let plan = Plan {
            safety_snapshot: None,
            ..plan()
        };
        let text = plan.to_text().unwrap();
        assert!(text.contains("\"safety_snapshot\": null"), "{text}");
        assert_eq!(Plan::parse(&text).unwrap(), plan);
    }

    #[test]
    fn a_separate_home_is_written_with_its_uuid_and_its_needs() {
        let text = with_separate_home().to_text().unwrap();
        assert!(text.contains("\"home\": \"restore\""), "{text}");
        assert!(
            text.contains(
                "\"separate_home\": {\n    \"uuid\": \"99999999-8888-7777-6666-555555555555\",\n    \"needs\": 2073741824\n  }"
            ),
            "{text}"
        );
        assert_eq!(Plan::parse(&text).unwrap(), with_separate_home());
    }

    #[test]
    fn an_old_format_plan_round_trips() {
        let plan = Plan {
            old_format: true,
            ..plan()
        };
        assert_eq!(Plan::parse(&plan.to_text().unwrap()).unwrap(), plan);
    }

    /// Sizes are whole byte counts up to `u64::MAX`, not floats.
    #[test]
    fn the_largest_size_round_trips() {
        let plan = Plan {
            root_needs: u64::MAX,
            ..plan()
        };
        assert_eq!(Plan::parse(&plan.to_text().unwrap()).unwrap(), plan);
    }

    #[test]
    fn a_field_of_the_wrong_kind_or_range_is_refused() {
        for (field, value, reason) in [
            (
                "snapshot",
                "\"2026-09-25\"",
                "\"snapshot\" isn't a snapshot name",
            ),
            (
                "snapshot",
                "\"../../etc/passwd_00-00\"",
                "\"snapshot\" isn't a snapshot name",
            ),
            ("snapshot", "7", "\"snapshot\" isn't text"),
            ("backup_uuid", "\"\"", "\"backup_uuid\" isn't a UUID"),
            ("backup_uuid", "\"../sda1\"", "\"backup_uuid\" isn't a UUID"),
            (
                "home",
                "\"both\"",
                "\"home\" is neither \"keep\" nor \"restore\"",
            ),
            ("home", "true", "\"home\" isn't text"),
            (
                "old_format",
                "\"false\"",
                "\"old_format\" isn't true or false",
            ),
            ("old_format", "0", "\"old_format\" isn't true or false"),
            (
                "safety_snapshot",
                "\"yesterday\"",
                "\"safety_snapshot\" isn't a snapshot name",
            ),
            ("safety_snapshot", "false", "\"safety_snapshot\" isn't text"),
            ("root_uuid", "null", "\"root_uuid\" isn't text"),
            ("root_uuid", "\"a b\"", "\"root_uuid\" isn't a UUID"),
            (
                "running_kernel",
                "\"\"",
                "\"running_kernel\" isn't a kernel version",
            ),
            (
                "running_kernel",
                "\"6.9/../x\"",
                "\"running_kernel\" isn't a kernel version",
            ),
            (
                "root_needs",
                "-1",
                "\"root_needs\" isn't a whole number of bytes",
            ),
            (
                "root_needs",
                "1.5",
                "\"root_needs\" isn't a whole number of bytes",
            ),
            (
                "root_needs",
                "\"12\"",
                "\"root_needs\" isn't a whole number of bytes",
            ),
            (
                "root_needs",
                "18446744073709551616",
                "\"root_needs\" isn't a whole number of bytes",
            ),
            (
                "separate_home",
                "\"none\"",
                "\"separate_home\" isn't an object or null",
            ),
            ("starter_uid", "-1", "\"starter_uid\" isn't a user id"),
            (
                "starter_uid",
                "4294967296",
                "\"starter_uid\" isn't a user id",
            ),
            ("starter_uid", "\"root\"", "\"starter_uid\" isn't a user id"),
            ("prepared_at", "0", "\"prepared_at\" isn't a time"),
            ("prepared_at", "-5", "\"prepared_at\" isn't a time"),
            ("prepared_at", "\"now\"", "\"prepared_at\" isn't a time"),
        ] {
            assert_eq!(
                invalid(Plan::parse(&text_with(field, value))),
                reason,
                "{field}: {value}"
            );
        }
    }

    #[test]
    fn a_plan_of_another_version_is_refused() {
        assert_eq!(
            invalid(Plan::parse(&text_with("version", "2"))),
            "version 2 (this Apsis reads 1)"
        );
    }

    #[test]
    fn a_plan_with_a_field_missing_or_added_is_refused() {
        let without = TEXT.replace("  \"old_format\": false,\n", "");
        assert_eq!(invalid(Plan::parse(&without)), "no \"old_format\"");
        let added = TEXT.replace("\"old_format\"", "\"delete_home\": true,\n  \"old_format\"");
        assert_eq!(
            invalid(Plan::parse(&added)),
            "unknown field \"delete_home\""
        );
    }

    /// The safety snapshot is of the system as it is now: it can't be what's restored.
    #[test]
    fn a_safety_snapshot_that_is_the_restored_one_is_refused() {
        assert_eq!(
            invalid(Plan::parse(&text_with(
                "safety_snapshot",
                &format!("\"{SNAPSHOT}\"")
            ))),
            "the safety snapshot is the snapshot to restore"
        );
    }

    /// PLAN 6b.3: a kept `/home` is never entered, so it has no partition to check.
    #[test]
    fn a_separate_home_with_home_kept_is_refused() {
        let text = with_separate_home()
            .to_text()
            .unwrap()
            .replace("\"home\": \"restore\"", "\"home\": \"keep\"");
        assert_eq!(
            invalid(Plan::parse(&text)),
            "a separate /home is named but home is kept"
        );
    }

    #[test]
    fn a_separate_home_on_the_root_filesystem_is_refused() {
        let text = with_separate_home().to_text().unwrap().replace(HOME, ROOT);
        assert_eq!(
            invalid(Plan::parse(&text)),
            "the separate /home is the root filesystem"
        );
    }

    #[test]
    fn a_separate_home_with_a_bad_or_extra_field_is_refused() {
        let text = with_separate_home().to_text().unwrap();
        for (from, to, reason) in [
            (HOME, "/dev/sda3", "\"uuid\" isn't a UUID"),
            (
                "\"needs\": 2073741824",
                "\"needs\": -1",
                "\"needs\" isn't a whole number of bytes",
            ),
            ("\"needs\": 2073741824", "\"need\": 1", "no \"needs\""),
            (
                "\"needs\": 2073741824",
                "\"needs\": 1, \"mount\": \"/home\"",
                "unknown field \"mount\"",
            ),
        ] {
            assert_eq!(
                invalid(Plan::parse(&text.replace(from, to))),
                format!("\"separate_home\": {reason}"),
                "{to}"
            );
        }
    }

    /// Nothing is written that wouldn't be read back.
    #[test]
    fn a_plan_that_wouldnt_read_back_isnt_written() {
        let dir = temp_dir("plan-bad");
        let bad = Plan {
            snapshot: "latest".to_owned(),
            ..plan()
        };
        assert!(matches!(bad.to_text(), Err(FileError::Invalid(_))));
        assert!(matches!(bad.save(&dir), Err(FileError::Invalid(_))));
        assert!(!dir.join(FILE).exists());
    }

    #[test]
    fn a_saved_plan_loads_from_request_json() {
        let dir = temp_dir("plan-save");
        with_separate_home().save(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("request.json")).unwrap(),
            with_separate_home().to_text().unwrap()
        );
        assert_eq!(Plan::load(&dir).unwrap(), with_separate_home());
    }

    #[test]
    fn a_file_that_fails_validation_is_refused_whole_and_named() {
        let dir = temp_dir("plan-load-bad");
        std::fs::write(dir.join(FILE), text_with("starter_uid", "-1")).unwrap();
        assert_eq!(
            invalid(Plan::load(&dir)),
            "request.json: \"starter_uid\" isn't a user id"
        );
    }

    #[test]
    fn a_missing_plan_is_an_io_error() {
        let dir = temp_dir("plan-missing");
        assert!(matches!(Plan::load(&dir), Err(FileError::Io(_))));
    }

    /// PLAN 6b.5: a plan left on the ready prompt for more than 30 minutes is too old.
    #[test]
    fn a_plan_is_too_old_after_thirty_minutes() {
        let plan = plan();
        assert!(!plan.is_too_old(PREPARED));
        assert!(!plan.is_too_old(PREPARED + 30 * 60));
        assert!(plan.is_too_old(PREPARED + 30 * 60 + 1));
    }

    /// A clock corrected by a little (NTP) between preparing and the check isn't a reason.
    #[test]
    fn a_plan_up_to_two_minutes_in_the_future_is_fine() {
        assert_eq!(MAX_FUTURE_SECS, 120);
        assert!(!plan().is_too_old(PREPARED - 1));
        assert!(!plan().is_too_old(PREPARED - 120));
    }

    /// The clock was set back by more since: the plan's age is unknown.
    #[test]
    fn a_plan_further_in_the_future_is_too_old() {
        assert!(plan().is_too_old(PREPARED - 121));
        assert!(plan().is_too_old(PREPARED - 3600));
        assert!(plan().is_too_old(i64::MIN));
    }

    #[test]
    fn a_clock_far_ahead_doesnt_overflow() {
        assert!(plan().is_too_old(i64::MAX));
    }
}
