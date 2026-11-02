// SPDX-License-Identifier: GPL-3.0-only

//! The one place that turns [`ApsisStatus`] into text: the panel label, the tooltip, the
//! icon's severity and the popup's line.
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
    /// From a list, or from the snapshot remembered while the disk is unplugged.
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
    /// is no scheduler.
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

    /// The popup's status line and disk bar while nothing runs. `chosen` is false when the
    /// list says no backup disk is set.
    #[must_use]
    pub fn popup(&self, chosen: bool) -> PopupStatus {
        let not_connected = PopupStatus {
            last: String::new(),
            overdue: false,
            usage: None,
            disk: fl!("popup-disk-not-connected"),
            figures: None,
            free_of: None,
        };
        let status = match self {
            Self::Unloaded => {
                return PopupStatus {
                    disk: String::new(),
                    ..not_connected
                };
            }
            Self::Failed => return not_connected,
            Self::Loaded(status) => status,
        };
        let (last, overdue) = match (status.last, status.due()) {
            (Some(age), due) => (
                fl!("popup-last", age = fmt::age_long(age)),
                matches!(due, Due::Overdue { .. }),
            ),
            (None, due) => (fl!("popup-no-snapshot"), matches!(due, Due::Never { .. })),
        };
        let mut popup = PopupStatus {
            last,
            overdue,
            ..not_connected
        };
        match &status.disk {
            _ if !chosen => popup.disk = fl!("popup-no-disk"),
            DiskStatus::Mounted { device, usage } => {
                popup.usage = Some(*usage);
                popup.disk = String::new();
                popup.free_of = Some(fl!(
                    "popup-free-of",
                    free = fmt::size_short(usage.free),
                    total = fmt::size_short(usage.total)
                ));
                popup.figures = Some(format!(
                    "{}  {} / {} · {}",
                    fmt::device_name(device),
                    fmt::size_short(usage.used),
                    fmt::size_short(usage.total),
                    fl!(
                        "strip-used-free",
                        pct = status.used_pct().unwrap_or(0).to_string(),
                        free = fmt::size_short(usage.free)
                    )
                ));
            }
            DiskStatus::NotMounted => {}
            DiskStatus::Unknown => popup.disk = fl!("popup-disk-unknown"),
        }
        popup
    }
}

/// What the popup shows while nothing runs. No widgets; the view draws the bar in the theme's
/// colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupStatus {
    /// `Last snapshot 34m ago`, `No snapshot yet`, or empty while nothing is known.
    pub last: String,
    /// `last` takes the warning colour: the reminder is due.
    pub overdue: bool,
    /// The backup disk's numbers, for the bar. `None`: no bar; `disk` says why.
    pub usage: Option<DiskUsage>,
    /// `Backup disk not connected`, `No backup disk chosen`, `Backup disk unknown`; empty
    /// with a bar, or while nothing is known.
    pub disk: String,
    /// The full figures, for the bar's tooltip: `sda  15G / 28G · 58% used · 11G free`.
    pub figures: Option<String>,
    /// `11G free of 28G`, under the bar.
    pub free_of: Option<String>,
}

impl PopupStatus {
    /// The popup's first line: `last`, or `Reading snapshots…` while nothing is known at all.
    /// With no `last` and a reason in `disk`, there's nothing being read: no line.
    #[must_use]
    pub fn line(&self) -> Option<String> {
        match (self.last.is_empty(), self.disk.is_empty()) {
            (false, _) => Some(self.last.clone()),
            (true, true) => Some(fl!("loading")),
            (true, false) => None,
        }
    }
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
    fn popup_says_the_age_and_has_the_disk_for_the_bar() {
        let popup = loaded(Some(12), Some(7), disk(372 * G, 224 * G)).popup(true);
        assert_eq!(popup.last, "Last snapshot 12 hours ago");
        assert!(!popup.overdue);
        assert_eq!(popup.disk, "");
        assert_eq!(
            popup.figures.as_deref(),
            Some("sdx1  372G / 596G · 63% used · 224G free")
        );
        assert_eq!(popup.usage.map(|u| u.free), Some(224 * G));
    }

    #[test]
    fn popup_says_free_of_total() {
        let popup = loaded(Some(12), Some(7), disk(372 * G, 224 * G)).popup(true);
        assert_eq!(popup.free_of.as_deref(), Some("224G free of 596G"));
        assert_eq!(StatusView::Failed.popup(true).free_of, None);
    }

    #[test]
    fn popup_warns_for_an_overdue_snapshot() {
        assert!(
            loaded(Some(9 * 24), Some(7), disk(899 * G, 99 * G))
                .popup(true)
                .overdue
        );
        let none = loaded(None, Some(7), disk(G, 99 * G)).popup(true);
        assert_eq!(none.last, "No snapshot yet");
        assert!(none.overdue);
        assert!(!loaded(None, None, disk(G, 99 * G)).popup(true).overdue);
    }

    #[test]
    fn popup_without_a_disk_has_no_bar() {
        for popup in [
            StatusView::Failed.popup(true),
            loaded(Some(1), None, DiskStatus::NotMounted).popup(true),
        ] {
            assert_eq!(popup.disk, "Backup disk not connected");
            assert_eq!((popup.usage, popup.figures), (None, None));
        }
        let unchosen = loaded(Some(1), None, DiskStatus::Unknown).popup(false);
        assert_eq!(unchosen.disk, "No backup disk chosen");
        assert_eq!(unchosen.usage, None);
        let unloaded = StatusView::Unloaded.popup(true);
        assert_eq!((unloaded.last.as_str(), unloaded.disk.as_str()), ("", ""));
    }

    #[test]
    fn popup_says_reading_only_while_nothing_is_known() {
        assert_eq!(
            StatusView::Unloaded.popup(true).line().as_deref(),
            Some("Reading snapshots…")
        );
        assert_eq!(StatusView::Failed.popup(true).line(), None);
        assert_eq!(
            loaded(Some(48), None, DiskStatus::NotMounted)
                .popup(true)
                .line()
                .as_deref(),
            Some("Last snapshot 2 days ago")
        );
    }
}
