use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator, IngestScalar,
    ingest_scalar_values,
};
use crate::utils::domain::Domain;
use crate::utils::synthetic_timing::{TimingModel, TimingStats};
use serde::{Deserialize, Serialize};

/// Evaluator that returns 1.0 for every sample.
pub struct UnitEvaluator {
    domain: Domain,
    fail_on_batch_nrs: Vec<usize>,
    timing: TimingModel,
    cpu_iterations_per_sample: u64,
    eval_batches_total: usize,
    timing_stats: TimingStats,
}

impl UnitEvaluator {
    pub fn new(domain: Domain, fail_on_batch_nrs: Vec<usize>, timing: TimingModel) -> Self {
        Self {
            domain,
            fail_on_batch_nrs,
            timing,
            cpu_iterations_per_sample: 0,
            eval_batches_total: 0,
            timing_stats: TimingStats::default(),
        }
    }

    pub fn from_params(params: UnitEvaluatorParams) -> Result<Self, BuildError> {
        params.timing.validate()?;
        if params.fail_on_build {
            return Err(BuildError::build("unit evaluator injected build failure"));
        }
        let mut evaluator = Self::new(
            Domain::rectangular(params.continuous_dims, params.discrete_dims),
            params.fail_on_batch_nrs,
            params.timing,
        );
        evaluator.cpu_iterations_per_sample = params.cpu_iterations_per_sample;
        Ok(evaluator)
    }

    fn scalar_ingestor(state: &mut AccumulatorState) -> Result<&mut dyn IngestScalar, EvalError> {
        match state {
            AccumulatorState::Empty(accumulator) => Ok(accumulator),
            AccumulatorState::Vector(accumulator) => Ok(accumulator),
            AccumulatorState::FullVector(accumulator) => Ok(accumulator),
            other => Err(EvalError::eval(format!(
                "unit evaluator scalar mode does not support accumulator kind {}",
                other.kind_str()
            ))),
        }
    }

    fn should_fail_on_batch(&self, batch_nr: usize) -> bool {
        self.fail_on_batch_nrs
            .iter()
            .any(|n| *n > 0 && *n == batch_nr)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UnitEvaluatorParams {
    pub continuous_dims: usize,
    pub discrete_dims: usize,
    #[serde(default)]
    pub fail_on_batch_nrs: Vec<usize>,
    #[serde(default)]
    pub fail_on_build: bool,
    #[serde(default)]
    pub timing: TimingModel,
    /// Fixed arithmetic work, independent of clocks and worker count.
    pub cpu_iterations_per_sample: u64,
}

impl Default for UnitEvaluatorParams {
    fn default() -> Self {
        Self {
            continuous_dims: 1,
            discrete_dims: 0,
            fail_on_batch_nrs: Vec::new(),
            fail_on_build: false,
            timing: TimingModel::default(),
            cpu_iterations_per_sample: 0,
        }
    }
}

impl Evaluator for UnitEvaluator {
    fn metadata(&self) -> serde_json::Value {
        serde_json::json!({"synthetic":true,"kind":"unit","timing":self.timing,"cpu_iterations_per_sample":self.cpu_iterations_per_sample})
    }

    fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"synthetic":true,"timing":self.timing_stats})
    }

    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        self.eval_batches_total = self.eval_batches_total.saturating_add(1);
        if self.should_fail_on_batch(self.eval_batches_total) {
            return Err(EvalError::eval(format!(
                "unit evaluator injected failure on batch {}",
                self.eval_batches_total
            )));
        }
        let mut observable_state = AccumulatorState::from_config(accumulator);
        // Hash batch endpoints and size: stable across worker assignment/retries.
        let mut key = batch.size() as u64;
        for point in batch
            .points()
            .first()
            .into_iter()
            .chain(batch.points().last())
        {
            for bits in point
                .continuous
                .iter()
                .map(|x| x.to_bits())
                .chain(point.discrete.iter().map(|x| *x as u64))
            {
                key = (key ^ bits).wrapping_mul(0x100000001b3);
            }
        }
        self.timing
            .wait(batch.size(), key, &mut self.timing_stats)?;
        cpu_work(self.cpu_iterations_per_sample, batch.size());
        let values = vec![1.0; batch.size()];
        let weighted_values = ingest_scalar_values(
            &values,
            batch.points(),
            options.require_training_values,
            Self::scalar_ingestor(&mut observable_state)?,
        )?;
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

/// Fixed work rather than a wall-clock spin, so contention remains measurable.
pub(crate) fn cpu_work(iterations: u64, samples: usize) {
    if iterations == 0 {
        return;
    }
    let mut value = std::hint::black_box(0x9e3779b97f4a7c15_u64);
    for _ in 0..samples {
        for _ in 0..iterations {
            value = value.wrapping_mul(6364136223846793005).rotate_left(17) ^ 1442695040888963407;
        }
        std::hint::black_box(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{Batch, Point};

    #[test]
    fn eval_batch_returns_weighted_ones_for_scalar_observable() {
        let batch = Batch::from_points([
            Point::new(vec![0.0], Vec::new(), 2.0),
            Point::new(vec![1.0], Vec::new(), 3.0),
        ])
        .expect("batch");
        let mut evaluator =
            UnitEvaluator::new(Domain::continuous(1), Vec::new(), TimingModel::default());

        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        let AccumulatorState::Vector(accumulator) = result.accumulator else {
            panic!("expected vector accumulator");
        };
        assert_eq!(accumulator.components.len(), 1);
        assert_eq!(accumulator.components[0].name, "value");
        assert_eq!(accumulator.components[0].state.count, 2);
        assert_eq!(accumulator.components[0].state.sum_weighted_value, 5.0);
    }
}
