//! Evaluator worker runner orchestration.

use crate::batch::BatchResults;
use crate::contracts::{
    AggregatedObservableFactory, BuildError, EngineError, EvalError, Evaluator, StoreError,
    WorkQueueStore,
};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt, time::Instant};

#[derive(Debug, Clone)]
pub struct WorkerTick {
    pub claimed_batch_id: Option<i64>,
    pub processed_samples: usize,
    pub eval_time_ms: f64,
}

#[derive(Debug)]
pub enum WorkerRunnerError {
    Build(BuildError),
    Engine(EngineError),
    Eval(EvalError),
    Store(StoreError),
}

impl fmt::Display for WorkerRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerRunnerError::Build(err) => write!(f, "{err}"),
            WorkerRunnerError::Engine(err) => write!(f, "{err}"),
            WorkerRunnerError::Eval(err) => write!(f, "{err}"),
            WorkerRunnerError::Store(err) => write!(f, "{err}"),
        }
    }
}

impl Error for WorkerRunnerError {}

impl From<StoreError> for WorkerRunnerError {
    fn from(value: StoreError) -> Self {
        WorkerRunnerError::Store(value)
    }
}

pub struct WorkerRunner<E, AOF, WQ> {
    run_id: i32,
    worker_id: String,
    evaluator: E,
    aggregated_observable_factory: AOF,
    observable_params: JsonValue,
    work_queue: WQ,
}

impl<E, AOF, WQ> WorkerRunner<E, AOF, WQ>
where
    E: Evaluator,
    AOF: AggregatedObservableFactory,
    WQ: WorkQueueStore,
{
    pub fn new(
        run_id: i32,
        worker_id: impl Into<String>,
        evaluator: E,
        aggregated_observable_factory: AOF,
        observable_params: JsonValue,
        work_queue: WQ,
    ) -> Self {
        Self {
            run_id,
            worker_id: worker_id.into(),
            evaluator,
            aggregated_observable_factory,
            observable_params,
            work_queue,
        }
    }

    pub async fn tick(&mut self) -> Result<WorkerTick, WorkerRunnerError> {
        let claimed = self
            .work_queue
            .claim_batch(self.run_id, &self.worker_id)
            .await
            .map_err(WorkerRunnerError::Store)?;

        let Some(claimed) = claimed else {
            return Ok(WorkerTick {
                claimed_batch_id: None,
                processed_samples: 0,
                eval_time_ms: 0.0,
            });
        };

        let started = Instant::now();

        match self.evaluator.eval_batch(&claimed.batch) {
            Ok(samples) => {
                let mut batch_observable = match self
                    .aggregated_observable_factory
                    .build(&self.observable_params)
                {
                    Ok(observable) => observable,
                    Err(err) => {
                        self.work_queue
                            .fail_batch(claimed.batch_id, &err.to_string())
                            .await
                            .map_err(WorkerRunnerError::Store)?;
                        return Err(WorkerRunnerError::Build(err));
                    }
                };

                for sample in &samples {
                    if let Err(err) = batch_observable.ingest_sample_observable(&sample.observable)
                    {
                        self.work_queue
                            .fail_batch(claimed.batch_id, &err.to_string())
                            .await
                            .map_err(WorkerRunnerError::Store)?;
                        return Err(WorkerRunnerError::Engine(err));
                    }
                }

                let batch_observable_snapshot = match batch_observable.snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        self.work_queue
                            .fail_batch(claimed.batch_id, &err.to_string())
                            .await
                            .map_err(WorkerRunnerError::Store)?;
                        return Err(WorkerRunnerError::Engine(err));
                    }
                };

                let training_results = BatchResults::from_evaluated_samples(&samples);
                let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;

                self.work_queue
                    .submit_batch_results(
                        claimed.batch_id,
                        &training_results,
                        &batch_observable_snapshot,
                        eval_time_ms,
                    )
                    .await
                    .map_err(WorkerRunnerError::Store)?;

                Ok(WorkerTick {
                    claimed_batch_id: Some(claimed.batch_id),
                    processed_samples: samples.len(),
                    eval_time_ms,
                })
            }
            Err(err) => {
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await
                    .map_err(WorkerRunnerError::Store)?;
                Err(WorkerRunnerError::Eval(err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{Batch, EvaluatedSample, WeightedPoint};
    use crate::contracts::{
        AggregatedObservable, AggregatedObservableFactory, BatchClaim, EvalError,
    };
    use crate::runners::test_support::MockWorkQueue;
    use serde_json::{Value as JsonValue, json};

    struct TestObservable;

    impl AggregatedObservable for TestObservable {
        fn implementation(&self) -> &'static str {
            "test_observable"
        }

        fn version(&self) -> &'static str {
            "v1"
        }

        fn restore(&mut self, _snapshot: Option<JsonValue>) -> Result<(), EngineError> {
            Ok(())
        }

        fn ingest_sample_observable(
            &mut self,
            _sample_observable: &JsonValue,
        ) -> Result<(), EngineError> {
            Ok(())
        }

        fn ingest_batch_observable(
            &mut self,
            _batch_observable: &JsonValue,
        ) -> Result<(), EngineError> {
            Ok(())
        }

        fn snapshot(&self) -> Result<JsonValue, EngineError> {
            Ok(JsonValue::Null)
        }
    }

    struct TestObservableFactory;

    impl AggregatedObservableFactory for TestObservableFactory {
        fn implementation(&self) -> &'static str {
            "test_observable"
        }

        fn version(&self) -> &'static str {
            "v1"
        }

        fn build(&self, _params: &JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> {
            Ok(Box::new(TestObservable))
        }
    }

    struct OkEvaluator;

    impl Evaluator for OkEvaluator {
        fn eval_point(&self, point: &serde_json::Value) -> Result<EvaluatedSample, EvalError> {
            let value = point
                .as_f64()
                .ok_or_else(|| EvalError::eval("expected numeric point"))?;
            Ok(EvaluatedSample::weight_only(value))
        }
    }

    struct FailingEvaluator;

    impl Evaluator for FailingEvaluator {
        fn eval_point(&self, _point: &serde_json::Value) -> Result<EvaluatedSample, EvalError> {
            Err(EvalError::eval("mock failure"))
        }
    }

    fn sample_batch() -> Batch {
        Batch::new(vec![
            WeightedPoint::new(json!(1.0), 1.0),
            WeightedPoint::new(json!(2.0), 1.0),
        ])
    }

    #[tokio::test]
    async fn tick_returns_empty_when_no_batch_claimed() {
        let queue = MockWorkQueue::default();
        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            OkEvaluator,
            TestObservableFactory,
            json!({}),
            queue,
        );

        let tick = runner.tick().await.expect("tick");

        assert!(tick.claimed_batch_id.is_none());
        assert_eq!(tick.processed_samples, 0);
    }

    #[tokio::test]
    async fn tick_submits_results_on_success() {
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").next_claim = Some(BatchClaim {
            batch_id: 42,
            batch: sample_batch(),
        });

        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            OkEvaluator,
            TestObservableFactory,
            json!({}),
            queue.clone(),
        );

        let tick = runner.tick().await.expect("tick");
        let state = queue.inner.lock().expect("poison").clone();

        assert_eq!(tick.claimed_batch_id, Some(42));
        assert_eq!(tick.processed_samples, 2);
        assert_eq!(state.submitted.len(), 1);
        assert_eq!(state.submitted[0].0, 42);
        assert!(state.failed.is_empty());
    }

    #[tokio::test]
    async fn tick_marks_batch_failed_on_eval_error() {
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").next_claim = Some(BatchClaim {
            batch_id: 99,
            batch: sample_batch(),
        });

        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            FailingEvaluator,
            TestObservableFactory,
            json!({}),
            queue.clone(),
        );

        let err = runner.tick().await.expect_err("expected eval error");
        let state = queue.inner.lock().expect("poison").clone();

        assert!(matches!(err, WorkerRunnerError::Eval(_)));
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.failed[0].0, 99);
        assert!(state.submitted.is_empty());
    }
}
