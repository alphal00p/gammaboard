use super::{GammaLoopAccumulatorState, ScalarAccumulatorState};
use crate::evaluation::evaluator::gammaloop::TrainingProjection;

impl GammaLoopAccumulatorState {
    /// The estimate's projection remains the complex norm, independently of
    /// which phase the evaluator returns for training.
    pub fn norm_statistics(&self) -> &ScalarAccumulatorState {
        &self.estimate.projection.state
    }

    pub fn training_statistics(&self) -> Option<&ScalarAccumulatorState> {
        match self.training_projection {
            Some(TrainingProjection::Real) => self.estimate.component("real").map(|c| &c.state),
            Some(TrainingProjection::Imag) => self.estimate.component("imag").map(|c| &c.state),
            Some(TrainingProjection::AbsSq) => self.training_norm_sqr.as_deref().filter(|state| {
                // Old snapshots do not contain squared-norm moments. Never
                // present moments collected only since an upgrade as full-run statistics.
                state.count + state.nan_count as i64
                    == self.norm_statistics().count + self.norm_statistics().nan_count as i64
            }),
            Some(TrainingProjection::Abs) | None => Some(self.norm_statistics()),
        }
    }

    pub fn rsd(&self) -> Option<f64> {
        self.training_statistics().map(ScalarAccumulatorState::rsd)
    }

    pub fn ess(&self) -> Option<f64> {
        self.training_statistics().map(ScalarAccumulatorState::ess)
    }

    pub(super) fn merge_training_statistics(&mut self, other: &Self) {
        if other.training_projection.is_some() {
            self.training_projection = other.training_projection;
        }
        if let Some(incoming) = &other.training_norm_sqr {
            if let Some(current) = &mut self.training_norm_sqr {
                current.merge_plain(incoming.as_ref().clone());
            } else {
                self.training_norm_sqr = Some(incoming.clone());
            }
        }
    }
}
