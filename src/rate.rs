use std::time::Duration;

/// Half-life of the rate estimate. Long enough that the ETA is stable across a
/// directory of mixed file sizes, short enough to react within a few seconds
/// when throughput genuinely changes.
pub const HALF_LIFE_SECS: f64 = 3.0;

/// Exponentially weighted moving average of transfer rate, in bytes per second.
pub struct RateEstimator {
    rate: Option<f64>,
    half_life: f64,
}

impl RateEstimator {
    pub fn new(half_life_secs: f64) -> Self {
        RateEstimator {
            rate: None,
            half_life: half_life_secs,
        }
    }

    /// Fold in `bytes` transferred over `dt`. Zero-length intervals are ignored
    /// rather than producing an infinite instantaneous rate.
    pub fn sample(&mut self, bytes: u64, dt: Duration) {
        let secs = dt.as_secs_f64();
        if secs <= 0.0 {
            return;
        }
        let instant = bytes as f64 / secs;
        self.rate = Some(match self.rate {
            None => instant,
            Some(current) => {
                let alpha = 1.0 - 0.5_f64.powf(secs / self.half_life);
                current + alpha * (instant - current)
            }
        });
    }

    pub fn rate(&self) -> Option<f64> {
        self.rate
    }

    /// Time to move `remaining` bytes at the current rate.
    pub fn eta(&self, remaining: u64) -> Option<Duration> {
        let rate = self.rate?;
        if rate <= 0.0 || !rate.is_finite() {
            return None;
        }
        let secs = remaining as f64 / rate;
        if !secs.is_finite() || secs < 0.0 {
            return None;
        }
        Some(Duration::from_secs_f64(secs.min(u32::MAX as f64)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn rate_is_unknown_before_any_sample() {
        let est = RateEstimator::new(3.0);
        assert_eq!(est.rate(), None);
        assert_eq!(est.eta(1000), None);
    }

    #[test]
    fn first_sample_sets_the_rate_exactly() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1));
        let r = est.rate().expect("rate after one sample");
        assert!((r - (100 * MB) as f64).abs() < 1.0, "got {r}");
    }

    #[test]
    fn steady_input_converges_to_the_steady_rate() {
        let mut est = RateEstimator::new(3.0);
        for _ in 0..100 {
            est.sample(10 * MB, Duration::from_millis(100));
        }
        let r = est.rate().expect("rate");
        let expected = (100 * MB) as f64; // 10 MiB per 100 ms
        assert!(
            (r - expected).abs() / expected < 0.01,
            "expected ~{expected}, got {r}"
        );
    }

    #[test]
    fn one_half_life_moves_the_estimate_halfway() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1)); // rate = 100 MiB/s
        est.sample(0, Duration::from_secs(3)); // instantaneous 0, one half-life
        let r = est.rate().expect("rate");
        let expected = (100 * MB) as f64 / 2.0;
        assert!(
            (r - expected).abs() / expected < 0.02,
            "expected ~{expected}, got {r}"
        );
    }

    #[test]
    fn zero_duration_samples_are_ignored() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1));
        let before = est.rate();
        est.sample(5 * MB, Duration::ZERO);
        assert_eq!(est.rate(), before);
        assert!(est.rate().expect("finite").is_finite());
    }

    #[test]
    fn eta_divides_remaining_by_rate() {
        let mut est = RateEstimator::new(3.0);
        est.sample(10 * MB, Duration::from_secs(1));
        let eta = est.eta(100 * MB).expect("eta");
        assert_eq!(eta.as_secs(), 10);
    }

    #[test]
    fn eta_handles_multi_terabyte_remainders() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1));
        let ten_tb = 10 * 1024_u64.pow(4);
        let eta = est.eta(ten_tb).expect("eta");
        // 10 TiB / 100 MiB/s = 104,857.6 s
        assert_eq!(eta.as_secs(), 104_857);
    }

    #[test]
    fn eta_is_unknown_when_the_rate_is_zero() {
        let mut est = RateEstimator::new(3.0);
        est.sample(0, Duration::from_secs(1));
        assert_eq!(est.eta(1000), None);
    }
}
