use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A rate over the last minute of active runner wall time, including time between
/// observations. Values are spread uniformly over each observation interval;
/// this also handles batches that take longer than the window itself.
pub(crate) struct WallTimeRate {
    last_observed_at: Instant,
    intervals: VecDeque<(Duration, f64)>,
    elapsed: Duration,
}

impl WallTimeRate {
    const WINDOW: Duration = Duration::from_secs(60);

    pub(crate) fn new(now: Instant) -> Self {
        Self {
            last_observed_at: now,
            intervals: VecDeque::new(),
            elapsed: Duration::ZERO,
        }
    }

    pub(crate) fn observe(&mut self, now: Instant, value: f64) -> f64 {
        let elapsed = now.duration_since(self.last_observed_at);
        self.last_observed_at = now;
        let value = if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        };
        self.intervals.push_back((elapsed, value));
        self.elapsed += elapsed;

        while self.elapsed > Self::WINDOW {
            let excess = self.elapsed - Self::WINDOW;
            let (seconds, value) = self.intervals.pop_front().expect("nonempty rate window");
            if seconds <= excess {
                self.elapsed -= seconds;
            } else {
                let retained_seconds = seconds - excess;
                let retained_value = value * retained_seconds.as_secs_f64() / seconds.as_secs_f64();
                self.intervals
                    .push_front((retained_seconds, retained_value));
                self.elapsed = Self::WINDOW;
            }
        }
        self.rate()
    }

    pub(crate) fn rate(&self) -> f64 {
        if !self.elapsed.is_zero() {
            // Sum the retained intervals so expiring a large batch cannot leave
            // a floating-point residue masquerading as a nonzero stalled rate.
            self.intervals.iter().map(|(_, value)| value).sum::<f64>() / self.elapsed.as_secs_f64()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_batches_include_waits_and_zero_completion_polls() {
        let start = Instant::now();
        let mut rate = WallTimeRate::new(start);
        for tick in 1..=1200 {
            // 5,000 samples every ten seconds, polled every 50 ms.
            let samples = if tick % 200 == 0 { 5000.0 } else { 0.0 };
            rate.observe(start + Duration::from_millis(tick * 50), samples);
        }
        assert!((rate.rate() - 500.0).abs() < 1e-8);
        assert_eq!(rate.observe(start + Duration::from_secs(120), 0.0), 0.0);
    }

    #[test]
    fn utilization_includes_sleep_between_compute_intervals() {
        let start = Instant::now();
        let mut busy = WallTimeRate::new(start);
        assert_eq!(busy.observe(start + Duration::from_secs(10), 10.0), 1.0);
        // A ten-second sampler update, then ten seconds evaluating.
        assert!((busy.observe(start + Duration::from_secs(30), 10.0) - 2.0 / 3.0).abs() < 1e-8);
        assert!((busy.observe(start + Duration::from_secs(60), 0.0) - 1.0 / 3.0).abs() < 1e-8);
    }

    #[test]
    fn long_batches_and_partial_window_expiry_keep_duration_weighting() {
        let start = Instant::now();
        let mut rate = WallTimeRate::new(start);
        assert_eq!(
            rate.observe(start + Duration::from_secs(120), 60000.0),
            500.0
        );
        assert_eq!(rate.observe(start + Duration::from_secs(150), 0.0), 250.0);
        assert_eq!(rate.observe(start + Duration::from_secs(180), 0.0), 0.0);
    }
}
