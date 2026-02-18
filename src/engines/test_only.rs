//! Test-only runtime implementations used by local control-plane smoke tests.

use crate::{
    EngineError, EngineState, EvalError, Evaluator, SamplerAggregatorEngine, WeightedPoint,
};
use rand::Rng;
use serde_json::{Value as JsonValue, json};
use std::{thread, time::Duration};

/// Test-only evaluator used for local end-to-end runs.
pub struct TestOnlySinEvaluator;

impl Evaluator for TestOnlySinEvaluator {
    fn eval_point(&self, point: &JsonValue) -> Result<f64, EvalError> {
        let x = point
            .as_f64()
            .ok_or_else(|| EvalError::new("expected f64 point"))?;
        Ok(x.sin() * (-x * x).exp())
    }
}

/// Test-only sampler-aggregator engine with simple random batch generation.
pub struct TestOnlyTrainingSamplerAggregatorEngine {
    batch_size: usize,
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
    trained_samples: usize,
    nr_batches: i64,
    nr_samples: i64,
    sum: f64,
}

impl TestOnlyTrainingSamplerAggregatorEngine {
    pub fn new(
        batch_size: usize,
        training_target_samples: usize,
        training_delay_per_sample_ms: u64,
    ) -> Self {
        Self {
            batch_size,
            training_target_samples,
            training_delay_per_sample_ms,
            trained_samples: 0,
            nr_batches: 0,
            nr_samples: 0,
            sum: 0.0,
        }
    }
}

impl SamplerAggregatorEngine for TestOnlyTrainingSamplerAggregatorEngine {
    fn implementation(&self) -> &'static str {
        "test_only_training_sampler_aggregator"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn init(&mut self, _state: Option<EngineState>) -> Result<(), EngineError> {
        Ok(())
    }

    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<crate::Batch>, EngineError> {
        let mut rng = rand::thread_rng();
        let mut out = Vec::with_capacity(max_batches);

        for _ in 0..max_batches {
            let mut points = Vec::with_capacity(self.batch_size);
            for _ in 0..self.batch_size {
                let x = rng.r#gen::<f64>() * 10.0;
                let w = 0.5 + rng.r#gen::<f64>();
                points.push(WeightedPoint::new(json!(x), w));
            }
            out.push(crate::Batch::new(points));
        }

        Ok(out)
    }

    fn ingest_completed(&mut self, completed: &[crate::CompletedBatch]) -> Result<(), EngineError> {
        let mut new_samples = 0usize;

        for batch in completed {
            if batch.results.values.len() != batch.batch.points.len() {
                continue;
            }
            self.nr_batches += 1;
            self.nr_samples += batch.results.values.len() as i64;
            new_samples += batch.results.values.len();
            self.sum += batch.results.values.iter().sum::<f64>();
        }

        if new_samples > 0 && self.training_delay_per_sample_ms > 0 {
            let remaining_training = self
                .training_target_samples
                .saturating_sub(self.trained_samples);
            let delayed_samples = remaining_training.min(new_samples);
            if delayed_samples > 0 {
                thread::sleep(Duration::from_millis(
                    delayed_samples as u64 * self.training_delay_per_sample_ms,
                ));
            }
        }
        self.trained_samples = self.trained_samples.saturating_add(new_samples);

        let mean = if self.nr_samples > 0 {
            self.sum / self.nr_samples as f64
        } else {
            0.0
        };
        println!(
            "📈 test-only engine: batches={}, samples={}, mean={mean:.6}",
            self.nr_batches, self.nr_samples
        );

        Ok(())
    }
}
