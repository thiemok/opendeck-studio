//! Decodes a quadrature encoder from its two phase levels, for boards without a pulse counter
//! (THI-84 KTD13).
//!
//! The ESP32-P4 build has no PCNT driver, so a GPIO interrupt reads both phases on every edge
//! and feeds them here. The count behaves like the ESP32's pulse counter: four counts per
//! full Gray-code cycle, and a reset to zero at ± the limit, so [`crate::encoder::DetentCounter`]
//! reads it unchanged.
//!
//! Every level change is decoded; none is skipped by time. Contact bounce toggles one phase
//! back and forth, which Gray-code decoding counts as +1 and −1 that cancel, and the encoder
//! module's RC filter on each phase removes short glitches. Skipping an edge instead would
//! leave the decoder's state stale and lose the next real step.

/// Quadrature state from two phase levels: bit 1 is phase A, bit 0 is phase B.
fn state(a: bool, b: bool) -> u8 {
    (u8::from(a) << 1) | u8::from(b)
}

pub struct Quadrature {
    state: u8,
    count: i16,
    limit: i16,
}

impl Quadrature {
    /// `a` and `b` are the phase levels when decoding starts. The count resets to zero when it
    /// reaches ± `limit`.
    pub fn new(a: bool, b: bool, limit: i16) -> Self {
        Self {
            state: state(a, b),
            count: 0,
            limit,
        }
    }

    /// Feeds the phase levels read after an edge on either phase.
    pub fn update(&mut self, a: bool, b: bool) {
        let next = state(a, b);
        // Clockwise in Gray code: 00 → 01 → 11 → 10 → 00.
        let step = match (self.state, next) {
            (0b00, 0b01) | (0b01, 0b11) | (0b11, 0b10) | (0b10, 0b00) => 1,
            (0b00, 0b10) | (0b10, 0b11) | (0b11, 0b01) | (0b01, 0b00) => -1,
            // No change, or both phases changed at once: an edge was missed, so the
            // direction is unknown.
            _ => 0,
        };
        self.state = next;
        self.count += step;
        if self.count.abs() >= self.limit {
            self.count = 0;
        }
    }

    /// The current count, as a pulse counter would report it.
    pub fn count(&self) -> i16 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::DetentCounter;

    const LIMIT: i16 = 4_000;

    fn feed(q: &mut Quadrature, levels: &[(bool, bool)]) {
        for &(a, b) in levels {
            q.update(a, b);
        }
    }

    const CLOCKWISE: [(bool, bool); 4] =
        [(false, true), (true, true), (true, false), (false, false)];

    #[test]
    fn a_clockwise_cycle_counts_four_and_the_reverse_minus_four() {
        let mut q = Quadrature::new(false, false, LIMIT);
        feed(&mut q, &CLOCKWISE);
        assert_eq!(q.count(), 4);

        let mut q = Quadrature::new(false, false, LIMIT);
        feed(
            &mut q,
            &[(true, false), (true, true), (false, true), (false, false)],
        );
        assert_eq!(q.count(), -4);
    }

    #[test]
    fn a_skipped_state_changes_no_count() {
        let mut q = Quadrature::new(false, false, LIMIT);
        feed(&mut q, &[(true, true)]);
        assert_eq!(q.count(), 0);
        feed(&mut q, &[(true, false)]);
        assert_eq!(q.count(), 1);
    }

    #[test]
    fn bounce_cancels_and_the_next_real_step_still_counts() {
        let mut q = Quadrature::new(false, false, LIMIT);
        // 00 → 01, then phase B bounces back and forth before settling at 01.
        feed(&mut q, &[(false, true), (false, false), (false, true)]);
        assert_eq!(q.count(), 1);
        // The next real step, 01 → 11, is only valid from the settled state.
        feed(&mut q, &[(true, true)]);
        assert_eq!(q.count(), 2);
    }

    #[test]
    fn a_repeated_state_changes_no_count() {
        let mut q = Quadrature::new(false, false, LIMIT);
        feed(&mut q, &[(false, false), (false, true), (false, true)]);
        assert_eq!(q.count(), 1);
    }

    #[test]
    fn the_count_resets_at_the_limit_like_a_pulse_counter() {
        let mut q = Quadrature::new(false, false, 4);
        feed(&mut q, &CLOCKWISE[..3]);
        assert_eq!(q.count(), 3);
        feed(&mut q, &CLOCKWISE[3..]);
        assert_eq!(q.count(), 0);
    }

    #[test]
    fn detent_counter_reads_one_detent_per_cycle() {
        let mut q = Quadrature::new(false, false, LIMIT);
        let mut detents = DetentCounter::new(4, LIMIT, q.count());
        let mut total = 0;
        for _ in 0..3 {
            for &(a, b) in &CLOCKWISE {
                q.update(a, b);
                total += detents.update(q.count());
            }
        }
        assert_eq!(total, 3);
    }
}
