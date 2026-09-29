// SPDX-License-Identifier: GPL-3.0-only

//! "Keep the last N snapshots": which snapshots to delete. Pure; the applet shows the plan and
//! deletes only after `y`, one snapshot at a time through the helper. Nothing is ever deleted
//! without that.
//!
//! Commented snapshots are pinned: they are never removed and don't count towards N, so N is
//! the number of uncommented snapshots kept. Tags don't matter (snapshots Timeshift's schedule
//! took count like any other). The newest snapshot is never deleted.

use std::cmp::Reverse;

use crate::model::Snapshot;

/// Why a manual snapshot past the count stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// One of the newest N uncommented snapshots.
    Recent,
    /// It has a comment: pinned, and not counted towards N.
    Comment,
    /// The newest snapshot of all. Can't happen with a count of at least 1 (it's then among
    /// the newest N); kept as a guard.
    Newest,
}

/// What keeping the last `keep` snapshots means for a list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManualPlan {
    /// Names to delete, oldest first.
    pub delete: Vec<String>,
    /// Every other snapshot and why it stays, newest first.
    pub kept: Vec<(String, Kept)>,
}

impl ManualPlan {
    pub fn is_empty(&self) -> bool {
        self.delete.is_empty()
    }
}

/// The plan for keeping the newest `keep` uncommented snapshots of `snapshots`, plus every
/// commented one. `keep == 0` means the setting is off: nothing to
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
    let mut manual: Vec<&Snapshot> = snapshots.iter().collect();
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
    use crate::model::{Tag, parse_snapshot_name};

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
    fn tags_dont_matter() {
        // Snapshots Timeshift's schedule took, or with no tag at all, count like the others.
        let all = vec![
            snap(DAYS[0], &[Tag::Daily], None),
            snap(DAYS[1], &[Tag::OnDemand, Tag::Hourly], None),
            snap(DAYS[2], &[], None),
            snap(DAYS[3], &[Tag::Boot], Some("pinned")),
            o(DAYS[4]),
        ];
        let plan = manual(&all, 1);
        assert_eq!(plan.delete, names(&DAYS[..3]));
        assert_eq!(
            plan.kept,
            [
                (DAYS[4].to_owned(), Kept::Recent),
                (DAYS[3].to_owned(), Kept::Comment)
            ]
        );
    }

    #[test]
    fn the_newest_snapshot_is_never_deleted() {
        // With N >= 1 the newest one is always among the newest N; the `Newest` guard
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
}
