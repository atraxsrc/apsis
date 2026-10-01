// SPDX-License-Identifier: GPL-3.0-only

//! What the panel shows at a glance: how old the newest snapshot is, whether that is overdue for
//! the reminder, and how full the backup device is. Pure math and no UI, so it is unit-tested;
//! `apsis` turns it into the panel label, tooltip and icon colour.

use std::path::Path;
use std::time::Duration;

use jiff::civil::DateTime;

use crate::{DiskUsage, SnapshotList};

/// Free space under this share of the usable space: the theme's warning colour.
pub const DISK_LOW: f64 = 0.10;
/// Free space under this share: the destructive colour.
pub const DISK_CRITICAL: f64 = 0.05;

const DAY: u64 = 24 * 60 * 60;

/// The newest snapshot, the reminder and the backup device, as of one list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApsisStatus {
    /// Age of the newest snapshot; `None` when there is none. A snapshot from the future
    /// (clock change) counts as age 0.
    pub last: Option<Duration>,
    /// Remind after this many days; `None` = off.
    pub remind: Option<u32>,
    pub disk: DiskStatus,
}

/// The backup device, the disk axis of the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskStatus {
    /// Mounted by the helper, with its space.
    Mounted { device: String, usage: DiskUsage },
    /// No device set, or it could not be reached (unplugged).
    NotMounted,
    /// The list worked but gave no space figures.
    Unknown,
}

/// Where the reminder stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// The reminder is off.
    Off,
    /// The newest snapshot is recent enough.
    Ok,
    /// The newest snapshot is older than `days`.
    Overdue { days: u32 },
    /// The reminder is on and there is no snapshot at all.
    Never { days: u32 },
}

/// How loudly the panel icon should say something: the theme's warning and destructive roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    None,
    Warning,
    Critical,
}

impl ApsisStatus {
    /// From a list at local time `now`. `remind_days` is the setting, 0 = off; it counts as off
    /// when no backup device is set, since there is nothing to judge by.
    #[must_use]
    pub fn from_list(list: &SnapshotList, remind_days: u32, now: DateTime) -> Self {
        let last = list
            .snapshots
            .iter()
            .map(|s| s.created)
            .max()
            .map(|created| age(created, now));
        let disk = match (&list.device, list.usage) {
            (Some(device), Some(usage)) => DiskStatus::Mounted {
                device: device.clone(),
                usage,
            },
            (Some(_), None) => DiskStatus::Unknown,
            (None, _) => DiskStatus::NotMounted,
        };
        let remind = (remind_days > 0 && list.device.is_some()).then_some(remind_days);
        Self { last, remind, disk }
    }

    /// The list failed (a disk that is unplugged, say): nothing to say about time, and no
    /// reminder, since nothing to judge by.
    #[must_use]
    pub fn disconnected() -> Self {
        Self {
            last: None,
            remind: None,
            disk: DiskStatus::NotMounted,
        }
    }

    /// The same, with the backup disk gone since the list (its UUID left `/dev/disk/by-uuid`):
    /// the disk reads `not connected`; the time side stays as the list showed it (time and disk
    /// fail independently).
    #[must_use]
    pub fn disk_gone(self) -> Self {
        Self {
            disk: DiskStatus::NotMounted,
            ..self
        }
    }

    #[must_use]
    pub fn due(&self) -> Due {
        let Some(days) = self.remind else {
            return Due::Off;
        };
        match self.last {
            None => Due::Never { days },
            Some(last) if last > Duration::from_secs(u64::from(days) * DAY) => {
                Due::Overdue { days }
            }
            Some(_) => Due::Ok,
        }
    }

    /// The reminder is due: the newest snapshot is too old, or there is none.
    #[must_use]
    pub fn stale_snap(&self) -> bool {
        matches!(self.due(), Due::Overdue { .. } | Due::Never { .. })
    }

    /// `12h · 62%`: age of the newest snapshot, share of the backup disk in use. An unknown
    /// side is `-`.
    #[must_use]
    pub fn label_short(&self) -> String {
        let last = self.last.map_or_else(|| "-".to_owned(), age_short);
        let used = self
            .used_pct()
            .map_or_else(|| "-".to_owned(), |p| format!("{p}%"));
        format!("{last} · {used}")
    }

    /// Share of the usable space in use as `df` prints it (rounded up), `0..=100`.
    #[must_use]
    pub fn used_pct(&self) -> Option<u8> {
        let DiskStatus::Mounted { usage, .. } = &self.disk else {
            return None;
        };
        let usable = u128::from(usage.used) + u128::from(usage.free);
        if usable == 0 {
            return Some(100);
        }
        let pct = (u128::from(usage.used) * 100).div_ceil(usable).min(100);
        u8::try_from(pct).ok()
    }

    #[must_use]
    pub fn free_bytes(&self) -> Option<u64> {
        match &self.disk {
            DiskStatus::Mounted { usage, .. } => Some(usage.free),
            DiskStatus::NotMounted | DiskStatus::Unknown => None,
        }
    }

    /// Free space is under [`DISK_LOW`] (which includes the critical case).
    #[must_use]
    pub fn disk_warning(&self) -> bool {
        matches!(&self.disk, DiskStatus::Mounted { usage, .. } if usage.free_fraction() < DISK_LOW)
    }

    /// Free space is under [`DISK_CRITICAL`].
    #[must_use]
    pub fn disk_critical(&self) -> bool {
        matches!(&self.disk, DiskStatus::Mounted { usage, .. } if usage.free_fraction() < DISK_CRITICAL)
    }

    /// The panel icon: destructive for critical space, warning for low space or a due reminder.
    #[must_use]
    pub fn icon_severity(&self) -> Severity {
        if self.disk_critical() {
            Severity::Critical
        } else if self.disk_warning() || self.stale_snap() {
            Severity::Warning
        } else {
            Severity::None
        }
    }
}

/// How old `then` is at `now`; zero if it is in the future.
fn age(then: DateTime, now: DateTime) -> Duration {
    let secs = now.duration_since(then).as_secs();
    Duration::from_secs(u64::try_from(secs).unwrap_or(0))
}

/// Where udev links filesystems by UUID.
pub const BY_UUID: &str = "/dev/disk/by-uuid";

/// Whether the filesystem `uuid` is connected: its link in `by_uuid` ([`BY_UUID`]) resolves.
/// No root and no mount, so the applet can ask every few seconds. `None` when `uuid` can't be a
/// file name there (empty, `.`, `..`, a `/`), so nothing is claimed about it.
#[must_use]
pub fn disk_connected(by_uuid: &Path, uuid: &str) -> Option<bool> {
    if uuid.is_empty() || uuid == "." || uuid == ".." || uuid.contains(['/', '\0']) {
        return None;
    }
    // `exists` follows the link: a link left behind to a device node that's gone is "no".
    Some(by_uuid.join(uuid).exists())
}

/// `now`, `5m`, `47h`, `3d`: the same steps as the popup's "5m ago".
#[must_use]
pub fn age_short(age: Duration) -> String {
    let minutes = age.as_secs() / 60;
    match minutes {
        ..1 => "now".to_owned(),
        1..60 => format!("{minutes}m"),
        60..2880 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

/// A device size the way lsblk prints it: `931.5G`, `512M`, binary units.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    #[allow(clippy::cast_precision_loss, reason = "one decimal shown")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text}{}", UNITS[unit])
}

/// [`size`] without the decimal from 10 up: `448G`, `4.5G`. For the disk line and the tooltip,
/// where the numbers sit side by side.
#[must_use]
pub fn size_short(bytes: u64) -> String {
    let text = size(bytes);
    let split = text
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    match number.parse::<f64>() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a displayed size under 1024"
        )]
        Ok(value) if value >= 10.0 => format!("{}{unit}", value.round() as u64),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;
    use crate::{Snapshot, Tag};

    #[test]
    fn the_backup_disk_is_there_while_its_uuid_link_resolves() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/by-uuid");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let device = dir.join("sdx1");
        std::fs::write(&device, "").unwrap();
        std::os::unix::fs::symlink(&device, dir.join("1a2b")).unwrap();
        assert_eq!(disk_connected(&dir, "1a2b"), Some(true));
        assert_eq!(disk_connected(&dir, "3c4d"), Some(false));
        // Unplugged: udev removes the device; a link left behind doesn't count.
        std::fs::remove_file(&device).unwrap();
        assert_eq!(disk_connected(&dir, "1a2b"), Some(false));
        // Not a UUID it can look up: no claim either way.
        for bad in ["", ".", "..", "../sdx1", "a/b"] {
            assert_eq!(disk_connected(&dir, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_gone_disk_keeps_the_time_side() {
        let status = ApsisStatus {
            last: Some(Duration::from_secs(12 * 3600)),
            remind: Some(7),
            disk: DiskStatus::Unknown,
        };
        let gone = status.clone().disk_gone();
        assert_eq!(gone.disk, DiskStatus::NotMounted);
        assert_eq!((gone.last, gone.remind), (status.last, status.remind));
        assert_eq!(gone.label_short(), "12h · -");
        assert!(!gone.disk_warning());
    }

    fn usage(used: u64, free: u64) -> DiskUsage {
        DiskUsage {
            total: used + free,
            used,
            free,
        }
    }

    fn mounted(used: u64, free: u64) -> DiskStatus {
        DiskStatus::Mounted {
            device: "/dev/sdx1".to_owned(),
            usage: usage(used, free),
        }
    }

    fn status(last_hours: Option<u64>, remind: Option<u32>, disk: DiskStatus) -> ApsisStatus {
        ApsisStatus {
            last: last_hours.map(|h| Duration::from_secs(h * 3600)),
            remind,
            disk,
        }
    }

    #[test]
    fn due_has_four_states() {
        let disk = || DiskStatus::Unknown;
        assert_eq!(status(Some(9 * 24), None, disk()).due(), Due::Off);
        assert_eq!(status(None, None, disk()).due(), Due::Off);
        assert_eq!(status(Some(24), Some(7), disk()).due(), Due::Ok);
        assert_eq!(
            status(Some(8 * 24), Some(7), disk()).due(),
            Due::Overdue { days: 7 }
        );
        assert_eq!(status(None, Some(7), disk()).due(), Due::Never { days: 7 });
    }

    #[test]
    fn due_is_strictly_older_than_n_days() {
        let at = |secs| ApsisStatus {
            last: Some(Duration::from_secs(secs)),
            remind: Some(7),
            disk: DiskStatus::Unknown,
        };
        assert_eq!(at(7 * DAY).due(), Due::Ok);
        assert_eq!(at(7 * DAY + 1).due(), Due::Overdue { days: 7 });
        assert!(!at(7 * DAY).stale_snap());
        assert!(at(7 * DAY + 1).stale_snap());
        assert!(status(None, Some(7), DiskStatus::Unknown).stale_snap());
        assert!(!status(None, None, DiskStatus::Unknown).stale_snap());
    }

    #[test]
    fn label_short_with_unknowns() {
        let full = status(Some(12), None, mounted(62, 38));
        assert_eq!(full.label_short(), "12h · 62%");
        assert_eq!(
            status(Some(12), None, DiskStatus::Unknown).label_short(),
            "12h · -"
        );
        assert_eq!(status(None, None, mounted(62, 38)).label_short(), "- · 62%");
        assert_eq!(
            status(None, None, DiskStatus::NotMounted).label_short(),
            "- · -"
        );
    }

    #[test]
    fn age_steps_match_the_popup() {
        let s = Duration::from_secs;
        assert_eq!(age_short(s(0)), "now");
        assert_eq!(age_short(s(59)), "now");
        assert_eq!(age_short(s(60)), "1m");
        assert_eq!(age_short(s(59 * 60)), "59m");
        assert_eq!(age_short(s(3600)), "1h");
        assert_eq!(age_short(s(47 * 3600 + 59 * 60)), "47h");
        assert_eq!(age_short(s(48 * 3600)), "2d");
        assert_eq!(age_short(s(3 * DAY)), "3d");
    }

    #[test]
    fn used_pct_rounds_up_like_df() {
        let pct = |used, free| status(None, None, mounted(used, free)).used_pct();
        assert_eq!(pct(62, 38), Some(62));
        assert_eq!(pct(372, 224), Some(63)); // 62.4% -> 63
        assert_eq!(pct(0, 100), Some(0));
        assert_eq!(pct(1, 999), Some(1));
        assert_eq!(pct(100, 0), Some(100));
        assert_eq!(pct(0, 0), Some(100));
        assert_eq!(status(None, None, DiskStatus::NotMounted).used_pct(), None);
        assert_eq!(status(None, None, DiskStatus::Unknown).used_pct(), None);
    }

    #[test]
    fn free_bytes_is_what_is_left_for_everyone() {
        assert_eq!(
            status(None, None, mounted(600, 350)).free_bytes(),
            Some(350)
        );
        // ext4 keeps blocks back for root: used + free < total.
        let reserved = ApsisStatus {
            disk: DiskStatus::Mounted {
                device: "d".to_owned(),
                usage: DiskUsage {
                    total: 1000,
                    used: 600,
                    free: 350,
                },
            },
            ..status(None, None, DiskStatus::Unknown)
        };
        assert_eq!(reserved.free_bytes(), Some(350));
        assert_eq!(
            status(None, None, DiskStatus::NotMounted).free_bytes(),
            None
        );
        assert_eq!(status(None, None, DiskStatus::Unknown).free_bytes(), None);
    }

    #[test]
    fn warning_and_critical_boundaries() {
        let of = |used, free| status(None, None, mounted(used, free));
        // exactly 10% free is not low; just under is
        assert!(!of(900, 100).disk_warning());
        assert!(of(901, 99).disk_warning());
        assert!(!of(901, 99).disk_critical());
        // exactly 5% free is not critical; just under is
        assert!(!of(950, 50).disk_critical());
        assert!(of(950, 50).disk_warning());
        assert!(of(951, 49).disk_critical());
        assert!(of(951, 49).disk_warning());
        assert!(of(1, 0).disk_critical());
        // no figures, no warning
        for disk in [DiskStatus::NotMounted, DiskStatus::Unknown] {
            let s = status(None, None, disk);
            assert!(!s.disk_warning() && !s.disk_critical());
        }
    }

    #[test]
    fn icon_severity_takes_the_worse_signal() {
        let ok = mounted(500, 500);
        assert_eq!(
            status(Some(1), Some(7), ok.clone()).icon_severity(),
            Severity::None
        );
        assert_eq!(
            status(Some(1), None, ok.clone()).icon_severity(),
            Severity::None
        );
        assert_eq!(
            status(Some(9 * 24), Some(7), ok.clone()).icon_severity(),
            Severity::Warning
        );
        assert_eq!(status(None, Some(7), ok).icon_severity(), Severity::Warning);
        assert_eq!(
            status(Some(1), Some(7), mounted(901, 99)).icon_severity(),
            Severity::Warning
        );
        // critical space wins over an overdue reminder
        assert_eq!(
            status(Some(9 * 24), Some(7), mounted(951, 49)).icon_severity(),
            Severity::Critical
        );
        // an overdue reminder still warns on a disk we know nothing about
        assert_eq!(
            status(Some(9 * 24), Some(7), DiskStatus::Unknown).icon_severity(),
            Severity::Warning
        );
    }

    fn snapshot(created: DateTime) -> Snapshot {
        Snapshot {
            name: created.strftime("%Y-%m-%d_%H-%M-%S").to_string(),
            created,
            tags: vec![Tag::OnDemand],
            comment: None,
            rsync_flags: None,
        }
    }

    #[test]
    fn from_list_uses_the_newest_snapshot_and_the_device() {
        let now = date(2026, 9, 29).at(18, 0, 0, 0);
        let list = SnapshotList {
            device: Some("/dev/sdx1".to_owned()),
            snapshots: vec![
                snapshot(date(2026, 9, 20).at(6, 0, 0, 0)),
                snapshot(date(2026, 9, 29).at(6, 0, 0, 0)),
            ],
            usage: Some(usage(600, 400)),
            ..SnapshotList::default()
        };
        let status = ApsisStatus::from_list(&list, 7, now);
        assert_eq!(status.last, Some(Duration::from_secs(12 * 3600)));
        assert_eq!(status.due(), Due::Ok);
        assert_eq!(status.label_short(), "12h · 60%");

        // reminder 0 = off
        assert_eq!(ApsisStatus::from_list(&list, 0, now).due(), Due::Off);
        // no space figures: unknown, not unplugged
        let bare = SnapshotList {
            usage: None,
            ..list.clone()
        };
        assert_eq!(
            ApsisStatus::from_list(&bare, 7, now).disk,
            DiskStatus::Unknown
        );
        // no device: nothing to judge by, so no reminder
        let unset = SnapshotList {
            device: None,
            snapshots: vec![],
            usage: None,
            ..list
        };
        let s = ApsisStatus::from_list(&unset, 7, now);
        assert_eq!(s.disk, DiskStatus::NotMounted);
        assert_eq!(s.due(), Due::Off);
    }

    #[test]
    fn from_list_without_snapshots_is_never() {
        let now = date(2026, 9, 29).at(18, 0, 0, 0);
        let list = SnapshotList {
            device: Some("/dev/sdx1".to_owned()),
            ..SnapshotList::default()
        };
        assert_eq!(
            ApsisStatus::from_list(&list, 7, now).due(),
            Due::Never { days: 7 }
        );
    }

    #[test]
    fn a_snapshot_from_the_future_is_age_zero() {
        let now = date(2026, 9, 29).at(6, 0, 0, 0);
        let list = SnapshotList {
            device: Some("d".to_owned()),
            snapshots: vec![snapshot(date(2026, 9, 29).at(7, 0, 0, 0))],
            ..SnapshotList::default()
        };
        assert_eq!(
            ApsisStatus::from_list(&list, 7, now).last,
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn disconnected_has_nothing_to_say() {
        let s = ApsisStatus::disconnected();
        assert_eq!(s.due(), Due::Off);
        assert_eq!(s.label_short(), "- · -");
        assert_eq!(s.icon_severity(), Severity::None);
    }

    #[test]
    fn sizes_one_decimal_below_ten_else_integer() {
        assert_eq!(size(0), "0B");
        assert_eq!(size(512 * 1024 * 1024), "512M");
        assert_eq!(size(1_000_203_837_440), "931.5G");
        assert_eq!(size(2 * 1024_u64.pow(4)), "2T");
        let g = 1024_u64.pow(3);
        assert_eq!(size_short(g * 9 + g / 2), "9.5G");
        assert_eq!(size_short(g * 10), "10G");
        assert_eq!(size_short(g * 10 + g / 2), "11G");
        assert_eq!(size_short(g * 448), "448G");
        assert_eq!(size_short(1_000_203_837_440), "932G");
        assert_eq!(size_short(512 * 1024 * 1024), "512M");
        assert_eq!(size_short(g * 3 / 2), "1.5G");
        assert_eq!(size_short(0), "0B");
    }
}
