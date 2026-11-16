//! Page-change timing on a bench surface (THI-76, KTD5).
//!
//! A page change makes Companion send a `KEY-STATE` for every key. A burst starts at the
//! first such image and takes the oldest unanswered page-change press as its trigger. It
//! ends complete once every key has been drawn at least once, or partial after
//! [`QUIET_MS`] without an image. Each finished burst becomes one [`PageChange`]; the
//! press-to-last-key times of complete ones feed a rolling median.

use crate::latency::Samples;
use crate::session::MAX_BENCH_KEYS;

/// A burst with no new image for this long has ended.
pub const QUIET_MS: u64 = 1_000;
/// A press that no image answers within this long is not a page-change trigger.
pub const PRESS_MAX_AGE_MS: u64 = 5_000;
/// A summary is due after this many timed page changes.
pub const SUMMARY_EVERY: usize = 10;

/// One finished page change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageChange {
    /// Keys on the surface, and distinct keys drawn in this burst.
    pub keys: u8,
    pub drawn: u8,
    /// From the triggering press to the last key drawn; `None` without a press (the page
    /// changed from Companion itself).
    pub press_to_last_ms: Option<u32>,
    /// From the first image's first byte to the last key drawn.
    pub first_to_last_ms: u32,
    /// `KEY-STATE` lines drawn and their total length on the wire.
    pub images: u16,
    pub wire_bytes: u32,
    pub receive_ms_sum: u32,
    pub receive_ms_max: u32,
    pub decode_ms_sum: u32,
    pub decode_ms_max: u32,
}

impl PageChange {
    /// Every key was drawn.
    pub fn complete(&self) -> bool {
        self.drawn == self.keys
    }
}

/// One drawn bench-key image.
#[derive(Debug, Clone, Copy)]
pub struct Drawn {
    pub key: u8,
    /// When the image's line started arriving, and when its draw finished.
    pub line_started_ms: u64,
    pub done_ms: u64,
    pub wire_bytes: usize,
    pub receive_ms: u32,
    pub decode_ms: u32,
}

struct Burst {
    started_ms: u64,
    pressed_ms: Option<u64>,
    seen: u32,
    last_ms: u64,
    change: PageChange,
}

pub struct PageTimer {
    keys: u8,
    pressed_ms: Option<u64>,
    burst: Option<Burst>,
    samples: Samples,
}

impl PageTimer {
    /// A timer for a surface with `keys` bench keys (at most [`MAX_BENCH_KEYS`]).
    pub const fn new(keys: u8) -> Self {
        Self {
            keys: if keys > MAX_BENCH_KEYS {
                MAX_BENCH_KEYS
            } else {
                keys
            },
            pressed_ms: None,
            burst: None,
            samples: Samples::new(SUMMARY_EVERY),
        }
    }

    /// A page-change key was pressed. The oldest unanswered press is the trigger.
    pub fn pressed(&mut self, now_ms: u64) {
        match self.pressed_ms {
            Some(t) if now_ms.saturating_sub(t) <= PRESS_MAX_AGE_MS => {}
            _ => self.pressed_ms = Some(now_ms),
        }
    }

    /// A bench key finished drawing. Returns the page change this image completes.
    pub fn drawn(&mut self, image: Drawn) -> Option<PageChange> {
        if image.key >= self.keys {
            return None;
        }
        let burst = self.burst.get_or_insert_with(|| {
            let pressed_ms = self
                .pressed_ms
                .take()
                .filter(|&t| image.line_started_ms.saturating_sub(t) <= PRESS_MAX_AGE_MS);
            Burst {
                started_ms: image.line_started_ms,
                pressed_ms,
                seen: 0,
                last_ms: image.done_ms,
                change: PageChange {
                    keys: self.keys,
                    drawn: 0,
                    press_to_last_ms: None,
                    first_to_last_ms: 0,
                    images: 0,
                    wire_bytes: 0,
                    receive_ms_sum: 0,
                    receive_ms_max: 0,
                    decode_ms_sum: 0,
                    decode_ms_max: 0,
                },
            }
        });
        let bit = 1u32 << image.key;
        if burst.seen & bit == 0 {
            burst.seen |= bit;
            burst.change.drawn += 1;
        }
        burst.last_ms = image.done_ms;
        let c = &mut burst.change;
        c.images = c.images.saturating_add(1);
        c.wire_bytes = c
            .wire_bytes
            .saturating_add(u32::try_from(image.wire_bytes).unwrap_or(u32::MAX));
        c.receive_ms_sum = c.receive_ms_sum.saturating_add(image.receive_ms);
        c.receive_ms_max = c.receive_ms_max.max(image.receive_ms);
        c.decode_ms_sum = c.decode_ms_sum.saturating_add(image.decode_ms);
        c.decode_ms_max = c.decode_ms_max.max(image.decode_ms);
        if c.drawn == self.keys {
            self.finish()
        } else {
            None
        }
    }

    /// Periodic check: a burst quiet for [`QUIET_MS`] ends as partial.
    pub fn tick(&mut self, now_ms: u64) -> Option<PageChange> {
        let quiet = self
            .burst
            .as_ref()
            .is_some_and(|b| now_ms.saturating_sub(b.last_ms) >= QUIET_MS);
        if quiet { self.finish() } else { None }
    }

    fn finish(&mut self) -> Option<PageChange> {
        let burst = self.burst.take()?;
        let mut change = burst.change;
        change.first_to_last_ms = ms(burst.last_ms.saturating_sub(burst.started_ms));
        change.press_to_last_ms = burst
            .pressed_ms
            .map(|t| ms(burst.last_ms.saturating_sub(t)));
        if change.complete()
            && let Some(press_ms) = change.press_to_last_ms
        {
            self.samples.push(press_ms);
        }
        Some(change)
    }

    /// True once per [`SUMMARY_EVERY`] complete, press-timed page changes.
    pub fn summary_due(&mut self) -> bool {
        self.samples.summary_due()
    }

    /// Median and maximum press-to-last-key time over the recent window.
    pub fn median(&self) -> Option<u32> {
        self.samples.median()
    }

    pub fn max(&self) -> Option<u32> {
        self.samples.max()
    }
}

fn ms(elapsed: u64) -> u32 {
    u32::try_from(elapsed).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(key: u8, start: u64, done: u64) -> Drawn {
        Drawn {
            key,
            line_started_ms: start,
            done_ms: done,
            wire_bytes: 1_000 + usize::from(key),
            receive_ms: 20 + u32::from(key),
            decode_ms: 5,
        }
    }

    /// Draws keys `0..n` one after another, 25 ms apart from `start`; returns what the last
    /// one completed.
    fn page(timer: &mut PageTimer, n: u8, start: u64) -> Option<PageChange> {
        let mut last = None;
        for k in 0..n {
            let t = start + u64::from(k) * 25;
            last = timer.drawn(image(k, t, t + 20));
        }
        last
    }

    #[test]
    fn a_press_then_every_key_gives_one_complete_summary() {
        let mut timer = PageTimer::new(32);
        timer.pressed(0);
        let change = page(&mut timer, 32, 100).expect("complete");
        // The 32nd key starts at 100 + 31 × 25 and finishes 20 ms later.
        assert_eq!(change.press_to_last_ms, Some(895));
        assert_eq!(change.first_to_last_ms, 795);
        assert!(change.complete());
        assert_eq!((change.images, change.drawn), (32, 32));
        assert_eq!(change.wire_bytes, 32 * 1_000 + (0..32).sum::<u32>());
        assert_eq!(change.receive_ms_sum, 32 * 20 + (0..32).sum::<u32>());
        assert_eq!(change.receive_ms_max, 51);
        assert_eq!((change.decode_ms_sum, change.decode_ms_max), (160, 5));
        assert_eq!(timer.tick(10_000), None, "nothing left open");
    }

    #[test]
    fn a_key_drawn_twice_counts_once_toward_completion() {
        let mut timer = PageTimer::new(16);
        assert_eq!(timer.drawn(image(3, 0, 10)), None);
        assert_eq!(timer.drawn(image(3, 10, 20)), None);
        let change = page(&mut timer, 16, 30).expect("complete");
        assert_eq!((change.drawn, change.images), (16, 18));
        assert_eq!(change.wire_bytes, 18 * 1_000 + (0..16).sum::<u32>() + 6);
    }

    #[test]
    fn images_without_a_press_start_at_the_first_line() {
        let mut timer = PageTimer::new(16);
        let change = page(&mut timer, 16, 500).expect("complete");
        assert_eq!(change.press_to_last_ms, None);
        assert_eq!(change.first_to_last_ms, 15 * 25 + 20);
        assert!(!timer.summary_due());
        assert_eq!(timer.median(), None, "untimed page changes aren't sampled");
    }

    #[test]
    fn a_quiet_second_ends_a_partial_page_change() {
        let mut timer = PageTimer::new(32);
        timer.pressed(0);
        assert_eq!(page(&mut timer, 20, 100), None);
        let last_done = 100 + 19 * 25 + 20;
        assert_eq!(timer.tick(last_done + QUIET_MS - 1), None);
        let change = timer.tick(last_done + QUIET_MS).expect("partial");
        assert_eq!((change.drawn, change.keys), (20, 32));
        assert!(!change.complete());
        assert_eq!(
            change.press_to_last_ms,
            Some(u32::try_from(last_done).unwrap())
        );
        assert_eq!(timer.median(), None, "partial page changes aren't sampled");
    }

    #[test]
    fn a_stale_press_is_not_the_trigger() {
        let mut timer = PageTimer::new(16);
        timer.pressed(0);
        let change = page(&mut timer, 16, PRESS_MAX_AGE_MS + 1).unwrap();
        assert_eq!(change.press_to_last_ms, None);
    }

    #[test]
    fn a_press_during_a_burst_waits_for_the_next_one() {
        let mut timer = PageTimer::new(16);
        timer.pressed(0);
        page(&mut timer, 8, 100);
        timer.pressed(200);
        let first = page(&mut timer, 16, 300).unwrap();
        assert_eq!(first.press_to_last_ms, Some(300 + 15 * 25 + 20));
        let second = page(&mut timer, 16, 1_000).unwrap();
        assert_eq!(second.press_to_last_ms, Some(1_000 + 15 * 25 + 20 - 200));
    }

    #[test]
    fn keys_outside_the_surface_are_ignored() {
        let mut timer = PageTimer::new(16);
        assert_eq!(timer.drawn(image(16, 0, 10)), None);
        assert_eq!(timer.tick(10_000), None, "no burst was started");
    }

    #[test]
    fn a_summary_is_due_every_ten_timed_page_changes() {
        let mut timer = PageTimer::new(16);
        let mut due = Vec::new();
        for i in 0..20u64 {
            let t = i * 10_000;
            timer.pressed(t);
            page(&mut timer, 16, t + 100 + i).unwrap();
            if timer.summary_due() {
                due.push(i + 1);
            }
        }
        assert_eq!(due, vec![10, 20]);
        // Press-to-last is 100 + i + 395 for i in 0..20.
        assert_eq!(
            timer.median(),
            Some(504),
            "mean of 504 and 505, rounded down"
        );
        assert_eq!(timer.max(), Some(514));
    }
}
