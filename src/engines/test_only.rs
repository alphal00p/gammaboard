//! Test-only runtime implementations used by local control-plane smoke tests.

use crate::batch::{Batch, EvaluatedSample, WeightedPoint};
use crate::contracts::{
    AggregatedObservable, AggregatedObservableFactory, BuildError, EngineError, EngineState,
    EvalError, Evaluator, SamplerAggregatorEngine,
};
use rand::Rng;
use serde_json::{Value as JsonValue, json};
use std::{
    thread,
    time::{Duration, Instant},
};

/// Test-only evaluator used for local end-to-end runs.
pub struct TestOnlySinEvaluator {
    min_eval_time_per_sample_ms: u64,
}

impl TestOnlySinEvaluator {
    pub fn new(min_eval_time_per_sample_ms: u64) -> Self {
        Self {
            min_eval_time_per_sample_ms,
        }
    }
}

impl Evaluator for TestOnlySinEvaluator {
    fn eval_point(&self, point: &JsonValue) -> Result<EvaluatedSample, EvalError> {
        let x = point
            .as_f64()
            .ok_or_else(|| EvalError::eval("expected f64 point"))?;
        let value = x.sin() * (-x * x).exp();
        Ok(EvaluatedSample {
            weight: value,
            observable: json!({
                "value": value,
                "x": x,
            }),
        })
    }

    fn eval_batch(&self, batch: &Batch) -> Result<Vec<EvaluatedSample>, EvalError> {
        let started = Instant::now();
        let mut samples = Vec::with_capacity(batch.size());
        for p in &batch.points {
            samples.push(self.eval_point(&p.point)?);
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        Ok(samples)
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

    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError> {
        let mut rng = rand::thread_rng();
        let mut out = Vec::with_capacity(max_batches);

        for _ in 0..max_batches {
            let mut points = Vec::with_capacity(self.batch_size);
            for _ in 0..self.batch_size {
                let x = rng.r#gen::<f64>() * 10.0;
                let w = 0.5 + rng.r#gen::<f64>();
                points.push(WeightedPoint::new(json!(x), w));
            }
            out.push(Batch::new(points));
        }

        Ok(out)
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        self.nr_batches += 1;
        self.nr_samples += training_weights.len() as i64;
        self.sum += training_weights.iter().sum::<f64>();

        if !training_weights.is_empty() && self.training_delay_per_sample_ms > 0 {
            let remaining_training = self
                .training_target_samples
                .saturating_sub(self.trained_samples);
            let delayed_samples = remaining_training.min(training_weights.len());
            if delayed_samples > 0 {
                thread::sleep(Duration::from_millis(
                    delayed_samples as u64 * self.training_delay_per_sample_ms,
                ));
            }
        }
        self.trained_samples = self.trained_samples.saturating_add(training_weights.len());

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

pub struct TestOnlyObservableAggregator {
    nr_samples: i64,
    sum: f64,
}

impl TestOnlyObservableAggregator {
    pub fn new() -> Self {
        Self {
            nr_samples: 0,
            sum: 0.0,
        }
    }
}

impl AggregatedObservable for TestOnlyObservableAggregator {
    fn implementation(&self) -> &'static str {
        "test_only_observable"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn restore(&mut self, snapshot: Option<JsonValue>) -> Result<(), EngineError> {
        if let Some(snapshot) = snapshot {
            self.nr_samples = snapshot
                .get("nr_samples")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| EngineError::engine("missing nr_samples in observable snapshot"))?;
            self.sum = snapshot
                .get("sum")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| EngineError::engine("missing sum in observable snapshot"))?;
        }
        Ok(())
    }

    fn ingest_sample_observable(
        &mut self,
        sample_observable: &JsonValue,
    ) -> Result<(), EngineError> {
        let value = sample_observable
            .get("value")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| EngineError::engine("sample observable missing numeric value"))?;
        self.nr_samples += 1;
        self.sum += value;
        Ok(())
    }

    fn ingest_batch_observable(&mut self, batch_observable: &JsonValue) -> Result<(), EngineError> {
        let nr_samples = batch_observable
            .get("nr_samples")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| EngineError::engine("batch observable missing nr_samples"))?;
        let sum = batch_observable
            .get("sum")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| EngineError::engine("batch observable missing sum"))?;
        self.nr_samples += nr_samples;
        self.sum += sum;
        Ok(())
    }

    fn snapshot(&self) -> Result<JsonValue, EngineError> {
        let mean = if self.nr_samples > 0 {
            Some(self.sum / self.nr_samples as f64)
        } else {
            None
        };

        Ok(json!({
            "nr_samples": self.nr_samples,
            "sum": self.sum,
            "mean": mean,
        }))
    }
}

#[derive(Clone, Copy, Default)]
pub struct TestOnlyObservableAggregatorFactory;

impl AggregatedObservableFactory for TestOnlyObservableAggregatorFactory {
    fn implementation(&self) -> &'static str {
        "test_only_observable"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn build(&self, _params: &JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> {
        Ok(Box::new(TestOnlyObservableAggregator::new()))
    }
}
