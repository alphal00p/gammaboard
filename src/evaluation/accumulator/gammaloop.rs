use super::{Accumulator, VectorAccumulatorState};
use crate::core::{EngineError, RunSpec};
use gammalooprs::integrands::evaluation::{EvaluationResult, StabilityStatus};
use gammalooprs::observables::{HistogramSnapshot, ObservableSnapshotBundle};
use gammalooprs::settings::runtime::Precision;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaLoopAccumulatorState {
    pub bundle: ObservableSnapshotBundle,
    pub estimate: VectorAccumulatorState,
    #[serde(default)]
    pub diagnostics: GammaLoopDiagnostics,
}

impl Default for GammaLoopAccumulatorState {
    fn default() -> Self {
        Self {
            bundle: ObservableSnapshotBundle::default(),
            estimate: VectorAccumulatorState::from_config(
                vec!["real".to_string(), "imag".to_string()],
                crate::core::TrainingProjection::AbsComplex {
                    real: "real".to_string(),
                    imag: "imag".to_string(),
                },
                None,
            ),
            diagnostics: GammaLoopDiagnostics::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GammaLoopDiagnostics {
    pub count_total: i64,
    pub count_double_precision: i64,
    pub count_quad_precision: i64,
    pub count_arb_precision: i64,
    pub count_nan: i64,
    pub count_nan_or_unstable: i64,
    pub count_loop_momenta_escalated: i64,
    pub total_eval_time_ms: f64,
    pub total_integrand_eval_time_ms: f64,
    pub total_evaluator_eval_time_ms: f64,
    pub total_parameterization_time_ms: f64,
    pub total_event_processing_time_ms: f64,
    pub total_generated_events: i64,
    pub total_accepted_events: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaLoopAccumulatorDigest {
    pub histogram_count: usize,
    pub sample_count: i64,
    pub primary_histogram_name: Option<String>,
    pub primary_histogram_title: Option<String>,
    pub real_mean: f64,
    pub imag_mean: f64,
    pub real_error: f64,
    pub imag_error: f64,
}

impl GammaLoopAccumulatorState {
    pub fn diagnostics_from_evaluation_results(
        results: &[EvaluationResult],
    ) -> GammaLoopDiagnostics {
        let mut diagnostics = GammaLoopDiagnostics::default();
        for result in results {
            diagnostics.count_total += 1;
            diagnostics.total_eval_time_ms +=
                result.evaluation_metadata.total_timing.as_secs_f64() * 1000.0;
            diagnostics.total_integrand_eval_time_ms += result
                .evaluation_metadata
                .integrand_evaluation_time
                .as_secs_f64()
                * 1000.0;
            diagnostics.total_evaluator_eval_time_ms += result
                .evaluation_metadata
                .evaluator_evaluation_time
                .as_secs_f64()
                * 1000.0;
            diagnostics.total_parameterization_time_ms += result
                .evaluation_metadata
                .parameterization_time
                .as_secs_f64()
                * 1000.0;
            diagnostics.total_event_processing_time_ms += result
                .evaluation_metadata
                .event_processing_time
                .as_secs_f64()
                * 1000.0;
            diagnostics.total_generated_events +=
                result.evaluation_metadata.generated_event_count as i64;
            diagnostics.total_accepted_events +=
                result.evaluation_metadata.accepted_event_count as i64;
            if result.evaluation_metadata.is_nan {
                diagnostics.count_nan += 1;
            }
            if result.evaluation_metadata.loop_momenta_escalation.is_some() {
                diagnostics.count_loop_momenta_escalated += 1;
            }
            let final_stability = result.evaluation_metadata.stability_results.last();
            if result.evaluation_metadata.is_nan
                || final_stability
                    .map(|entry| matches!(entry.status, StabilityStatus::Unstable(_)))
                    .unwrap_or(false)
            {
                diagnostics.count_nan_or_unstable += 1;
            }
            match final_stability
                .map(|entry| entry.precision)
                .unwrap_or(Precision::Double)
            {
                Precision::Double => diagnostics.count_double_precision += 1,
                Precision::Quad => diagnostics.count_quad_precision += 1,
                Precision::Arb => diagnostics.count_arb_precision += 1,
            }
        }
        diagnostics
    }

    pub fn cleared_bundle(bundle: &ObservableSnapshotBundle) -> ObservableSnapshotBundle {
        ObservableSnapshotBundle {
            histograms: bundle
                .histograms
                .iter()
                .map(|(name, histogram)| (name.clone(), cleared_histogram_snapshot(histogram)))
                .collect(),
        }
    }

    pub fn merge_in_place(&mut self, other: Self) -> Result<(), EngineError> {
        if self.bundle.histograms.is_empty() {
            self.bundle = other.bundle;
        } else if !other.bundle.histograms.is_empty() {
            self.bundle.merge_in_place(&other.bundle).map_err(|err| {
                EngineError::engine(format!(
                    "failed to merge gammaloop accumulator bundle: {err}"
                ))
            })?;
        }
        self.estimate.merge(other.estimate);
        self.diagnostics.merge_in_place(other.diagnostics);
        Ok(())
    }

    pub fn histogram_count(&self) -> usize {
        self.bundle.histograms.len()
    }

    pub fn primary_histogram_name(&self) -> Option<&str> {
        // Prefer an explicitly ordered or tagged histogram if present, otherwise
        // fall back to the histogram with the largest sample_count, then to the
        // first map entry.
        if let Some((name, _)) = self
            .bundle
            .histograms
            .iter()
            .find(|(_, h)| h.discrete_ordering.is_some())
        {
            return Some(name.as_str());
        }
        if let Some((name, _)) = self
            .bundle
            .histograms
            .iter()
            .max_by_key(|(_, h)| h.sample_count)
        {
            return Some(name.as_str());
        }
        self.bundle
            .histograms
            .iter()
            .next()
            .map(|(name, _)| name.as_str())
    }

    pub fn primary_histogram(&self) -> Option<&HistogramSnapshot> {
        if let Some((_, hist)) = self
            .bundle
            .histograms
            .iter()
            .find(|(_, h)| h.discrete_ordering.is_some())
        {
            return Some(hist);
        }
        if let Some((_, hist)) = self
            .bundle
            .histograms
            .iter()
            .max_by_key(|(_, h)| h.sample_count)
        {
            return Some(hist);
        }
        self.bundle.histograms.values().next()
    }

    pub fn real_mean(&self) -> f64 {
        self.estimate
            .component("real")
            .map(|component| component.state.mean())
            .unwrap_or_default()
    }

    pub fn imag_mean(&self) -> f64 {
        self.estimate
            .component("imag")
            .map(|component| component.state.mean())
            .unwrap_or_default()
    }

    pub fn abs_mean(&self) -> f64 {
        self.estimate.projection.state.mean()
    }

    pub fn real_stderr(&self) -> f64 {
        self.estimate
            .component("real")
            .map(|component| component.state.stderr())
            .unwrap_or_default()
    }

    pub fn imag_stderr(&self) -> f64 {
        self.estimate
            .component("imag")
            .map(|component| component.state.stderr())
            .unwrap_or_default()
    }

    pub fn abs_stderr(&self) -> f64 {
        self.estimate.projection.state.stderr()
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.estimate.signal_to_noise()
    }

    pub fn rsd(&self) -> f64 {
        self.estimate.rsd()
    }
}

impl GammaLoopDiagnostics {
    pub fn merge_in_place(&mut self, other: Self) {
        self.count_total += other.count_total;
        self.count_double_precision += other.count_double_precision;
        self.count_quad_precision += other.count_quad_precision;
        self.count_arb_precision += other.count_arb_precision;
        self.count_nan += other.count_nan;
        self.count_nan_or_unstable += other.count_nan_or_unstable;
        self.count_loop_momenta_escalated += other.count_loop_momenta_escalated;
        self.total_eval_time_ms += other.total_eval_time_ms;
        self.total_integrand_eval_time_ms += other.total_integrand_eval_time_ms;
        self.total_evaluator_eval_time_ms += other.total_evaluator_eval_time_ms;
        self.total_parameterization_time_ms += other.total_parameterization_time_ms;
        self.total_event_processing_time_ms += other.total_event_processing_time_ms;
        self.total_generated_events += other.total_generated_events;
        self.total_accepted_events += other.total_accepted_events;
    }

    pub fn avg_eval_time_ms(&self) -> f64 {
        safe_ratio(self.total_eval_time_ms, self.count_total)
    }

    pub fn avg_integrand_eval_time_ms(&self) -> f64 {
        safe_ratio(self.total_integrand_eval_time_ms, self.count_total)
    }

    pub fn avg_evaluator_eval_time_ms(&self) -> f64 {
        safe_ratio(self.total_evaluator_eval_time_ms, self.count_total)
    }

    pub fn avg_parameterization_time_ms(&self) -> f64 {
        safe_ratio(self.total_parameterization_time_ms, self.count_total)
    }

    pub fn avg_event_processing_time_ms(&self) -> f64 {
        safe_ratio(self.total_event_processing_time_ms, self.count_total)
    }

    pub fn promoted_to_quad_ratio(&self) -> f64 {
        safe_ratio(self.count_quad_precision as f64, self.count_total)
    }

    pub fn promoted_to_arb_ratio(&self) -> f64 {
        safe_ratio(self.count_arb_precision as f64, self.count_total)
    }

    pub fn nan_or_unstable_ratio(&self) -> f64 {
        safe_ratio(self.count_nan_or_unstable as f64, self.count_total)
    }

    pub fn loop_momenta_escalated_ratio(&self) -> f64 {
        safe_ratio(self.count_loop_momenta_escalated as f64, self.count_total)
    }

    pub fn accepted_event_ratio(&self) -> f64 {
        safe_ratio(
            self.total_accepted_events as f64,
            self.total_generated_events,
        )
    }
}

fn safe_ratio(numerator: f64, denominator: i64) -> f64 {
    if denominator > 0 {
        numerator / denominator as f64
    } else {
        0.0
    }
}

fn cleared_histogram_snapshot(histogram: &HistogramSnapshot) -> HistogramSnapshot {
    let mut cleared = histogram.clone();
    cleared.sample_count = 0;
    for bin in &mut cleared.bins {
        clear_histogram_bin(bin);
    }
    clear_histogram_bin(&mut cleared.underflow_bin);
    clear_histogram_bin(&mut cleared.overflow_bin);
    cleared.statistics.in_range_entry_count = 0;
    cleared.statistics.nan_value_count = 0;
    cleared.statistics.mitigated_pair_count = 0;
    cleared
}

fn clear_histogram_bin(bin: &mut gammalooprs::observables::HistogramBinSnapshot) {
    bin.entry_count = 0;
    bin.sum_weights = 0.0;
    bin.sum_weights_squared = 0.0;
    bin.mitigated_fill_count = 0;
}

impl Accumulator for GammaLoopAccumulatorState {
    type Persistent = Self;
    type Digest = GammaLoopAccumulatorDigest;

    fn sample_count(&self) -> i64 {
        self.estimate.sample_count()
    }

    fn merge(&mut self, other: Self) {
        self.merge_in_place(other)
            .expect("gammaloop accumulator payloads should be merge-compatible");
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }

    fn get_digest(&self, _run_spec: &RunSpec) -> Result<Self::Digest, EngineError> {
        let primary_histogram = self.primary_histogram();
        Ok(GammaLoopAccumulatorDigest {
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

impl From<GammaLoopAccumulatorState> for GammaLoopAccumulatorDigest {
    fn from(state: GammaLoopAccumulatorState) -> Self {
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
    use super::{GammaLoopAccumulatorState, GammaLoopDiagnostics};
    use crate::evaluation::{Accumulator, VectorAccumulatorState};
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
    ) -> GammaLoopAccumulatorState {
        GammaLoopAccumulatorState {
            bundle: ObservableSnapshotBundle {
                histograms: BTreeMap::from([(
                    "pt".to_string(),
                    HistogramSnapshot {
                        kind: gammalooprs::observables::HistogramSnapshotKind::Continuous,
                        title: "pt".to_string(),
                        type_description: "HwU".to_string(),
                        phase: ObservablePhase::Real,
                        value_transform: ObservableValueTransform::Identity,
                        supports_misbinning_mitigation: false,
                        discrete_min_bin_id: None,
                        x_min: Some(0.0),
                        x_max: Some(1.0),
                        discrete_ordering: None,
                        sample_count: histogram_sample_count,
                        log_x_axis: false,
                        log_y_axis: false,
                        bins: vec![HistogramBinSnapshot {
                            x_min: Some(0.0),
                            x_max: Some(1.0),
                            bin_id: None,
                            label: None,
                            entry_count: histogram_sample_count,
                            sum_weights: histogram_sum_weights,
                            sum_weights_squared: histogram_sum_weights_squared,
                            mitigated_fill_count: 0,
                        }],
                        underflow_bin: HistogramBinSnapshot {
                            x_min: None,
                            x_max: None,
                            bin_id: None,
                            label: None,
                            entry_count: 0,
                            sum_weights: 0.0,
                            sum_weights_squared: 0.0,
                            mitigated_fill_count: 0,
                        },
                        overflow_bin: HistogramBinSnapshot {
                            x_min: None,
                            x_max: None,
                            bin_id: None,
                            label: None,
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
            estimate: test_estimate(estimate_real_sum, estimate_real_sq_sum, estimate_count),
            diagnostics: GammaLoopDiagnostics::default(),
        }
    }

    fn test_estimate(sum: f64, sum_sq: f64, count: i64) -> VectorAccumulatorState {
        let mut estimate = VectorAccumulatorState::from_config(
            vec!["real".to_string(), "imag".to_string()],
            crate::core::TrainingProjection::AbsComplex {
                real: "real".to_string(),
                imag: "imag".to_string(),
            },
            None,
        );
        estimate.components[0].state.count = count;
        estimate.components[0].state.sum_weighted_value = sum;
        estimate.components[0].state.sum_abs = sum.abs();
        estimate.components[0].state.sum_sq = sum_sq;
        estimate.components[1].state.count = count;
        estimate.projection.state.count = count;
        estimate.projection.state.sum_weighted_value = sum.abs();
        estimate.projection.state.sum_abs = sum.abs();
        estimate.projection.state.sum_sq = sum_sq;
        estimate
    }

    #[test]
    fn empty_state_accepts_first_non_empty_merge() {
        let mut left = GammaLoopAccumulatorState::default();
        let right = state(4.0, 20.0, 2, 6.0, 20.0, 2);

        left.merge(right.clone());

        assert_eq!(left.histogram_count(), 1);
        assert_eq!(left.sample_count(), 2);
        assert_eq!(left.real_mean(), 3.0);
        assert_eq!(left.primary_histogram_name(), Some("pt"));
    }

    #[test]
    fn cleared_bundle_preserves_layout_and_zeros_counts() {
        let original = state(4.0, 20.0, 2, 6.0, 20.0, 2).bundle;

        let cleared = GammaLoopAccumulatorState::cleared_bundle(&original);

        let histogram = cleared.histograms.get("pt").unwrap();
        assert_eq!(histogram.title, "pt");
        assert_eq!(histogram.type_description, "HwU");
        assert_eq!(histogram.sample_count, 0);
        assert_eq!(histogram.bins[0].entry_count, 0);
        assert_eq!(histogram.bins[0].sum_weights, 0.0);
        assert_eq!(histogram.bins[0].sum_weights_squared, 0.0);
        assert_eq!(histogram.statistics.in_range_entry_count, 0);
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
