// SPDX-License-Identifier: GPL-3.0-only

//! Progress parsing on real output. `rsync-progress2.txt` is `rsync -a --info=progress2
//! --bwlimit=12000` of 40 files of 3 MB (random data) with rsync 3.2.7: only byte counts,
//! rates and times, `\r` between updates.

use apsis_core::progress::{parse_rsync, read_segments};

const RSYNC: &[u8] = include_bytes!("fixtures/rsync-progress2.txt");

#[test]
fn every_rsync_progress2_update_is_read() {
    let mut seen = Vec::new();
    let kept = read_segments(RSYNC, true, &mut |segment| {
        let progress = parse_rsync(segment);
        seen.extend(progress.clone());
        progress.is_some() || segment.trim().is_empty()
    })
    .unwrap();
    // Nothing but progress in that output.
    assert_eq!(kept, "");
    assert!(seen.len() > 40, "{}", seen.len());
    let percents: Vec<f64> = seen.iter().filter_map(|p| p.percent).collect();
    assert_eq!(percents.len(), seen.len());
    assert!(percents.windows(2).all(|w| w[0] <= w[1]), "{percents:?}");
    assert!(!seen[0].has_estimate(), "starts at 0%");
    let last = seen.last().unwrap();
    assert_eq!((last.percent, last.eta_seconds), (Some(100.0), Some(0)));
    // Somewhere in the middle there's a real estimate: about 10 s in all at 12 MB/s.
    let middle = seen.iter().find(|p| p.percent >= Some(40.0)).unwrap();
    let eta = middle.eta_seconds.unwrap();
    assert!((2..=10).contains(&eta), "{middle:?}");
}
