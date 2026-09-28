// SPDX-License-Identifier: GPL-3.0-only

//! "Keep the last N manual snapshots": which on-demand snapshots to delete. Pure; the applet
//! shows the plan and deletes only after `y`, one snapshot at a time through the helper.
//!
//! Commented snapshots are pinned: they are never removed and don't count towards N, so N is
//! the number of uncommented on-demand snapshots kept (unlike Timeshift, whose levels count
//! commented ones, `SnapshotRepo.vala:638-688` in 24.01.1). The uncommented ones past the
//! count are candidates, newest first. A candidate is deleted only if on-demand is its only
//! tag: the `timeshift` command line can't take one tag off, and a snapshot that is also
//! hourly, daily... is Timeshift's retention's to handle. And the newest snapshot is never
//! deleted.

use std::cmp::Reverse;

use crate::model::{Snapshot, Tag};

/// Why a manual snapshot past the count stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// One of the newest N uncommented manual snapshots.
    Recent,
    /// It has a comment: pinned, and not counted towards N.
    Comment,
    /// It has other tags too (boot, hourly...): Timeshift's retention decides.
    OtherTags,
    /// The newest snapshot of all. Can't happen with a count of at least 1 (it's then among
    /// the newest N); kept as a guard.
    Newest,
}

/// What keeping the last `keep` manual snapshots means for a list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManualPlan {
    /// Names to delete, oldest first.
    pub delete: Vec<String>,
    /// Every other manual snapshot and why it stays, newest first.
    pub kept: Vec<(String, Kept)>,
}

impl ManualPlan {
    pub fn is_empty(&self) -> bool {
        self.delete.is_empty()
    }
}

/// The plan for keeping the newest `keep` uncommented manual (on-demand) snapshots of
/// `snapshots`, plus every commented one. `keep == 0` means the setting is off: nothing to
/// delete.
#[must_use]
pub fn manual(snapshots: &[Snapshot], keep: u32) -> ManualPlan {
    let mut plan = ManualPlan::default();
    if keep == 0 {
        return plan;
    }
    let newest = snapshots
        .iter()
        .max_by_key(|s| (s.created, &s.name))
        .map(|s| s.name.as_str());
    let mut manual: Vec<&Snapshot> = snapshots
        .iter()
        .filter(|s| s.tags.contains(&Tag::OnDemand))
        .collect();
    manual.sort_by_key(|s| Reverse((s.created, &s.name)));
    let keep = usize::try_from(keep).unwrap_or(usize::MAX);
    let mut counted = 0;
    for snapshot in manual {
        let kept = if snapshot
            .comment
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty())
        {
            Some(Kept::Comment)
        } else if counted < keep {
            counted += 1;
            Some(Kept::Recent)
        } else if snapshot.tags.iter().any(|&t| t != Tag::OnDemand) {
            Some(Kept::OtherTags)
        } else if Some(snapshot.name.as_str()) == newest {
            Some(Kept::Newest)
        } else {
            None
        };
        match kept {
            Some(reason) => plan.kept.push((snapshot.name.clone(), reason)),
            None => plan.delete.push(snapshot.name.clone()),
        }
    }
    plan.delete.reverse();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse_snapshot_name;

    fn snap(name: &str, tags: &[Tag], comment: Option<&str>) -> Snapshot {
        Snapshot {
            name: name.to_owned(),
            created: parse_snapshot_name(name).unwrap(),
            tags: tags.to_vec(),
            comment: comment.map(str::to_owned),
        }
    }

    fn o(name: &str) -> Snapshot {
        snap(name, &[Tag::OnDemand], None)
    }

    const DAYS: [&str; 6] = [
        "2026-09-20_10-00-00",
        "2026-09-21_10-00-00",
        "2026-09-22_10-00-00",
        "2026-09-23_10-00-00",
        "2026-09-24_10-00-00",
        "2026-09-25_10-00-00",
    ];

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn off_deletes_nothing() {
        let all: Vec<_> = DAYS.iter().map(|d| o(d)).collect();
        assert_eq!(manual(&all, 0), ManualPlan::default());
    }

    #[test]
    fn keeps_the_newest_n_and_deletes_the_rest_oldest_first() {
        // Given out of order: the plan sorts by date.
        let all: Vec<_> = DAYS.iter().rev().map(|d| o(d)).collect();
        let plan = manual(&all, 2);
        assert_eq!(plan.delete, names(&DAYS[..4]));
        assert_eq!(
            plan.kept,
            [
                (DAYS[5].to_owned(), Kept::Recent),
                (DAYS[4].to_owned(), Kept::Recent)
            ]
        );
    }

    #[test]
    fn exactly_n_or_fewer_deletes_nothing() {
        let all: Vec<_> = DAYS[..3].iter().map(|d| o(d)).collect();
        assert!(manual(&all, 3).is_empty());
        assert!(manual(&all, 999).is_empty());
        assert!(manual(&[], 1).is_empty());
    }

    #[test]
    fn commented_ones_are_pinned_and_not_counted() {
        let mut all: Vec<_> = DAYS.iter().map(|d| o(d)).collect();
        all[0].comment = Some("before upgrade".to_owned());
        all[5].comment = Some("newest, commented".to_owned());
        // A comment of spaces is no comment.
        all[1].comment = Some("   ".to_owned());
        let plan = manual(&all, 2);
        // The two newest uncommented ones (24, 23) are kept; the commented ones stay on top.
        assert_eq!(plan.delete, names(&DAYS[1..3]));
        assert_eq!(
            plan.kept,
            [
                (DAYS[5].to_owned(), Kept::Comment),
                (DAYS[4].to_owned(), Kept::Recent),
                (DAYS[3].to_owned(), Kept::Recent),
                (DAYS[0].to_owned(), Kept::Comment),
            ]
        );
    }

    #[test]
    fn only_commented_ones_deletes_nothing() {
        let all: Vec<_> = DAYS
            .iter()
            .map(|d| snap(d, &[Tag::OnDemand], Some("kept")))
            .collect();
        let plan = manual(&all, 1);
        assert!(plan.is_empty());
        assert!(plan.kept.iter().all(|(_, k)| *k == Kept::Comment));
    }

    #[test]
    fn uncommented_multi_tag_ones_count_towards_n() {
        // An on-demand snapshot that is also hourly is uncommented on-demand: it takes a slot.
        let all = vec![
            o(DAYS[0]),
            o(DAYS[1]),
            snap(DAYS[2], &[Tag::OnDemand, Tag::Hourly], None),
        ];
        let plan = manual(&all, 1);
        assert_eq!(plan.delete, names(&DAYS[..2]));
        assert_eq!(plan.kept, [(DAYS[2].to_owned(), Kept::Recent)]);
    }

    #[test]
    fn scheduled_and_multi_tag_snapshots_are_left_to_timeshift() {
        let all = vec![
            snap(DAYS[0], &[Tag::Daily], None),
            snap(DAYS[1], &[Tag::OnDemand, Tag::Hourly], None),
            o(DAYS[2]),
            snap(DAYS[3], &[Tag::Boot], None),
            o(DAYS[4]),
        ];
        let plan = manual(&all, 1);
        // Only on-demand-only ones go; daily and boot aren't manual at all.
        assert_eq!(plan.delete, names(&[DAYS[2]]));
        assert!(plan.kept.contains(&(DAYS[1].to_owned(), Kept::OtherTags)));
        assert!(!plan.kept.iter().any(|(n, _)| n == DAYS[0] || n == DAYS[3]));
    }

    #[test]
    fn untagged_snapshots_are_not_manual() {
        // Timeshift would delete a tagless one; this setting doesn't touch it.
        let all = vec![snap(DAYS[0], &[], None), o(DAYS[1]), o(DAYS[2])];
        let plan = manual(&all, 1);
        assert_eq!(plan.delete, names(&[DAYS[1]]));
    }

    #[test]
    fn the_newest_snapshot_is_never_deleted() {
        // With N >= 1 the newest manual one is always among the newest N; the `Newest` guard
        // is there in case that ever changes. Checked for every count and position.
        for keep in 1..=7 {
            for comment_at in 0..DAYS.len() {
                let mut all: Vec<_> = DAYS.iter().map(|d| o(d)).collect();
                all[comment_at].comment = Some("x".to_owned());
                let plan = manual(&all, keep);
                assert!(
                    !plan.delete.contains(&DAYS[5].to_owned()),
                    "{keep} {comment_at}"
                );
                assert_eq!(plan.delete.len() + plan.kept.len(), DAYS.len());
            }
        }
    }

    #[test]
    fn a_newer_scheduled_snapshot_doesnt_change_the_manual_count() {
        let mut all: Vec<_> = DAYS[..3].iter().map(|d| o(d)).collect();
        all.push(snap("2026-09-28_00-00-00", &[Tag::Hourly], None));
        let plan = manual(&all, 2);
        assert_eq!(plan.delete, names(&[DAYS[0]]));
    }
}
