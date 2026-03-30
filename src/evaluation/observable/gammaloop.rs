use super::Observable;
use crate::core::{EngineError, RunSpec};
use gammalooprs::observables::{HistogramSnapshot, ObservableSnapshotBundle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GammaLoopObservableState {
    pub bundle: ObservableSnapshotBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaLoopObservableDigest {
    pub histogram_count: usize,
    pub sample_count: i64,
    pub primary_histogram_name: Option<String>,
    pub primary_histogram_title: Option<String>,
    pub primary_histogram_mean: Option<f64>,
    pub primary_histogram_error: Option<f64>,
}

impl GammaLoopObservableState {
    pub fn merge_in_place(&mut self, other: Self) -> Result<(), EngineError> {
        if self.bundle.histograms.is_empty() {
            self.bundle = other.bundle;
            return Ok(());
        }
        if other.bundle.histograms.is_empty() {
            return Ok(());
        }
        self.bundle.merge_in_place(&other.bundle).map_err(|err| {
            EngineError::engine(format!(
                "failed to merge gammaloop observable bundle: {err}"
            ))
        })
    }

    pub fn histogram_count(&self) -> usize {
        self.bundle.histograms.len()
    }

    pub fn primary_histogram_name(&self) -> Option<&str> {
        self.bundle
            .histograms
            .iter()
            .next()
            .map(|(name, _)| name.as_str())
    }

    pub fn primary_histogram(&self) -> Option<&HistogramSnapshot> {
        self.bundle.histograms.values().next()
    }

    pub fn primary_mean(&self) -> f64 {
        self.primary_histogram()
            .map(histogram_total_mean)
            .unwrap_or(0.0)
    }

    pub fn primary_stderr(&self) -> f64 {
        self.primary_histogram()
            .map(histogram_total_stderr)
            .unwrap_or(0.0)
    }

    pub fn signal_to_noise(&self) -> f64 {
        let stderr = self.primary_stderr();
        if stderr <= 0.0 {
            0.0
        } else {
            let mean = self.primary_mean();
            (mean * mean) / (stderr * stderr)
        }
    }
}

impl Observable for GammaLoopObservableState {
    type Persistent = ObservableSnapshotBundle;
    type Digest = GammaLoopObservableDigest;

    fn sample_count(&self) -> i64 {
        self.bundle
            .histograms
            .values()
            .map(|histogram| histogram.sample_count as i64)
            .max()
            .unwrap_or(0)
    }

    fn merge(&mut self, other: Self) {
        self.merge_in_place(other)
            .expect("gammaloop observable bundles should be merge-compatible");
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.bundle.clone()
    }

    fn get_digest(&self, _run_spec: &RunSpec) -> Result<Self::Digest, EngineError> {
        let primary_histogram = self.primary_histogram();
        Ok(GammaLoopObservableDigest {
            histogram_count: self.histogram_count(),
            sample_count: self.sample_count(),
            primary_histogram_name: self.primary_histogram_name().map(str::to_string),
            primary_histogram_title: primary_histogram.map(|histogram| histogram.title.clone()),
            primary_histogram_mean: primary_histogram.map(histogram_total_mean),
            primary_histogram_error: primary_histogram.map(histogram_total_stderr),
        })
    }
}

impl From<GammaLoopObservableState> for GammaLoopObservableDigest {
    fn from(state: GammaLoopObservableState) -> Self {
        let primary_histogram = state.primary_histogram().cloned();
        Self {
            histogram_count: state.histogram_count(),
            sample_count: state.sample_count(),
            primary_histogram_name: state.primary_histogram_name().map(str::to_string),
            primary_histogram_title: primary_histogram
                .as_ref()
                .map(|histogram| histogram.title.clone()),
            primary_histogram_mean: primary_histogram.as_ref().map(histogram_total_mean),
            primary_histogram_error: primary_histogram.as_ref().map(histogram_total_stderr),
        }
    }
}

fn histogram_total_mean(histogram: &HistogramSnapshot) -> f64 {
    let sample_count = histogram.sample_count;
    if sample_count == 0 {
        return 0.0;
    }
    histogram_total_sum_weights(histogram) / sample_count as f64
}

fn histogram_total_stderr(histogram: &HistogramSnapshot) -> f64 {
    let sample_count = histogram.sample_count;
    if sample_count <= 1 {
        return 0.0;
    }

    let n = sample_count as f64;
    let sum = histogram_total_sum_weights(histogram);
    let sum_sq = histogram_total_sum_weights_squared(histogram);
    let variance_numerator = sum_sq - (sum * sum) / n;
    if !variance_numerator.is_finite() || variance_numerator <= 0.0 {
        0.0
    } else {
        (variance_numerator / (n * (n - 1.0))).sqrt()
    }
}

fn histogram_total_sum_weights(histogram: &HistogramSnapshot) -> f64 {
    histogram
        .bins
        .iter()
        .map(|bin| bin.sum_weights)
        .sum::<f64>()
        + histogram.underflow_bin.sum_weights
        + histogram.overflow_bin.sum_weights
}

fn histogram_total_sum_weights_squared(histogram: &HistogramSnapshot) -> f64 {
    histogram
        .bins
        .iter()
        .map(|bin| bin.sum_weights_squared)
        .sum::<f64>()
        + histogram.underflow_bin.sum_weights_squared
        + histogram.overflow_bin.sum_weights_squared
}

#[cfg(test)]
mod tests {
    use super::GammaLoopObservableState;
    use crate::evaluation::Observable;
    use gammalooprs::observables::{
        HistogramBinSnapshot, HistogramSnapshot, HistogramStatisticsSnapshot, ObservablePhase,
        ObservableSnapshotBundle, ObservableValueTransform,
    };
    use std::collections::BTreeMap;

    fn state(
        sum_weights: f64,
        sum_weights_squared: f64,
        sample_count: usize,
    ) -> GammaLoopObservableState {
        GammaLoopObservableState {
            bundle: ObservableSnapshotBundle {
                histograms: BTreeMap::from([(
                    "pt".to_string(),
                    HistogramSnapshot {
                        title: "pt".to_string(),
                        type_description: "HwU".to_string(),
                        phase: ObservablePhase::Real,
                        value_transform: ObservableValueTransform::Identity,
                        supports_misbinning_mitigation: false,
                        x_min: 0.0,
                        x_max: 1.0,
                        sample_count,
                        log_x_axis: false,
                        log_y_axis: false,
                        bins: vec![HistogramBinSnapshot {
                            x_min: Some(0.0),
                            x_max: Some(1.0),
                            entry_count: sample_count,
                            sum_weights,
                            sum_weights_squared,
                            mitigated_fill_count: 0,
                        }],
                        underflow_bin: HistogramBinSnapshot {
                            x_min: None,
                            x_max: None,
                            entry_count: 0,
                            sum_weights: 0.0,
                            sum_weights_squared: 0.0,
                            mitigated_fill_count: 0,
                        },
                        overflow_bin: HistogramBinSnapshot {
                            x_min: None,
                            x_max: None,
                            entry_count: 0,
                            sum_weights: 0.0,
                            sum_weights_squared: 0.0,
                            mitigated_fill_count: 0,
                        },
                        statistics: HistogramStatisticsSnapshot {
                            in_range_entry_count: sample_count,
                            nan_value_count: 0,
                            mitigated_pair_count: 0,
                        },
                    },
                )]),
            },
        }
    }

    #[test]
    fn empty_state_accepts_first_non_empty_merge() {
        let mut left = GammaLoopObservableState::default();
        let right = state(4.0, 20.0, 2);

        left.merge_in_place(right).expect("merge should succeed");

        assert_eq!(left.sample_count(), 2);
        assert_eq!(left.histogram_count(), 1);
    }

    #[test]
    fn primary_summary_uses_histogram_totals() {
        let state = state(6.0, 20.0, 3);

        assert_eq!(state.sample_count(), 3);
        assert_eq!(state.primary_mean(), 2.0);
        assert!(state.primary_stderr() >= 0.0);
    }
}
