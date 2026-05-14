use serde::{Deserialize, Serialize};

use super::{Accumulator, VectorAccumulatorState};
use crate::core::{EngineError, RunSpec, TrainingProjection};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaLoopAccumulatorState {
    pub estimate: VectorAccumulatorState,
    #[serde(default)]
    pub diagnostics: GammaLoopDiagnostics,
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
    pub fn merge_in_place(&mut self, other: Self) -> Result<(), EngineError> {
        Accumulator::merge(&mut self.estimate, other.estimate);
        Ok(())
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.estimate.signal_to_noise()
    }

    pub fn rsd(&self) -> f64 {
        self.estimate.rsd()
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
}

impl GammaLoopDiagnostics {
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

impl Accumulator for GammaLoopAccumulatorState {
    type Persistent = Self;
    type Digest = GammaLoopAccumulatorDigest;

    fn sample_count(&self) -> i64 {
        self.estimate.sample_count()
    }

    fn merge(&mut self, other: Self) {
        let _ = self.merge_in_place(other);
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }

    fn get_digest(&self, _run_spec: &RunSpec) -> Result<Self::Digest, EngineError> {
        Ok(self.clone().into())
    }
}

impl From<GammaLoopAccumulatorState> for GammaLoopAccumulatorDigest {
    fn from(state: GammaLoopAccumulatorState) -> Self {
        Self {
            histogram_count: 0,
            sample_count: state.estimate.sample_count(),
            primary_histogram_name: None,
            primary_histogram_title: None,
            real_mean: state.real_mean(),
            imag_mean: state.imag_mean(),
            real_error: state.real_stderr(),
            imag_error: state.imag_stderr(),
        }
    }
}

impl Default for GammaLoopAccumulatorState {
    fn default() -> Self {
        Self {
            estimate: VectorAccumulatorState::from_config(
                vec!["real".to_string(), "imag".to_string()],
                TrainingProjection::AbsComplex {
                    real: "real".to_string(),
                    imag: "imag".to_string(),
                },
                None,
            ),
            diagnostics: GammaLoopDiagnostics::default(),
        }
    }
}

fn safe_ratio(numerator: f64, denominator: i64) -> f64 {
    if denominator > 0 {
        numerator / denominator as f64
    } else {
        0.0
    }
}
