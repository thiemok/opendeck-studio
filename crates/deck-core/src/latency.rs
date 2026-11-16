//! Press-to-image round trips, measured on the device (KTD27).
//!
//! A sent press or rotation starts a timer for its control; the next completed draw of that
//! control stops it. A burst is timed from its first unanswered send. The last
//! [`WINDOW`] samples give a median and a maximum for the log.

use crate::session::Control;

/// A draw this long after the send is not counted as its answer.
pub const MAX_AGE_MS: u64 = 2_000;
/// How many recent samples the summary covers.
pub const WINDOW: usize = 64;
/// A summary is due after this many new samples.
pub const SUMMARY_EVERY: usize = 20;

pub struct RoundTrips {
    pending: [Option<u64>; 3],
    samples: Samples,
}

/// The last [`WINDOW`] samples in milliseconds, with a summary due every `every` new ones.
pub struct Samples {
    samples: [u32; WINDOW],
    len: usize,
    next: usize,
    since_summary: usize,
    every: usize,
}

impl Samples {
    pub const fn new(every: usize) -> Self {
        Self {
            samples: [0; WINDOW],
            len: 0,
            next: 0,
            since_summary: 0,
            every,
        }
    }

    pub fn push(&mut self, ms: u32) {
        self.samples[self.next] = ms;
        self.next = (self.next + 1) % WINDOW;
        self.len = (self.len + 1).min(WINDOW);
        self.since_summary += 1;
    }

    /// True once per `every` samples.
    pub fn summary_due(&mut self) -> bool {
        if self.since_summary < self.every {
            return false;
        }
        self.since_summary = 0;
        true
    }

    /// Median of the window; the mean of the middle two when the count is even.
    pub fn median(&self) -> Option<u32> {
        if self.len == 0 {
            return None;
        }
        let mut sorted = self.samples;
        let sorted = &mut sorted[..self.len];
        sorted.sort_unstable();
        let mid = self.len / 2;
        if self.len.is_multiple_of(2) {
            let sum = u64::from(sorted[mid - 1]) + u64::from(sorted[mid]);
            u32::try_from(sum / 2).ok()
        } else {
            Some(sorted[mid])
        }
    }

    pub fn max(&self) -> Option<u32> {
        self.samples[..self.len].iter().copied().max()
    }
}

impl Default for RoundTrips {
    fn default() -> Self {
        Self::new()
    }
}

impl RoundTrips {
    pub const fn new() -> Self {
        Self {
            pending: [None; 3],
            samples: Samples::new(SUMMARY_EVERY),
        }
    }

    /// A press or rotation for `control` went out. Bench keys are timed per page change
    /// instead (`pagetime`), so they are ignored here.
    pub fn sent(&mut self, control: Control, now_ms: u64) {
        let Some(slot) = slot(control) else {
            return;
        };
        let pending = &mut self.pending[slot];
        match *pending {
            Some(sent) if now_ms.saturating_sub(sent) <= MAX_AGE_MS => {}
            _ => *pending = Some(now_ms),
        }
    }

    /// `control` finished drawing; returns the round trip in milliseconds when it answers a send.
    pub fn drawn(&mut self, control: Control, now_ms: u64) -> Option<u32> {
        let sent = self.pending[slot(control)?].take()?;
        let elapsed = now_ms.saturating_sub(sent);
        if elapsed > MAX_AGE_MS {
            return None;
        }
        let ms = u32::try_from(elapsed).unwrap_or(u32::MAX);
        self.samples.push(ms);
        Some(ms)
    }

    /// True once per [`SUMMARY_EVERY`] samples.
    pub fn summary_due(&mut self) -> bool {
        self.samples.summary_due()
    }

    pub fn median(&self) -> Option<u32> {
        self.samples.median()
    }

    pub fn max(&self) -> Option<u32> {
        self.samples.max()
    }
}

fn slot(control: Control) -> Option<usize> {
    match control {
        Control::KeyA => Some(0),
        Control::KeyB => Some(1),
        Control::Encoder => Some(2),
        Control::Bench(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_answers_the_send_for_its_control() {
        let mut rt = RoundTrips::new();
        rt.sent(Control::KeyA, 100);
        assert_eq!(rt.drawn(Control::KeyA, 160), Some(60));
        assert_eq!(rt.drawn(Control::KeyA, 200), None, "answered only once");
    }

    #[test]
    fn a_draw_for_another_control_is_not_an_answer() {
        let mut rt = RoundTrips::new();
        rt.sent(Control::KeyA, 100);
        assert_eq!(rt.drawn(Control::KeyB, 150), None);
        assert_eq!(rt.drawn(Control::KeyA, 170), Some(70));
    }

    #[test]
    fn bench_keys_are_not_timed_here() {
        let mut rt = RoundTrips::new();
        rt.sent(Control::Bench(0), 100);
        assert_eq!(rt.drawn(Control::Bench(0), 150), None);
    }

    #[test]
    fn a_draw_with_nothing_pending_is_not_a_sample() {
        let mut rt = RoundTrips::new();
        assert_eq!(rt.drawn(Control::Encoder, 50), None);
    }

    #[test]
    fn a_burst_is_timed_from_its_first_send() {
        let mut rt = RoundTrips::new();
        for t in [100, 120, 140] {
            rt.sent(Control::Encoder, t);
        }
        assert_eq!(rt.drawn(Control::Encoder, 200), Some(100));
    }

    #[test]
    fn a_stale_send_is_dropped() {
        let mut rt = RoundTrips::new();
        rt.sent(Control::KeyA, 0);
        assert_eq!(rt.drawn(Control::KeyA, 2_001), None);
        assert_eq!(rt.drawn(Control::KeyA, 2_100), None, "cleared");
        // A new send after a stale one starts a fresh timer.
        rt.sent(Control::KeyB, 0);
        rt.sent(Control::KeyB, 3_000);
        assert_eq!(rt.drawn(Control::KeyB, 3_050), Some(50));
    }

    #[test]
    fn median_and_max_cover_the_last_window() {
        let mut rt = RoundTrips::new();
        assert_eq!((rt.median(), rt.max()), (None, None));
        for (i, ms) in [10, 20, 30, 40].into_iter().enumerate() {
            let t = i as u64 * 10_000;
            rt.sent(Control::KeyA, t);
            rt.drawn(Control::KeyA, t + ms);
        }
        assert_eq!(rt.median(), Some(25));
        assert_eq!(rt.max(), Some(40));

        // 70 more samples of 5 ms push the earlier ones out of the 64-sample window.
        for i in 0..70u64 {
            let t = 100_000 + i * 10_000;
            rt.sent(Control::KeyA, t);
            rt.drawn(Control::KeyA, t + 5);
        }
        assert_eq!(rt.median(), Some(5));
        assert_eq!(rt.max(), Some(5));
    }

    #[test]
    fn a_summary_is_due_every_twenty_samples() {
        let mut rt = RoundTrips::new();
        let mut due = Vec::new();
        for i in 0..40u64 {
            rt.sent(Control::KeyA, i * 10_000);
            rt.drawn(Control::KeyA, i * 10_000 + 1);
            if rt.summary_due() {
                due.push(i + 1);
            }
        }
        assert_eq!(due, vec![20, 40]);
    }
}
