// SPDX-License-Identifier: GPL-3.0-only

//! Progress of a long operation (a create or a restore): the percent and time left that
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

    /// This progress as part `index` (from 0) of `count` equal parts: a restore that runs
    /// rsync once per path. The time left is only known for the last part.
    #[must_use]
    pub fn part_of(mut self, index: usize, count: usize) -> Self {
        if count > 1 {
            #[allow(clippy::cast_precision_loss, reason = "a handful of paths")]
            let (index, count) = (index as f64, count as f64);
            self.percent = self.percent.map(|p| (index * 100.0 + p) / count);
            if index + 1.0 < count {
                self.eta_seconds = None;
            }
        }
        self
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
    fn parts_of_a_restore() {
        let half = Progress {
            percent: Some(50.0),
            eta_seconds: Some(10),
            text: String::new(),
        };
        let first = half.clone().part_of(0, 2);
        assert_eq!((first.percent, first.eta_seconds), (Some(25.0), None));
        let last = half.clone().part_of(1, 2);
        assert_eq!((last.percent, last.eta_seconds), (Some(75.0), Some(10)));
        assert_eq!(half.clone().part_of(0, 1), half);
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
