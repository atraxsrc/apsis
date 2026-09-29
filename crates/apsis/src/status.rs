// SPDX-License-Identifier: GPL-3.0-only

//! The one place that turns [`ApsisStatus`] into text: the panel label, the tooltip and the
//! icon's severity. Later phases (the strip in the popup and window) call the same functions.
//! No widgets and no theme, so it is unit-testable; colours are chosen by the caller.

use std::time::Duration;

use apsis_core::status::{DISK_CRITICAL, DISK_LOW, age_short};
use apsis_core::{ApsisStatus, DiskStatus, DiskUsage, Due, Severity};

use crate::{fl, fmt};

/// What the panel knows: nothing yet, a list that failed, or a status from a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusView {
    /// No list yet.
    Unloaded,
    /// The last list failed: the backup disk is most likely not connected.
    Failed,
    Loaded(ApsisStatus),
}

impl StatusView {
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            Self::Loaded(status) => status.icon_severity(),
            Self::Unloaded | Self::Failed => Severity::None,
        }
    }

    /// `12h · 62%`, with `-` for what is not known.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Loaded(status) => status.label_short(),
            Self::Unloaded | Self::Failed => ApsisStatus::disconnected().label_short(),
        }
    }

    /// The panel tooltip: `last`, `next` and `disk` lines. `next` is always `manual only`: there
    /// is no scheduler (see DECISIONS.md).
    #[must_use]
    pub fn tooltip(&self) -> String {
        match self {
            Self::Unloaded => fl!("app-title"),
            Self::Failed => format!(
                "{}\n{}",
                fl!("app-title"),
                disk_line(&DiskStatus::NotMounted)
            ),
            Self::Loaded(status) => format!(
                "{}\n{}\n{}",
                last_line(status),
                fl!("tooltip-next-manual"),
                disk_line(&status.disk)
            ),
        }
    }

    /// The apsis strip's text (window, and later the popup).
    #[must_use]
    pub fn strip(&self) -> Strip {
        let next = fl!("strip-next-manual");
        match self {
            Self::Unloaded => Strip {
                last: "-".to_owned(),
                last_note: None,
                next,
                disk: DiskStrip::Text("-".to_owned()),
            },
            Self::Failed => Strip {
                last: "-".to_owned(),
                last_note: None,
                next,
                disk: DiskStrip::Text(fl!("strip-disk-not-connected")),
            },
            Self::Loaded(status) => {
                let (last, last_note) = match (status.last, status.due()) {
                    (Some(age), Due::Overdue { days }) => {
                        (ago(age), Some(fl!("strip-over", days = days.to_string())))
                    }
                    (Some(age), _) => (ago(age), None),
                    (None, Due::Never { days }) => (
                        fl!("strip-last-none"),
                        Some(fl!("strip-over", days = days.to_string())),
                    ),
                    (None, _) => (fl!("strip-last-none"), None),
                };
                let disk = match &status.disk {
                    DiskStatus::Mounted { device, usage } => DiskStrip::Mounted {
                        device: fmt::device_name(device).to_owned(),
                        size: format!(
                            "{} / {}",
                            fmt::size_short(usage.used),
                            fmt::size_short(usage.total)
                        ),
                        usage: *usage,
                        used_free: fl!(
                            "strip-used-free",
                            pct = status.used_pct().unwrap_or(0).to_string(),
                            free = fmt::size_short(usage.free)
                        ),
                        note: low_limit(status)
                            .map(|(pct, severity)| (fl!("strip-under", pct = pct), severity)),
                    },
                    DiskStatus::NotMounted => DiskStrip::Text(fl!("strip-disk-not-connected")),
                    DiskStatus::Unknown => DiskStrip::Text(fl!("strip-disk-unknown")),
                };
                Strip {
                    last,
                    last_note,
                    next,
                    disk,
                }
            }
        }
    }
}

/// The apsis strip's text: the time column and the disk column. No widgets; `app.rs` lays it out
/// and picks the colours (`Severity` says which role a phrase takes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strip {
    /// `12h ago`, `none yet`, or `-` before the first list.
    pub last: String,
    /// `(over 7 days)` when the reminder is due; the phrase that takes the warning colour.
    pub last_note: Option<String>,
    /// `manual only` (no scheduler).
    pub next: String,
    pub disk: DiskStrip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskStrip {
    Mounted {
        /// `sda1`
        device: String,
        /// `372G / 596G`
        size: String,
        /// The numbers for the bar.
        usage: DiskUsage,
        /// `62% used · 224G free`
        used_free: String,
        /// `(under 10%)` with the role it takes, when space is low.
        note: Option<(String, Severity)>,
    },
    /// `not connected`, `unknown`, or `-`.
    Text(String),
}

/// `12h ago`, or `just now`.
fn ago(age: Duration) -> String {
    match age_short(age).as_str() {
        "now" => "just now".to_owned(),
        short => format!("{short} ago"),
    }
}

/// `last  12h ago`, with `(over 7 days)` when the reminder is due.
fn last_line(status: &ApsisStatus) -> String {
    let due = status.due();
    match (status.last, due) {
        (Some(age), Due::Overdue { days }) => fl!(
            "tooltip-last-stale",
            ago = ago(age),
            days = days.to_string()
        ),
        (Some(age), _) => fl!("tooltip-last", ago = ago(age)),
        (None, Due::Never { days }) => fl!("tooltip-last-none-stale", days = days.to_string()),
        (None, _) => fl!("tooltip-last-none"),
    }
}

/// `disk  372G / 596G  224G free`; under the low limits `disk  28G free  (under 10%)`.
fn disk_line(disk: &DiskStatus) -> String {
    match disk {
        DiskStatus::Mounted { usage, .. } => {
            let status = ApsisStatus {
                last: None,
                remind: None,
                disk: disk.clone(),
            };
            let free = fmt::size_short(usage.free);
            if status.disk_critical() {
                fl!(
                    "tooltip-disk-low",
                    free = free,
                    pct = percent(DISK_CRITICAL)
                )
            } else if status.disk_warning() {
                fl!("tooltip-disk-low", free = free, pct = percent(DISK_LOW))
            } else {
                fl!(
                    "tooltip-disk",
                    used = fmt::size_short(usage.used),
                    total = fmt::size_short(usage.total),
                    free = free
                )
            }
        }
        DiskStatus::NotMounted => fl!("tooltip-disk-not-connected"),
        DiskStatus::Unknown => fl!("tooltip-disk-unknown"),
    }
}

/// The limit free space is under, as a whole percent, and the role it takes: `("10", Warning)`
/// or `("5", Critical)`. `None` when there is plenty, or nothing is known.
fn low_limit(status: &ApsisStatus) -> Option<(String, Severity)> {
    if status.disk_critical() {
        Some((percent(DISK_CRITICAL), Severity::Critical))
    } else if status.disk_warning() {
        Some((percent(DISK_LOW), Severity::Warning))
    } else {
        None
    }
}

/// `0.05` as `5`.
fn percent(share: f64) -> String {
    format!("{:.0}", share * 100.0)
}

#[cfg(test)]
mod tests {
    use apsis_core::DiskUsage;

    use super::*;

    const G: u64 = 1024 * 1024 * 1024;

    fn loaded(last_hours: Option<u64>, remind: Option<u32>, disk: DiskStatus) -> StatusView {
        StatusView::Loaded(ApsisStatus {
            last: last_hours.map(|h| Duration::from_secs(h * 3600)),
            remind,
            disk,
        })
    }

    fn disk(used: u64, free: u64) -> DiskStatus {
        DiskStatus::Mounted {
            device: "/dev/sdx1".to_owned(),
            usage: DiskUsage {
                total: used + free,
                used,
                free,
            },
        }
    }

    #[test]
    fn normal() {
        let view = loaded(Some(12), Some(7), disk(372 * G, 224 * G));
        assert_eq!(
            view.tooltip(),
            "last  12h ago\nnext  manual only\ndisk  372G / 596G  224G free"
        );
        assert_eq!(view.label(), "12h · 63%");
        assert_eq!(view.severity(), Severity::None);
    }

    #[test]
    fn overdue_names_the_days() {
        let view = loaded(Some(9 * 24), Some(7), disk(372 * G, 224 * G));
        assert_eq!(
            view.tooltip(),
            "last  9d ago  (over 7 days)\nnext  manual only\ndisk  372G / 596G  224G free"
        );
        assert_eq!(view.severity(), Severity::Warning);
    }

    #[test]
    fn reminder_off_says_nothing_about_age() {
        let view = loaded(Some(9 * 24), None, disk(372 * G, 224 * G));
        assert_eq!(
            view.tooltip(),
            "last  9d ago\nnext  manual only\ndisk  372G / 596G  224G free"
        );
        assert_eq!(view.severity(), Severity::None);
    }

    #[test]
    fn no_snapshots_yet() {
        assert_eq!(
            loaded(None, None, disk(G, 99 * G)).tooltip(),
            "last  none yet\nnext  manual only\ndisk  1G / 100G  99G free"
        );
        let view = loaded(None, Some(7), disk(G, 99 * G));
        assert_eq!(
            view.tooltip(),
            "last  none yet  (over 7 days)\nnext  manual only\ndisk  1G / 100G  99G free"
        );
        assert_eq!(view.severity(), Severity::Warning);
        assert_eq!(view.label(), "- · 1%");
    }

    #[test]
    fn a_fresh_snapshot_reads_just_now() {
        let view = loaded(Some(0), None, DiskStatus::Unknown);
        assert!(view.tooltip().starts_with("last  just now\n"));
    }

    #[test]
    fn low_disk_names_the_limit_and_colours_the_icon() {
        let low = loaded(Some(1), Some(7), disk(899 * G, 99 * G));
        assert_eq!(
            low.tooltip(),
            "last  1h ago\nnext  manual only\ndisk  99G free  (under 10%)"
        );
        assert_eq!(low.severity(), Severity::Warning);
        let critical = loaded(Some(1), Some(7), disk(960 * G, 40 * G));
        assert_eq!(
            critical.tooltip(),
            "last  1h ago\nnext  manual only\ndisk  40G free  (under 5%)"
        );
        assert_eq!(critical.severity(), Severity::Critical);
    }

    #[test]
    fn disk_not_connected_and_unknown() {
        assert_eq!(StatusView::Failed.tooltip(), "Apsis\ndisk  not connected");
        assert_eq!(StatusView::Failed.label(), "- · -");
        assert_eq!(StatusView::Failed.severity(), Severity::None);
        assert_eq!(
            loaded(Some(2), None, DiskStatus::NotMounted).tooltip(),
            "last  2h ago\nnext  manual only\ndisk  not connected"
        );
        let unknown = loaded(Some(2), None, DiskStatus::Unknown);
        assert_eq!(
            unknown.tooltip(),
            "last  2h ago\nnext  manual only\ndisk  unknown"
        );
        assert_eq!(unknown.label(), "2h · -");
    }

    #[test]
    fn nothing_loaded_is_just_the_name() {
        assert_eq!(StatusView::Unloaded.tooltip(), "Apsis");
        assert_eq!(StatusView::Unloaded.label(), "- · -");
        assert_eq!(StatusView::Unloaded.severity(), Severity::None);
    }

    #[test]
    fn limits_print_as_whole_percents() {
        assert_eq!(percent(DISK_LOW), "10");
        assert_eq!(percent(DISK_CRITICAL), "5");
    }

    #[test]
    fn strip_says_time_and_disk_in_words() {
        let strip = loaded(Some(12), Some(7), disk(372 * G, 224 * G)).strip();
        assert_eq!(strip.last, "12h ago");
        assert_eq!(strip.last_note, None);
        assert_eq!(strip.next, "manual only");
        let DiskStrip::Mounted {
            device,
            size,
            used_free,
            note,
            ..
        } = strip.disk
        else {
            panic!("{:?}", strip.disk)
        };
        assert_eq!(device, "sdx1");
        assert_eq!(size, "372G / 596G");
        assert_eq!(used_free, "63% used · 224G free");
        assert_eq!(note, None);
    }

    #[test]
    fn strip_names_the_overdue_phrase_and_the_low_limit() {
        let strip = loaded(Some(9 * 24), Some(7), disk(960 * G, 40 * G)).strip();
        assert_eq!(strip.last, "9d ago");
        assert_eq!(strip.last_note.as_deref(), Some("(over 7 days)"));
        let DiskStrip::Mounted { note, .. } = strip.disk else {
            panic!()
        };
        assert_eq!(note, Some(("(under 5%)".to_owned(), Severity::Critical)));
        let low = loaded(Some(1), None, disk(899 * G, 99 * G)).strip();
        let DiskStrip::Mounted { note, .. } = low.disk else {
            panic!()
        };
        assert_eq!(note, Some(("(under 10%)".to_owned(), Severity::Warning)));
        let none = loaded(None, Some(7), disk(G, 99 * G)).strip();
        assert_eq!(none.last, "none yet");
        assert_eq!(none.last_note.as_deref(), Some("(over 7 days)"));
    }

    #[test]
    fn strip_without_a_disk_or_a_list() {
        let text = |view: StatusView| match view.strip().disk {
            DiskStrip::Text(text) => text,
            other => panic!("{other:?}"),
        };
        assert_eq!(text(StatusView::Unloaded), "-");
        assert_eq!(text(StatusView::Failed), "not connected");
        assert_eq!(
            text(loaded(Some(1), None, DiskStatus::NotMounted)),
            "not connected"
        );
        assert_eq!(text(loaded(Some(1), None, DiskStatus::Unknown)), "unknown");
        assert_eq!(StatusView::Unloaded.strip().last, "-");
        assert_eq!(StatusView::Failed.strip().next, "manual only");
    }
}
