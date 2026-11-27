//! Decodes a quadrature encoder from its two phase levels, for boards without a pulse counter
//! (THI-84 KTD13).
//!
//! The ESP32-P4 build has no PCNT driver, so a GPIO interrupt reads both phases on every edge
//! and feeds them here. The count behaves like the ESP32's pulse counter: four counts per
//! full Gray-code cycle, and a reset to zero at ± the limit, so [`crate::encoder::DetentCounter`]
//! reads it unchanged. A minimum edge interval stands in for PCNT's glitch filter.

/// Quadrature state from two phase levels: bit 1 is phase A, bit 0 is phase B.
fn state(a: bool, b: bool) -> u8 {
    (u8::from(a) << 1) | u8::from(b)
}

pub struct Quadrature {
    state: u8,
    count: i16,
    limit: i16,
    min_interval_us: u64,
    last_edge_us: Option<u64>,
}

impl Quadrature {
    /// `a` and `b` are the phase levels when decoding starts. The count resets to zero when it
    /// reaches ± `limit`; edges closer than `min_interval_us` to the last accepted one are
    /// ignored as contact bounce.
    pub fn new(a: bool, b: bool, limit: i16, min_interval_us: u64) -> Self {
        Self {
            state: state(a, b),
            count: 0,
            limit,
            min_interval_us,
            last_edge_us: None,
        }
    }

    /// Feeds the phase levels read at `now_us` after an edge on either phase.
    pub fn update(&mut self, a: bool, b: bool, now_us: u64) {
        let next = state(a, b);
        if next == self.state {
            return;
        }
        if let Some(last) = self.last_edge_us
            && now_us.saturating_sub(last) < self.min_interval_us
        {
            return;
        }
        // Clockwise in Gray code: 00 → 01 → 11 → 10 → 00.
        let step = match (self.state, next) {
            (0b00, 0b01) | (0b01, 0b11) | (0b11, 0b10) | (0b10, 0b00) => 1,
            (0b00, 0b10) | (0b10, 0b11) | (0b11, 0b01) | (0b01, 0b00) => -1,
            // Both phases changed at once: an edge was missed, so the direction is unknown.
            _ => 0,
        };
        self.state = next;
        self.last_edge_us = Some(now_us);
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
    const INTERVAL: u64 = 100;

    /// Feeds `(a, b)` levels 1 ms apart, well past the edge interval, carrying on from the
    /// last accepted edge.
    fn feed(q: &mut Quadrature, levels: &[(bool, bool)]) {
        let mut now = q.last_edge_us.unwrap_or(0);
        for &(a, b) in levels {
            now += 1_000;
            q.update(a, b, now);
        }
    }

    const CLOCKWISE: [(bool, bool); 4] =
        [(false, true), (true, true), (true, false), (false, false)];

    #[test]
    fn a_clockwise_cycle_counts_four_and_the_reverse_minus_four() {
        let mut q = Quadrature::new(false, false, LIMIT, INTERVAL);
        feed(&mut q, &CLOCKWISE);
        assert_eq!(q.count(), 4);

        let mut q = Quadrature::new(false, false, LIMIT, INTERVAL);
        feed(
            &mut q,
            &[(true, false), (true, true), (false, true), (false, false)],
        );
        assert_eq!(q.count(), -4);
    }

    #[test]
    fn a_skipped_state_changes_no_count() {
        let mut q = Quadrature::new(false, false, LIMIT, INTERVAL);
        feed(&mut q, &[(true, true)]);
        assert_eq!(q.count(), 0);
        // Decoding carries on from the new state.
        feed(&mut q, &[(true, false)]);
        assert_eq!(q.count(), 1);
    }

    #[test]
    fn an_edge_inside_the_interval_is_ignored_as_bounce() {
        let mut q = Quadrature::new(false, false, LIMIT, INTERVAL);
        q.update(false, true, 1_000);
        q.update(false, false, 1_050);
        q.update(false, true, 1_080);
        assert_eq!(q.count(), 1);
    }

    #[test]
    fn a_repeated_state_changes_no_count() {
        let mut q = Quadrature::new(false, false, LIMIT, INTERVAL);
        feed(&mut q, &[(false, false), (false, true), (false, true)]);
        assert_eq!(q.count(), 1);
    }

    #[test]
    fn the_count_resets_at_the_limit_like_a_pulse_counter() {
        let mut q = Quadrature::new(false, false, 4, INTERVAL);
        feed(&mut q, &CLOCKWISE[..3]);
        assert_eq!(q.count(), 3);
        feed(&mut q, &CLOCKWISE[3..]);
        assert_eq!(q.count(), 0);
    }

    #[test]
    fn detent_counter_reads_one_detent_per_cycle() {
        let mut q = Quadrature::new(false, false, LIMIT, INTERVAL);
        let mut detents = DetentCounter::new(4, LIMIT, q.count());
        let mut total = 0;
        let mut t = 0;
        for _ in 0..3 {
            for &(a, b) in &CLOCKWISE {
                t += 1_000;
                q.update(a, b, t);
                total += detents.update(q.count());
            }
        }
        assert_eq!(total, 3);
    }
}
