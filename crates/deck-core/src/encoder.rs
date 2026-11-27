//! Turns the encoder's hardware count into whole detents (KTD23).
//!
//! The count resets to zero when it reaches its high or low limit instead of wrapping (the
//! ESP32's pulse counter does, and [`crate::quadrature::Quadrature`] copies it), so readings
//! are compared modulo that limit. Partial steps are kept until they add up to a detent, so a
//! knob nudged and released emits nothing.

/// Feed it successive counter readings; it returns the detents completed in between.
pub struct DetentCounter {
    counts_per_detent: i32,
    limit: i32,
    last: i32,
    partial: i32,
}

impl DetentCounter {
    /// `limit` is the counter's configured high limit (the low limit is its negative), and
    /// `start` is the reading at the moment counting begins.
    pub fn new(counts_per_detent: i16, limit: i16, start: i16) -> Self {
        Self {
            counts_per_detent: i32::from(counts_per_detent),
            limit: i32::from(limit),
            last: i32::from(start),
            partial: 0,
        }
    }

    /// Detents since the previous reading: positive clockwise, negative anticlockwise.
    pub fn update(&mut self, reading: i16) -> i32 {
        let reading = i32::from(reading);
        let mut delta = reading - self.last;
        self.last = reading;
        // Readings come far more often than half the limit can be turned, so a bigger jump
        // means the counter reached a limit and reset to zero.
        if delta > self.limit / 2 {
            delta -= self.limit;
        } else if delta < -self.limit / 2 {
            delta += self.limit;
        }
        self.partial += delta;
        let detents = self.partial / self.counts_per_detent;
        self.partial -= detents * self.counts_per_detent;
        detents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(counter: &mut DetentCounter, readings: &[i16]) -> Vec<i32> {
        readings.iter().map(|&r| counter.update(r)).collect()
    }

    #[test]
    fn four_counts_make_one_detent_either_way() {
        let mut c = DetentCounter::new(4, 4_000, 0);
        assert_eq!(feed(&mut c, &[4]), vec![1]);
        let mut c = DetentCounter::new(4, 4_000, 0);
        assert_eq!(feed(&mut c, &[-8]), vec![-2]);
    }

    #[test]
    fn partial_steps_wait_until_they_complete_a_detent() {
        let mut c = DetentCounter::new(4, 4_000, 0);
        assert_eq!(feed(&mut c, &[3, 4]), vec![0, 1]);
    }

    #[test]
    fn a_nudge_that_returns_emits_nothing() {
        let mut c = DetentCounter::new(4, 4_000, 0);
        assert_eq!(feed(&mut c, &[2, 0]), vec![0, 0]);
        // Bouncing back from a detent does not undo it.
        let mut c = DetentCounter::new(4, 4_000, 0);
        assert_eq!(feed(&mut c, &[4, 3, 4]), vec![1, 0, 0]);
    }

    #[test]
    fn a_reset_at_the_limit_counts_as_continued_motion() {
        // Clockwise: 3,999 → 4,000, where the counter resets to 0, is +1 count.
        let mut c = DetentCounter::new(4, 4_000, 3_996);
        assert_eq!(feed(&mut c, &[3_999, 0]), vec![0, 1]);
        // Anticlockwise: −3,999 → −4,000 → 0 is −1 count.
        let mut c = DetentCounter::new(4, 4_000, -3_996);
        assert_eq!(feed(&mut c, &[-3_999, 0]), vec![0, -1]);
    }

    #[test]
    fn counts_per_detent_is_configurable() {
        let mut c = DetentCounter::new(2, 4_000, 0);
        assert_eq!(feed(&mut c, &[4]), vec![2]);
    }

    #[test]
    fn a_full_turn_of_twenty_detents_counts_twenty() {
        let mut c = DetentCounter::new(4, 4_000, 0);
        let total: i32 = (1..=80).map(|r| c.update(r)).sum();
        assert_eq!(total, 20);
    }
}
