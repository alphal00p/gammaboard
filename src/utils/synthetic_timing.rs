//! Reproducible synthetic service time, independent of worker assignment.
use crate::core::EngineError;
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimingModel {
    #[serde(deserialize_with = "nonnegative")]
    pub per_sample_seconds: f64,
    #[serde(deserialize_with = "nonnegative")]
    pub overhead_seconds: f64,
    #[serde(deserialize_with = "nonnegative")]
    pub sigma_per_sample_seconds: f64,
    #[serde(deserialize_with = "nonnegative")]
    pub sigma_overhead_seconds: f64,
    pub seed: u64,
}

fn nonnegative<'de, D: serde::Deserializer<'de>>(de: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(de)?;
    if value.is_finite() && value >= 0.0 && Duration::try_from_secs_f64(value).is_ok() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "timing seconds must be finite, nonnegative and representable",
        ))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimingStats {
    pub calls: u64,
    pub requested_seconds: f64,
    pub actual_seconds: f64,
    pub clipped_calls: u64,
}

impl TimingModel {
    pub fn validate(&self) -> Result<(), EngineError> {
        for v in [
            self.per_sample_seconds,
            self.overhead_seconds,
            self.sigma_per_sample_seconds,
            self.sigma_overhead_seconds,
        ] {
            if !v.is_finite() || v < 0.0 || Duration::try_from_secs_f64(v).is_err() {
                return Err(EngineError::invalid_input(
                    "timing seconds must be finite, nonnegative and representable",
                ));
            }
        }
        Ok(())
    }

    /// A single correlated per-sample error and independent overhead error per operation.
    pub fn duration(&self, samples: usize, work_key: u64) -> Result<(Duration, bool), EngineError> {
        self.validate()?;
        let mut rng = Xoshiro256StarStar::seed_from_u64(self.seed ^ work_key);
        let radius = (-2.0 * (1.0 - rng.random::<f64>()).ln()).sqrt();
        let angle = std::f64::consts::TAU * rng.random::<f64>();
        let seconds = samples as f64
            * (self.per_sample_seconds + self.sigma_per_sample_seconds * radius * angle.cos())
            + self.overhead_seconds
            + self.sigma_overhead_seconds * radius * angle.sin();
        if !seconds.is_finite() {
            return Err(EngineError::invalid_input(
                "synthetic batch duration overflow",
            ));
        }
        let duration = Duration::try_from_secs_f64(seconds.max(0.0))
            .map_err(|_| EngineError::invalid_input("synthetic batch duration overflow"))?;
        Ok((duration, seconds < 0.0))
    }

    pub fn wait(
        &self,
        samples: usize,
        work_key: u64,
        stats: &mut TimingStats,
    ) -> Result<(), EngineError> {
        self.wait_with(samples, work_key, stats, |duration| {
            if duration.is_zero() {
                return Duration::ZERO;
            }
            let start = Instant::now();
            std::thread::sleep(duration);
            start.elapsed()
        })
    }

    fn wait_with(
        &self,
        samples: usize,
        key: u64,
        stats: &mut TimingStats,
        wait: impl FnOnce(Duration) -> Duration,
    ) -> Result<(), EngineError> {
        let (requested, clipped) = self.duration(samples, key)?;
        let actual = wait(requested);
        stats.calls += 1;
        stats.requested_seconds += requested.as_secs_f64();
        stats.actual_seconds += actual.as_secs_f64();
        stats.clipped_calls += u64::from(clipped);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_duration_and_injected_clock_accounting() {
        let model = TimingModel {
            per_sample_seconds: 0.0000005,
            overhead_seconds: 0.01,
            ..Default::default()
        };
        let mut stats = TimingStats::default();
        model
            .wait_with(2000, 7, &mut stats, |d| {
                assert_eq!(d, Duration::from_millis(11));
                d + Duration::from_micros(50)
            })
            .unwrap();
        assert_eq!(stats.requested_seconds, 0.011);
        assert_eq!(stats.actual_seconds, 0.01105);
        assert_eq!(stats.calls, 1);
    }
    #[test]
    fn gaussian_noise_has_expected_mean_variance_and_replays() {
        let model = TimingModel {
            per_sample_seconds: 0.01,
            overhead_seconds: 0.2,
            sigma_per_sample_seconds: 0.001,
            sigma_overhead_seconds: 0.02,
            seed: 12,
        };
        let values: Vec<_> = (0..20000)
            .map(|key| model.duration(100, key).unwrap().0.as_secs_f64())
            .collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        assert!((mean - 1.2).abs() < 0.005);
        assert!((variance - 0.0104).abs() < 0.0006);
        assert_eq!(
            model.duration(100, 42).unwrap(),
            model.clone().duration(100, 42).unwrap()
        );
    }
    #[test]
    fn invalid_values_overflow_and_negative_draws_are_handled() {
        for text in [
            "per_sample_seconds = -1.0",
            "overhead_seconds = nan",
            "sigma_overhead_seconds = inf",
        ] {
            assert!(toml::from_str::<TimingModel>(text).is_err());
        }
        let model = TimingModel {
            per_sample_seconds: 1e18,
            ..Default::default()
        };
        assert!(model.duration(usize::MAX, 0).is_err());
        let noise = TimingModel {
            sigma_overhead_seconds: 1.0,
            ..Default::default()
        };
        assert!((0..100).any(|key| noise.duration(0, key).unwrap() == (Duration::ZERO, true)));
    }
}
