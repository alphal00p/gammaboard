//! Evaluator worker runner orchestration.

use crate::batch::BatchResults;
use crate::batch::PointSpec;
use crate::core::{StoreError, WorkQueueStore};
use crate::engines::{AggregatedObservable, BuildError, EngineError, EvalError, Evaluator};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt, sync::Arc, time::Instant};

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

pub struct WorkerRunner<E, WQ> {
    run_id: i32,
    worker_id: String,
    evaluator: E,
    build_observable:
        Arc<dyn Fn(&JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> + Send + Sync>,
    observable_params: JsonValue,
    point_spec: PointSpec,
    work_queue: WQ,
}

impl<E, WQ> WorkerRunner<E, WQ>
where
    E: Evaluator,
    WQ: WorkQueueStore,
{
    pub fn new(
        run_id: i32,
        worker_id: impl Into<String>,
        evaluator: E,
        build_observable: Arc<
            dyn Fn(&JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> + Send + Sync,
        >,
        observable_params: JsonValue,
        point_spec: PointSpec,
        work_queue: WQ,
    ) -> Self {
        Self {
            run_id,
            worker_id: worker_id.into(),
            evaluator,
            build_observable,
            observable_params,
            point_spec,
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

        if let Err(err) = claimed.batch.validate_point_spec(&self.point_spec) {
            let err = EngineError::engine(format!("invalid batch point shape: {err}"));
            self.work_queue
                .fail_batch(claimed.batch_id, &err.to_string())
                .await
                .map_err(WorkerRunnerError::Store)?;
            return Err(WorkerRunnerError::Engine(err));
        }

        let started = Instant::now();

        match self.evaluator.eval_batch(&claimed.batch) {
            Ok(samples) => {
                let mut batch_observable = match (self.build_observable)(&self.observable_params) {
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
    use crate::batch::{Batch, EvaluatedSample, Point, PointSpec, PointView, WeightedPoint};
    use crate::core::BatchClaim;
    use crate::engines::{AggregatedObservable, EvalError};
    use crate::runners::test_support::MockWorkQueue;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;

    struct TestObservable;

    impl AggregatedObservable for TestObservable {
        fn from_params(_params: &JsonValue) -> Result<Self, BuildError>
        where
            Self: Sized,
        {
            Ok(Self)
        }

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

    struct OkEvaluator;

    impl Evaluator for OkEvaluator {
        fn from_params(_params: &JsonValue) -> Result<Self, BuildError>
        where
            Self: Sized,
        {
            Ok(Self)
        }

        fn implementation(&self) -> &'static str {
            "ok_evaluator"
        }

        fn version(&self) -> &'static str {
            "v1"
        }

        fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
            if point_spec.continuous_dims != 1 || point_spec.discrete_dims != 0 {
                return Err(BuildError::build(
                    "OkEvaluator expects 1 continuous and 0 discrete",
                ));
            }
            Ok(())
        }

        fn eval_point(&self, point: PointView<'_>) -> Result<EvaluatedSample, EvalError> {
            let value = *point
                .continuous()
                .first()
                .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
            Ok(EvaluatedSample::weight_only(value))
        }
    }

    struct FailingEvaluator;

    impl Evaluator for FailingEvaluator {
        fn from_params(_params: &JsonValue) -> Result<Self, BuildError>
        where
            Self: Sized,
        {
            Ok(Self)
        }

        fn implementation(&self) -> &'static str {
            "failing_evaluator"
        }

        fn version(&self) -> &'static str {
            "v1"
        }

        fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
            Ok(())
        }

        fn eval_point(&self, _point: PointView<'_>) -> Result<EvaluatedSample, EvalError> {
            Err(EvalError::eval("mock failure"))
        }
    }

    fn sample_batch() -> Batch {
        Batch::new(vec![
            WeightedPoint::new(Point::scalar_continuous(1.0), 1.0),
            WeightedPoint::new(Point::scalar_continuous(2.0), 1.0),
        ])
    }

    fn build_test_observable()
    -> Arc<dyn Fn(&JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> + Send + Sync>
    {
        Arc::new(|_params: &JsonValue| Ok(Box::new(TestObservable)))
    }

    #[tokio::test]
    async fn tick_returns_empty_when_no_batch_claimed() {
        let queue = MockWorkQueue::default();
        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            OkEvaluator,
            build_test_observable(),
            json!({}),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
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
            build_test_observable(),
            json!({}),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
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
            build_test_observable(),
            json!({}),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
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
