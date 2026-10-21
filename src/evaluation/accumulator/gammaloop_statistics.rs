use super::{GammaLoopAccumulatorState, ScalarAccumulatorState};
use crate::evaluation::evaluator::gammaloop::TrainingProjection;

impl GammaLoopAccumulatorState {
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
        self.estimate.projection().state.mean()
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
        self.estimate.projection().state.stderr()
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.estimate.signal_to_noise()
    }

    pub(crate) fn validate_values(&self) -> Result<(), crate::core::EngineError> {
        #[cfg(feature = "gammaloop")]
        super::gammaloop::validate_histogram_bundle(&self.bundle)?;
        self.estimate.validate()?;
        if let Some(state) = &self.training_norm_sqr {
            state.validate()?;
            if state.count != self.estimate.projection().state.count {
                return Err(crate::core::EngineError::eval(
                    "inconsistent GammaLoop training sample count",
                ));
            }
        }
        Ok(())
    }

    /// The estimate's projection remains the complex norm, independently of
    /// which phase the evaluator returns for training.
    pub fn norm_statistics(&self) -> &ScalarAccumulatorState {
        &self.estimate.projection().state
    }

    pub fn training_statistics(&self) -> Option<&ScalarAccumulatorState> {
        match self.training_projection {
            Some(TrainingProjection::Real) => self.estimate.component("real").map(|c| &c.state),
            Some(TrainingProjection::Imag) => self.estimate.component("imag").map(|c| &c.state),
            Some(TrainingProjection::AbsSq) => self.training_norm_sqr.as_deref().filter(|state| {
                // A partial moment set cannot describe the complete accumulator.
                state.count == self.norm_statistics().count
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

    pub(super) fn validate_training_statistics(
        &self,
        other: &Self,
    ) -> Result<(), crate::core::EngineError> {
        if self.training_projection != other.training_projection
            || self.training_norm_sqr.is_some() != other.training_norm_sqr.is_some()
        {
            return Err(crate::core::EngineError::engine(
                "incompatible GammaLoop training statistics",
            ));
        }
        if let (Some(left), Some(right)) = (&self.training_norm_sqr, &other.training_norm_sqr) {
            left.validate_merge(right)?;
        }
        Ok(())
    }

    pub(super) fn merge_training_statistics(&mut self, other: &Self) {
        if let Some(incoming) = &other.training_norm_sqr {
            if let Some(current) = &mut self.training_norm_sqr {
                current.merge_plain(incoming.as_ref().clone());
            } else {
                self.training_norm_sqr = Some(incoming.clone());
            }
        }
    }
}
