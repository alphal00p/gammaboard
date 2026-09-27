use serde::{Deserialize, Serialize};

use super::{Accumulator, GammaLoopDiagnostics, VectorAccumulatorState};
use crate::core::{AccumulatorMomentConfig, EngineError, RunSpec, TrainingProjection};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaLoopAccumulatorState {
    pub estimate: VectorAccumulatorState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_projection: Option<crate::evaluation::evaluator::gammaloop::TrainingProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_norm_sqr: Option<Box<super::ScalarAccumulatorState>>,
    #[serde(default)]
    pub diagnostics: GammaLoopDiagnostics,
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
        self.merge_training_statistics(&other);
        self.diagnostics.merge_in_place(other.diagnostics);
        Accumulator::merge(&mut self.estimate, other.estimate);
        Ok(())
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.estimate.signal_to_noise()
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
                TrainingProjection::Norm,
                None,
                // Mirror the enabled accumulator: track 4th-order moments so the
                // RSD metric can carry an uncertainty.
                AccumulatorMomentConfig::MaxOrder4,
            ),
            diagnostics: GammaLoopDiagnostics::default(),
            training_projection: None,
            training_norm_sqr: None,
        }
    }
}
