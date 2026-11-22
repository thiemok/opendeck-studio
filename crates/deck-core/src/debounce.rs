//! Key debouncing: an edge counts once the level has been stable for the interval (KTD18).

/// Default stable time before an edge counts.
pub const DEFAULT_INTERVAL_MS: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Pressed,
    Released,
}

/// Fed with the key's logical state (`true` = pressed) and the time.
pub struct Debouncer {
    interval_ms: u64,
    stable: bool,
    candidate: bool,
    since_ms: u64,
}

impl Debouncer {
    /// Starts released.
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            stable: false,
            candidate: false,
            since_ms: 0,
        }
    }

    /// True when no level change is waiting out the interval, so no further samples are
    /// needed until the input changes again.
    pub fn settled(&self) -> bool {
        self.candidate == self.stable
    }

    /// The debounced level: true from a counted press until a counted release.
    pub fn pressed(&self) -> bool {
        self.stable
    }

    /// Returns an edge when the new level has held for the interval.
    pub fn update(&mut self, pressed: bool, now_ms: u64) -> Option<Edge> {
        if pressed != self.candidate {
            self.candidate = pressed;
            self.since_ms = now_ms;
        }
        if self.candidate == self.stable || now_ms.saturating_sub(self.since_ms) < self.interval_ms
        {
            return None;
        }
        self.stable = self.candidate;
        Some(if self.stable {
            Edge::Pressed
        } else {
            Edge::Released
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples `(level, at_ms)` and collects the edges.
    fn run(samples: &[(bool, u64)]) -> Vec<(Edge, u64)> {
        let mut d = Debouncer::new(DEFAULT_INTERVAL_MS);
        samples
            .iter()
            .filter_map(|&(level, t)| d.update(level, t).map(|e| (e, t)))
            .collect()
    }

    fn every_ms(from: u64, to: u64, level: bool) -> impl Iterator<Item = (bool, u64)> {
        (from..=to).map(move |t| (level, t))
    }

    #[test]
    fn a_bouncy_press_yields_one_edge_after_20ms_stable() {
        let mut samples = vec![(true, 0), (false, 2), (true, 5)];
        samples.extend(every_ms(6, 40, true));
        assert_eq!(run(&samples), vec![(Edge::Pressed, 25)]);
    }

    #[test]
    fn release_yields_one_edge_after_20ms_stable() {
        let mut samples: Vec<_> = every_ms(0, 30, true).collect();
        samples.extend([(false, 31), (true, 33)]);
        samples.extend(every_ms(34, 70, false));
        assert_eq!(
            run(&samples),
            vec![(Edge::Pressed, 20), (Edge::Released, 54)]
        );
    }

    #[test]
    fn settled_only_while_no_change_is_pending() {
        let mut d = Debouncer::new(DEFAULT_INTERVAL_MS);
        assert!(d.settled());
        d.update(true, 0);
        assert!(!d.settled(), "a press is waiting out the interval");
        d.update(true, 20);
        assert!(d.settled(), "the press counted");
        d.update(false, 30);
        d.update(true, 32);
        assert!(
            d.settled(),
            "a bounce back to the stable level leaves nothing pending"
        );
    }

    #[test]
    fn pressed_reports_the_debounced_level_not_the_raw_one() {
        let mut d = Debouncer::new(DEFAULT_INTERVAL_MS);
        d.update(true, 0);
        assert!(!d.pressed(), "not yet counted");
        d.update(true, 20);
        assert!(d.pressed());
        d.update(false, 25);
        assert!(
            d.pressed(),
            "a release waiting out the interval is still pressed"
        );
    }

    #[test]
    fn a_glitch_shorter_than_the_interval_is_ignored() {
        let mut samples: Vec<_> = every_ms(0, 10, false).collect();
        samples.extend(every_ms(11, 25, true));
        samples.extend(every_ms(26, 60, false));
        assert!(run(&samples).is_empty());
    }
}
