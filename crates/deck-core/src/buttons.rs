//! What the button reader does after each read of the expander (THI-89, KTD4 in
//! `docs/plans/2026-11-21-1733-feat-mcp23017-key-inputs-bench-plan.md`).
//!
//! The firmware waits for the expander's interrupt line, reads the buttons, and keeps
//! reading every few milliseconds while a debouncer still needs samples. A read clears the
//! interrupt, so a line that stays low through settled reads means it isn't wired; the
//! reader then polls instead of spinning on the shared bus.

/// Settled reads in a row that must all find the interrupt line low before it counts as
/// stuck: about 100 ms at a 2 ms poll.
pub const INT_STUCK_POLLS: u32 = 50;

/// What to do after a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Nothing is settling and the interrupt line is high: wait for it to fall.
    WaitForInterrupt,
    /// Read again after the poll period. `warn_stuck` is true once, when the interrupt line
    /// has just been found stuck low.
    Poll { warn_stuck: bool },
}

/// Tracks the interrupt line across reads.
#[derive(Debug, Default)]
pub struct ReadLoop {
    low_polls: u32,
}

impl ReadLoop {
    pub const fn new() -> Self {
        Self { low_polls: 0 }
    }

    /// After a successful read: `settled` is whether every debouncer has counted its level,
    /// `int_low` the interrupt line's level now.
    pub fn after_read(&mut self, settled: bool, int_low: bool) -> Next {
        if !settled {
            return Next::Poll { warn_stuck: false };
        }
        if !int_low {
            self.low_polls = 0;
            return Next::WaitForInterrupt;
        }
        self.low_polls = self.low_polls.saturating_add(1);
        Next::Poll {
            warn_stuck: self.low_polls == INT_STUCK_POLLS,
        }
    }

    /// After a failed read (the caller has already backed off): whether to keep reading.
    /// The last good read cleared the interrupt, so a button still settling would never
    /// raise it again.
    pub fn after_failed_read(&self, settled: bool) -> bool {
        !settled
    }
}

/// What a read of the reference key means for the edge-to-detection measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    /// The press's first closed read: time it against the stamped edge.
    Sample,
    /// The key rests open again: drop any stamp the last press's bounces left.
    Rearm,
}

/// Picks one read per press to time (KTD5), however much the switch bounces. It rearms
/// once the key has read open for the whole debounce interval, so a bounce inside a press
/// doesn't rearm it and a contact too short to count still does.
#[derive(Debug)]
pub struct PressSampler {
    interval_ms: u64,
    armed: bool,
    open_since_ms: Option<u64>,
}

impl PressSampler {
    /// Starts armed, with the key open.
    pub const fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            armed: true,
            open_since_ms: None,
        }
    }

    /// False while the key is open but not yet rearmed, which takes further reads to see.
    pub fn settled(&self) -> bool {
        self.open_since_ms.is_none()
    }

    pub fn update(&mut self, closed: bool, now_ms: u64) -> Option<Reference> {
        if closed {
            self.open_since_ms = None;
            return core::mem::take(&mut self.armed).then_some(Reference::Sample);
        }
        if self.armed {
            return None;
        }
        let since = *self.open_since_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) < self.interval_ms {
            return None;
        }
        self.armed = true;
        self.open_since_ms = None;
        Some(Reference::Rearm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_polling_while_a_button_settles() {
        let mut r = ReadLoop::new();
        assert_eq!(r.after_read(false, true), Next::Poll { warn_stuck: false });
        assert_eq!(r.after_read(false, false), Next::Poll { warn_stuck: false });
    }

    #[test]
    fn waits_for_the_interrupt_once_settled_and_the_line_is_high() {
        let mut r = ReadLoop::new();
        assert_eq!(r.after_read(true, false), Next::WaitForInterrupt);
    }

    #[test]
    fn a_line_low_for_one_settled_read_is_a_new_press_not_a_fault() {
        let mut r = ReadLoop::new();
        assert_eq!(r.after_read(true, true), Next::Poll { warn_stuck: false });
        assert_eq!(r.after_read(true, false), Next::WaitForInterrupt);
    }

    #[test]
    fn warns_once_when_the_line_stays_low_through_settled_reads() {
        let mut r = ReadLoop::new();
        let warnings = (0..INT_STUCK_POLLS * 3)
            .filter(|_| r.after_read(true, true) == Next::Poll { warn_stuck: true })
            .count();
        assert_eq!(warnings, 1);
    }

    #[test]
    fn warns_again_after_the_line_recovered_and_got_stuck_again() {
        let mut r = ReadLoop::new();
        for _ in 0..INT_STUCK_POLLS {
            r.after_read(true, true);
        }
        r.after_read(true, false);
        let warned = (0..INT_STUCK_POLLS)
            .any(|_| r.after_read(true, true) == Next::Poll { warn_stuck: true });
        assert!(warned);
    }

    #[test]
    fn a_failed_read_keeps_reading_while_a_button_settles() {
        let r = ReadLoop::new();
        assert!(r.after_failed_read(false));
        assert!(!r.after_failed_read(true));
    }

    /// Feeds `(closed, at_ms)` reads and collects what they mean.
    fn sampled(reads: &[(bool, u64)]) -> Vec<(Reference, u64)> {
        let mut s = PressSampler::new(20);
        reads
            .iter()
            .filter_map(|&(closed, t)| s.update(closed, t).map(|r| (r, t)))
            .collect()
    }

    #[test]
    fn a_bounce_inside_a_press_takes_no_second_sample() {
        let reads = [(true, 0), (false, 2), (true, 4), (false, 8), (true, 10)];
        assert_eq!(sampled(&reads), [(Reference::Sample, 0)]);
    }

    #[test]
    fn a_release_rearms_once_open_for_the_interval() {
        let reads = [
            (true, 0),
            (true, 100),
            (false, 102),
            (false, 121),
            (false, 122),
            (false, 124),
            (true, 300),
        ];
        assert_eq!(
            sampled(&reads),
            [
                (Reference::Sample, 0),
                (Reference::Rearm, 122),
                (Reference::Sample, 300),
            ]
        );
    }

    #[test]
    fn a_bounce_during_the_release_restarts_the_interval() {
        let reads = [(true, 0), (false, 50), (true, 60), (false, 64), (false, 70)];
        assert_eq!(sampled(&reads), [(Reference::Sample, 0)]);
    }

    #[test]
    fn a_contact_too_short_to_count_still_rearms() {
        let reads = [(true, 0), (false, 4), (false, 24), (true, 500)];
        assert_eq!(
            sampled(&reads),
            [
                (Reference::Sample, 0),
                (Reference::Rearm, 24),
                (Reference::Sample, 500),
            ]
        );
    }

    #[test]
    fn needs_reads_only_while_open_and_not_yet_rearmed() {
        let mut s = PressSampler::new(20);
        assert!(s.settled(), "idle");
        s.update(true, 0);
        assert!(s.settled(), "held");
        s.update(false, 4);
        assert!(!s.settled(), "waiting out the interval");
        s.update(false, 24);
        assert!(s.settled(), "rearmed");
    }
}
