// SPDX-License-Identifier: GPL-3.0-only

//! Progress of a snapshot being made: the percent and time left that
//! rsync prints while it runs, read from its output as it arrives.
//!
//! rsync redraws one terminal line with `\r`, so its stdout is split on `\r` as well as `\n`
//! ([`Segments`]) before [`parse_rsync`] looks at a piece.

use std::io::{self, Read};
use std::time::{Duration, Instant};

/// How far an operation is. `percent` and `eta_seconds` are `None` until there's a real
/// number.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// `0.0..=100.0`.
    pub percent: Option<f64>,
    pub eta_seconds: Option<u64>,
    /// The line it was read from, trimmed.
    pub text: String,
}

impl Progress {
    /// Whether there's a number worth showing: some progress made. `0%` (rsync prints it
    /// before anything is copied) is still "estimating".
    #[must_use]
    pub fn has_estimate(&self) -> bool {
        self.percent.is_some_and(|p| p > 0.0)
    }
}

/// rsync's `--info=progress2` line: `bytes percent% rate time [(xfr#N, to-chk=A/B)]`, e.g.
/// `  50,883,584  42%   11.72MB/s    0:00:05`. On a plain line the time is what rsync thinks is
/// left; on a line that ends a file (`(xfr#...)`) it is the time taken so far (checked with
/// rsync 3.2.7: it counts up there), so the time left is worked out from it and the percent.
/// `None` for any other text (file names with `-v`, `sending incremental file list`...).
#[must_use]
pub fn parse_rsync(segment: &str) -> Option<Progress> {
    let text = segment.trim();
    let mut tokens = text.split_whitespace();
    let bytes = tokens.next()?;
    if !bytes.bytes().all(|b| b.is_ascii_digit() || b == b',') {
        return None;
    }
    let percent = parse_percent(tokens.next()?.strip_suffix('%')?)?;
    let rate = tokens.next()?;
    if !rate.ends_with("/s") {
        return None;
    }
    let time = parse_clock(tokens.next()?)?;
    let eta_seconds = match tokens.next() {
        None => Some(time),
        Some(xfr) if xfr.starts_with("(xfr#") => eta_from_elapsed(time, percent),
        Some(_) => return None,
    };
    Some(Progress {
        percent: Some(percent),
        eta_seconds,
        text: text.to_owned(),
    })
}

/// Time left if `elapsed` seconds got `percent` done at the same pace. `None` before any.
fn eta_from_elapsed(elapsed: u64, percent: f64) -> Option<u64> {
    if percent >= 100.0 {
        return Some(0);
    }
    if percent <= 0.0 {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "seconds, rounded"
    )]
    let eta = (elapsed as f64 * (100.0 - percent) / percent).round() as u64;
    Some(eta)
}

/// `58.23` or ` 7` (a number from 0 to 100).
fn parse_percent(text: &str) -> Option<f64> {
    let text = text.trim();
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let value: f64 = text.parse().ok().filter(|_| digits)?;
    (0.0..=100.0).contains(&value).then_some(value)
}

/// `hh:mm:ss` or `h:mm:ss` (the hours can run past 99): seconds.
fn parse_clock(text: &str) -> Option<u64> {
    let mut parts = text.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let number = |part: &str| {
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse::<u64>().ok())
            .flatten()
    };
    let (h, m, s) = (number(h)?, number(m)?, number(s)?);
    (m < 60 && s < 60).then(|| h * 3600 + m * 60 + s)
}

/// rsync's file count at the end of a progress line: `to-chk=A/B` (A of B files still to check,
/// the list complete) or `ir-chk=A/B` (the same while rsync is still scanning, so B grows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checked {
    pub left: u64,
    pub total: u64,
    /// `to-chk`: `total` is final.
    pub complete: bool,
}

impl Checked {
    /// Reads the count from a progress line's text; `None` when it has none.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (complete, rest) = if let Some((_, rest)) = text.split_once("to-chk=") {
            (true, rest)
        } else {
            (false, text.split_once("ir-chk=")?.1)
        };
        let (left, rest) = rest.split_once('/')?;
        let total = rest.split(|c: char| !c.is_ascii_digit()).next()?;
        let left: u64 = left.parse().ok()?;
        let total: u64 = total.parse().ok()?;
        (left <= total && total > 0).then_some(Self {
            left,
            total,
            complete,
        })
    }

    /// Share of the files checked, `0.0..=100.0`.
    #[must_use]
    pub fn percent(self) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "a percent")]
        let done = (self.total - self.left) as f64 / self.total as f64;
        done * 100.0
    }
}

/// Seconds of copying before a time left is shown: rsync's pace in the first seconds says
/// little about the rest.
pub const ETA_AFTER_SECONDS: u64 = 30;
/// Percent done before a time left is shown.
pub const ETA_AFTER_PERCENT: f64 = 5.0;

/// Turns a create's rsync progress lines into progress that only goes forward.
///
/// rsync's own percent can't be shown as it is: with incremental recursion its total grows
/// while it runs, so the percent jumps back (`rsync_argv` turns that off), and with
/// `--link-dest` it counts only the bytes copied, so a snapshot of a mostly unchanged system
/// ends at a few percent. So: nothing (`percent: None`, "scanning") until rsync's file list is
/// complete (the first `to-chk`), then the larger of files checked and rsync's byte percent,
/// never less than before. The time left is worked out here from the whole run's pace, and
/// only once [`ETA_AFTER_SECONDS`] and [`ETA_AFTER_PERCENT`] have passed; rsync's figure
/// follows the pace of the last few files.
#[derive(Debug, Default)]
pub struct CreateProgress {
    /// When the file list was complete, and the percent then.
    counting_since: Option<(Instant, f64)>,
    /// The last files-checked percent seen.
    files: f64,
    /// The highest percent handed out.
    shown: f64,
}

impl CreateProgress {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The progress to show for rsync's `line`, read at `now`.
    pub fn update(&mut self, line: &Progress, now: Instant) -> Progress {
        let checked = Checked::parse(&line.text);
        // An `ir-chk` count (still scanning) has a total that will grow: not used.
        if let Some(checked) = checked.filter(|c| c.complete) {
            self.files = self.files.max(checked.percent());
        }
        let counting = checked.is_some_and(|c| c.complete) || self.counting_since.is_some();
        if !counting {
            return Progress {
                percent: None,
                eta_seconds: None,
                text: line.text.clone(),
            };
        }
        let bytes = line.percent.unwrap_or(0.0);
        let percent = self.shown.max(self.files).max(bytes).min(100.0);
        self.shown = percent;
        let (since, at_start) = *self.counting_since.get_or_insert((now, percent));
        let eta_seconds = if percent >= 100.0 {
            Some(0)
        } else {
            let elapsed = now.saturating_duration_since(since).as_secs();
            let done = percent - at_start;
            (elapsed >= ETA_AFTER_SECONDS && percent >= ETA_AFTER_PERCENT && done > 0.0).then(
                || {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        clippy::cast_precision_loss,
                        reason = "seconds, rounded"
                    )]
                    let eta = (elapsed as f64 * (100.0 - percent) / done).round() as u64;
                    eta
                },
            )
        };
        Progress {
            percent: Some(percent),
            eta_seconds,
            text: line.text.clone(),
        }
    }
}

/// Splits a byte stream into pieces ending at `\r` or `\n`, as a terminal would draw them.
/// Invalid UTF-8 is replaced.
#[derive(Debug, Default)]
pub struct Segments {
    pending: Vec<u8>,
}

impl Segments {
    /// Adds `bytes`; calls `each` for every piece they complete.
    pub fn push(&mut self, bytes: &[u8], mut each: impl FnMut(&str)) {
        for &byte in bytes {
            if byte == b'\r' || byte == b'\n' {
                each(&String::from_utf8_lossy(&self.pending));
                self.pending.clear();
            } else {
                self.pending.push(byte);
            }
        }
    }

    /// The last piece, if the stream didn't end with `\r` or `\n`.
    pub fn finish(self, mut each: impl FnMut(&str)) {
        if !self.pending.is_empty() {
            each(&String::from_utf8_lossy(&self.pending));
        }
    }
}

/// Reads `stdout` to the end, handing each piece ([`Segments`]) to `on_segment`. The pieces it
/// doesn't take (returns `false` for) are kept, one per line, and returned when `keep` is set:
/// progress lines are taken, so they don't end up among the output that is parsed or shown
/// after the run.
///
/// # Errors
///
/// Reading fails.
pub fn read_segments(
    mut stdout: impl Read,
    keep: bool,
    on_segment: &mut dyn FnMut(&str) -> bool,
) -> io::Result<String> {
    let mut kept = String::new();
    let mut segments = Segments::default();
    let mut buffer = [0_u8; 8192];
    let mut take = |segment: &str| {
        if !on_segment(segment) && keep {
            kept.push_str(segment);
            kept.push('\n');
        }
    };
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => segments.push(&buffer[..n], &mut take),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    segments.finish(&mut take);
    Ok(kept)
}

/// Lets something through at most once per `interval` (the first time always).
#[derive(Debug)]
pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// Whether to let one through at `now`; if so, the next waits `interval` from `now`.
    pub fn ready(&mut self, now: Instant) -> bool {
        let ready = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= self.interval);
        if ready {
            self.last = Some(now);
        }
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsync_lines() {
        let p = parse_rsync("     50,883,584  42%   11.72MB/s    0:00:05  ").unwrap();
        assert_eq!((p.percent, p.eta_seconds), (Some(42.0), Some(5)));
        // After a file: elapsed time, so 25% in 2 s leaves 6 s.
        let p = parse_rsync("     30,000,000  25%   12.01MB/s    0:00:02 (xfr#10, to-chk=30/41)")
            .unwrap();
        assert_eq!((p.percent, p.eta_seconds), (Some(25.0), Some(6)));
        let p = parse_rsync("  1,000 100%  1.00kB/s  0:00:09 (xfr#1, ir-chk=0/1)").unwrap();
        assert_eq!(p.eta_seconds, Some(0));
        let p = parse_rsync("  32,768   0%    0.00kB/s    0:00:00 (xfr#1, to-chk=3/4)").unwrap();
        assert_eq!((p.percent, p.eta_seconds), (Some(0.0), None));
        for other in [
            "sending incremental file list",
            "etc/hosts",
            "APSIS >f+++++++++ 12 etc/hosts",
            "12  5%  1MB 0:00:01",
            "12 5% 1MB/s",
            "12 5% 1MB/s 0:00:01 extra",
            "",
        ] {
            assert_eq!(parse_rsync(other), None, "{other}");
        }
    }

    #[test]
    fn checked_counts() {
        let to = Checked::parse("1,000  2%  1.00kB/s  0:00:09 (xfr#5, to-chk=750/1000)").unwrap();
        assert_eq!((to.left, to.total, to.complete), (750, 1000, true));
        assert!((to.percent() - 25.0).abs() < 1e-9);
        let ir = Checked::parse("1,000  2%  1.00kB/s  0:00:09 (xfr#5, ir-chk=10/40)").unwrap();
        assert!(!ir.complete);
        for other in [
            "1,000  2%  1.00kB/s  0:00:09",
            "x to-chk=5/0)",
            "x to-chk=9/5)",
            "x to-chk=/5)",
        ] {
            assert_eq!(Checked::parse(other), None, "{other}");
        }
    }

    /// Feeds `lines` a second apart from `start`; the percents and times left handed out.
    fn feed(lines: &[&str], start: Instant) -> Vec<(Option<f64>, Option<u64>)> {
        let mut tracker = CreateProgress::new();
        lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let progress = parse_rsync(line).unwrap();
                let now = start + Duration::from_secs(u64::try_from(i).unwrap());
                let out = tracker.update(&progress, now);
                (out.percent, out.eta_seconds)
            })
            .collect()
    }

    /// Incremental recursion: rsync's percent jumps back while it scans. Nothing is shown
    /// until the list is complete, and then only forward.
    #[test]
    fn scanning_until_the_file_list_is_complete_then_only_forward() {
        let seen = feed(
            &[
                "  0   0%  0.00kB/s  0:00:00",
                "  9  36%  1.00kB/s  0:00:09 (xfr#1, ir-chk=10/20)",
                "  9   1%  1.00kB/s  0:00:09 (xfr#2, ir-chk=10/900)",
                "  9  40%  1.00kB/s  0:00:09 (xfr#3, to-chk=500/1000)",
                "  9  13%  1.00kB/s  0:00:09",
                "  9  45%  1.00kB/s  0:00:09 (xfr#4, to-chk=600/1000)",
                "  9  70%  1.00kB/s  0:00:09 (xfr#5, to-chk=0/1000)",
            ],
            Instant::now(),
        );
        let percents: Vec<_> = seen.iter().map(|s| s.0).collect();
        assert_eq!(
            percents,
            [
                None,
                None,
                None,
                Some(50.0),
                Some(50.0),
                Some(50.0),
                Some(100.0)
            ]
        );
        assert_eq!(seen.last().unwrap().1, Some(0));
    }

    /// `--link-dest`: rsync's byte percent stays low (unchanged files aren't copied); the
    /// files checked carry the run to 100%.
    #[test]
    fn link_dest_runs_end_at_100_percent() {
        let seen = feed(
            &[
                "  9   0%  1.00kB/s  0:00:00 (xfr#1, to-chk=900/1000)",
                "  9   1%  1.00kB/s  0:00:00 (xfr#2, to-chk=400/1000)",
                "  9   2%  1.00kB/s  0:00:00 (xfr#3, to-chk=0/1000)",
            ],
            Instant::now(),
        );
        let percents: Vec<_> = seen.iter().map(|s| s.0).collect();
        assert_eq!(percents, [Some(10.0), Some(60.0), Some(100.0)]);
    }

    /// No time left in the first seconds or the first percent; then the whole run's pace.
    #[test]
    fn time_left_only_once_it_means_something() {
        let mut tracker = CreateProgress::new();
        let start = Instant::now();
        let at = |tracker: &mut CreateProgress, secs: u64, left: u64| {
            let line = format!("  9   0%  1.00kB/s  0:00:01 (xfr#1, to-chk={left}/1000)");
            tracker.update(
                &parse_rsync(&line).unwrap(),
                start + Duration::from_secs(secs),
            )
        };
        assert_eq!(at(&mut tracker, 0, 1000).eta_seconds, None);
        // 20% after 10 s: too early.
        assert_eq!(at(&mut tracker, 10, 800).eta_seconds, None);
        // 3% after 40 s would be too little done, but percent never goes back: still 20%.
        let p = at(&mut tracker, 40, 970);
        assert_eq!(p.percent, Some(20.0));
        // 20% in 40 s: 160 s for the other 80%.
        assert_eq!(p.eta_seconds, Some(160));
        // rsync's own figure (0:00:01) isn't used.
        assert_eq!(at(&mut tracker, 60, 500).eta_seconds, Some(60));
    }

    #[test]
    fn segments_split_on_carriage_returns_and_newlines() {
        let mut pieces = Vec::new();
        let mut segments = Segments::default();
        segments.push(b" 1% a\r 2%", |s| pieces.push(s.to_owned()));
        segments.push(b" b\rline\nlast", |s| pieces.push(s.to_owned()));
        segments.finish(|s| pieces.push(s.to_owned()));
        assert_eq!(pieces, [" 1% a", " 2% b", "line", "last"]);
    }

    #[test]
    fn read_segments_keeps_what_is_not_taken() {
        let input = &b"sending incremental file list\n  1,024   5%  1.00kB/s  0:00:10\r  \
2,048   9%  1.00kB/s  0:00:09\r    \rrsync: failed\n"[..];
        let mut seen = Vec::new();
        let kept = read_segments(input, true, &mut |s| {
            let progress = parse_rsync(s);
            seen.extend(progress.clone());
            progress.is_some()
        })
        .unwrap();
        assert_eq!(kept, "sending incremental file list\n    \nrsync: failed\n");
        assert_eq!(seen.len(), 2);
        let dropped = read_segments(input, false, &mut |_| false).unwrap();
        assert!(dropped.is_empty());
    }

    #[test]
    fn throttle_lets_one_through_per_interval() {
        let start = Instant::now();
        let mut throttle = Throttle::new(Duration::from_millis(500));
        assert!(throttle.ready(start));
        assert!(!throttle.ready(start + Duration::from_millis(499)));
        assert!(throttle.ready(start + Duration::from_millis(500)));
        assert!(!throttle.ready(start + Duration::from_millis(600)));
    }
}
