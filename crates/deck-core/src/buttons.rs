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

    /// After a failed read (the caller has already backed off). The last good read cleared
    /// the interrupt, so a button still settling would never raise it again: keep reading.
    pub fn after_failed_read(&self, settled: bool) -> Next {
        if settled {
            Next::WaitForInterrupt
        } else {
            Next::Poll { warn_stuck: false }
        }
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
        assert_eq!(r.after_failed_read(false), Next::Poll { warn_stuck: false });
        assert_eq!(r.after_failed_read(true), Next::WaitForInterrupt);
    }
}
