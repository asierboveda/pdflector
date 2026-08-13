//! Frame timing and memory metrics for the debug overlay (docs/PLAN.md §3.5).
//!
//! `FrameTimer` keeps a ring buffer of the last frame durations and answers
//! percentile queries (the p95 frame time target is < 16.6 ms at 60 fps,
//! docs/PLAN.md §8); `read_rss_kb` reads the process's resident set size from
//! `/proc` for the RSS budget (< 150 MB on tablet).

use std::time::Duration;

/// Default number of samples kept by `FrameTimer::new` (≈ 10 s at 60 fps).
const DEFAULT_CAPACITY: usize = 600;

/// Rolling window of the last `capacity` frame durations.
///
/// Push is O(1) amortized; `p95` sorts the window on demand (the overlay calls
/// it at most once per frame, 600 `Duration` sorts are trivially cheap).
pub struct FrameTimer {
    /// Ring buffer; once full its length stays `capacity` forever and `next`
    /// points at the oldest sample.
    samples: Vec<Duration>,
    /// Index of the next slot to overwrite (== `samples.len()` until full).
    next: usize,
    capacity: usize,
}

impl FrameTimer {
    /// Timer with the default window of the last 600 samples.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Timer with a custom window size (`capacity == 0` disables recording).
    /// Exposed mainly so tests can exercise the ring-buffer wrap with small N.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            next: 0,
            capacity,
        }
    }

    /// Records one frame duration, overwriting the oldest sample when the
    /// window is full.
    pub fn push(&mut self, duration: Duration) {
        if self.capacity == 0 {
            return;
        }
        if self.samples.len() < self.capacity {
            self.samples.push(duration);
        } else {
            self.samples[self.next] = duration;
        }
        self.next = (self.next + 1) % self.capacity;
    }

    /// Samples in chronological order (oldest first), for percentile queries.
    fn ordered(&self) -> Vec<Duration> {
        // `next` is `samples.len()` until the buffer fills, so `split_at`
        // degrades gracefully before the first wrap.
        let (tail, head) = self.samples.split_at(self.next);
        head.iter().chain(tail.iter()).copied().collect()
    }

    /// 95th percentile of the samples in the window, nearest-rank method:
    /// the smallest sample with rank >= `ceil(0.95 * n)` in sorted order.
    /// `None` while no sample has been recorded.
    pub fn p95(&self) -> Option<Duration> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.ordered();
        sorted.sort();
        let rank = ((sorted.len() as f64) * 0.95).ceil() as usize;
        let idx = rank.saturating_sub(1).min(sorted.len() - 1);
        Some(sorted[idx])
    }

    /// Drops every recorded sample.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.next = 0;
    }
}

impl Default for FrameTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads the process's resident set size in kibibytes from
/// `/proc/self/status` (line `VmRSS:`).
///
/// Desktop Linux: always available. Android: `/proc` is usually mounted, but
/// the line may be missing or unreadable on locked-down devices — the API
/// then returns `None` and the overlay simply omits the RSS figure.
pub fn read_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("VmRSS:") {
            return parts.next()?.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known samples 1..=20 ms in a window of 64: sorted rank is
    /// ceil(0.95 * 20) = 19, so p95 is the 19th smallest = 19 ms.
    #[test]
    fn p95_of_known_samples() {
        let mut t = FrameTimer::with_capacity(64);
        for i in 1..=20u64 {
            t.push(Duration::from_millis(i));
        }
        assert_eq!(t.p95(), Some(Duration::from_millis(19)));
    }

    /// A single sample is its own p95.
    #[test]
    fn p95_single_sample() {
        let mut t = FrameTimer::with_capacity(64);
        t.push(Duration::from_millis(1234));
        assert_eq!(t.p95(), Some(Duration::from_millis(1234)));
    }

    /// The ring buffer keeps only the last N samples: pushing 1..=8 ms into a
    /// window of 4 leaves 5,6,7,8 ms, whose p95 is 8 ms.
    #[test]
    fn ring_buffer_keeps_last_n_samples() {
        let mut t = FrameTimer::with_capacity(4);
        for i in 1..=8u64 {
            t.push(Duration::from_millis(i));
        }
        assert_eq!(t.p95(), Some(Duration::from_millis(8)));
    }

    #[test]
    fn p95_empty_is_none() {
        assert_eq!(FrameTimer::with_capacity(4).p95(), None);
    }

    #[test]
    fn clear_resets_samples() {
        let mut t = FrameTimer::with_capacity(4);
        t.push(Duration::from_millis(5));
        t.clear();
        assert_eq!(t.p95(), None);
        t.push(Duration::from_millis(9));
        assert_eq!(t.p95(), Some(Duration::from_millis(9)));
    }

    /// Zero-capacity timer records nothing but never panics.
    #[test]
    fn zero_capacity_records_nothing() {
        let mut t = FrameTimer::with_capacity(0);
        t.push(Duration::from_millis(5));
        assert_eq!(t.p95(), None);
    }

    /// On Linux `/proc/self/status` exists and carries VmRSS; only assert
    /// that it is readable (never a specific value, which varies by machine
    /// and phase).
    #[cfg(target_os = "linux")]
    #[test]
    fn read_rss_kb_is_some_on_linux() {
        assert!(
            read_rss_kb().is_some(),
            "VmRSS must be readable from /proc/self/status on Linux"
        );
    }
}
