use super::{ComplexObservableState, Observable};
use crate::core::{EngineError, RunSpec};
use gammalooprs::observables::{HistogramSnapshot, ObservableSnapshotBundle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GammaLoopObservableState {
    pub bundle: ObservableSnapshotBundle,
    pub estimate: ComplexObservableState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaLoopObservableDigest {
    pub histogram_count: usize,
    pub sample_count: i64,
    pub primary_histogram_name: Option<String>,
    pub primary_histogram_title: Option<String>,
    pub real_mean: f64,
    pub imag_mean: f64,
    pub real_error: f64,
    pub imag_error: f64,
}

impl GammaLoopObservableState {
    pub fn merge_in_place(&mut self, other: Self) -> Result<(), EngineError> {
        if self.bundle.histograms.is_empty() {
            self.bundle = other.bundle;
        } else if !other.bundle.histograms.is_empty() {
            self.bundle.merge_in_place(&other.bundle).map_err(|err| {
                EngineError::engine(format!(
                    "failed to merge gammaloop observable bundle: {err}"
                ))
            })?;
        }
        self.estimate.merge(other.estimate);
        Ok(())
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

    pub fn real_mean(&self) -> f64 {
        self.estimate.real_mean()
    }

    pub fn imag_mean(&self) -> f64 {
        self.estimate.imag_mean()
    }

    pub fn abs_mean(&self) -> f64 {
        self.estimate.abs_mean()
    }

    pub fn real_stderr(&self) -> f64 {
        self.estimate.real_stderr()
    }

    pub fn imag_stderr(&self) -> f64 {
        self.estimate.imag_stderr()
    }

    pub fn abs_stderr(&self) -> f64 {
        self.estimate.abs_stderr()
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.estimate.signal_to_noise()
    }

    pub fn rsd(&self) -> f64 {
        self.estimate.rsd()
    }
}

impl Observable for GammaLoopObservableState {
    type Persistent = Self;
    type Digest = GammaLoopObservableDigest;

    fn sample_count(&self) -> i64 {
        self.estimate.sample_count()
    }

    fn merge(&mut self, other: Self) {
        self.merge_in_place(other)
            .expect("gammaloop observable payloads should be merge-compatible");
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }

    fn get_digest(&self, _run_spec: &RunSpec) -> Result<Self::Digest, EngineError> {
        let primary_histogram = self.primary_histogram();
        Ok(GammaLoopObservableDigest {
            histogram_count: self.histogram_count(),
            sample_count: self.sample_count(),
            primary_histogram_name: self.primary_histogram_name().map(str::to_string),
            primary_histogram_title: primary_histogram.map(|histogram| histogram.title.clone()),
            real_mean: self.real_mean(),
            imag_mean: self.imag_mean(),
            real_error: self.real_stderr(),
            imag_error: self.imag_stderr(),
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
            real_mean: state.real_mean(),
            imag_mean: state.imag_mean(),
            real_error: state.real_stderr(),
            imag_error: state.imag_stderr(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GammaLoopObservableState;
    use crate::evaluation::{ComplexObservableState, Observable};
    use gammalooprs::observables::{
        HistogramBinSnapshot, HistogramSnapshot, HistogramStatisticsSnapshot, ObservablePhase,
        ObservableSnapshotBundle, ObservableValueTransform,
    };
    use std::collections::BTreeMap;

    fn state(
        histogram_sum_weights: f64,
        histogram_sum_weights_squared: f64,
        histogram_sample_count: usize,
        estimate_real_sum: f64,
        estimate_real_sq_sum: f64,
        estimate_count: i64,
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
                        sample_count: histogram_sample_count,
                        log_x_axis: false,
                        log_y_axis: false,
                        bins: vec![HistogramBinSnapshot {
                            x_min: Some(0.0),
                            x_max: Some(1.0),
                            entry_count: histogram_sample_count,
                            sum_weights: histogram_sum_weights,
                            sum_weights_squared: histogram_sum_weights_squared,
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
                            in_range_entry_count: histogram_sample_count,
                            nan_value_count: 0,
                            mitigated_pair_count: 0,
                        },
                    },
                )]),
            },
            estimate: ComplexObservableState {
                count: estimate_count,
                real_sum: estimate_real_sum,
                imag_sum: 0.0,
                abs_sum: estimate_real_sum.abs(),
                abs_sq_sum: estimate_real_sq_sum,
                real_sq_sum: estimate_real_sq_sum,
                imag_sq_sum: 0.0,
                weight_sum: estimate_count as f64,
                nan_count: 0,
            },
        }
    }

    #[test]
    fn empty_state_accepts_first_non_empty_merge() {
        let mut left = GammaLoopObservableState::default();
        let right = state(4.0, 20.0, 2, 6.0, 20.0, 2);

        left.merge(right.clone());

        assert_eq!(left.histogram_count(), 1);
        assert_eq!(left.sample_count(), 2);
        assert_eq!(left.real_mean(), 3.0);
        assert_eq!(left.primary_histogram_name(), Some("pt"));
    }

    #[test]
    fn merge_combines_bundle_and_estimate() {
        let mut left = state(4.0, 20.0, 2, 6.0, 20.0, 2);
        let right = state(2.0, 10.0, 1, 1.0, 1.0, 1);

        left.merge(right);

        assert_eq!(left.sample_count(), 3);
        assert_eq!(left.real_mean(), 7.0 / 3.0);
        assert_eq!(
            left.primary_histogram()
                .expect("primary histogram")
                .sample_count,
            3
        );
    }
}
