//! Evaluator worker runner orchestration.

use crate::batch::{BatchResult, PointSpec};
use crate::core::{StoreError, WorkQueueStore};
use crate::engines::{BuildError, EngineError, EvalError, Evaluator, Observable};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt, sync::Arc, time::Instant};

#[derive(Debug, Clone)]
pub struct EvaluatorRunnerTick {
    pub claimed_batch_id: Option<i64>,
    pub processed_samples: usize,
    pub eval_time_ms: f64,
}

#[derive(Debug)]
pub enum EvaluatorRunnerError {
    Build(BuildError),
    Engine(EngineError),
    Eval(EvalError),
    Store(StoreError),
}

impl fmt::Display for EvaluatorRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvaluatorRunnerError::Build(err) => write!(f, "{err}"),
            EvaluatorRunnerError::Engine(err) => write!(f, "{err}"),
            EvaluatorRunnerError::Eval(err) => write!(f, "{err}"),
            EvaluatorRunnerError::Store(err) => write!(f, "{err}"),
        }
    }
}

impl Error for EvaluatorRunnerError {}

impl From<StoreError> for EvaluatorRunnerError {
    fn from(value: StoreError) -> Self {
        EvaluatorRunnerError::Store(value)
    }
}

pub struct EvaluatorRunner<WQ> {
    run_id: i32,
    worker_id: String,
    evaluator: Box<dyn Evaluator>,
    build_observable:
        Arc<dyn Fn(&JsonValue) -> Result<Box<dyn Observable>, BuildError> + Send + Sync>,
    observable_params: JsonValue,
    point_spec: PointSpec,
    work_queue: WQ,
}

impl<WQ> EvaluatorRunner<WQ>
where
    WQ: WorkQueueStore,
{
    pub fn new(
        run_id: i32,
        worker_id: impl Into<String>,
        evaluator: Box<dyn Evaluator>,
        build_observable: Arc<
            dyn Fn(&JsonValue) -> Result<Box<dyn Observable>, BuildError> + Send + Sync,
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

    pub async fn tick(&mut self) -> Result<EvaluatorRunnerTick, EvaluatorRunnerError> {
        let claimed = self
            .work_queue
            .claim_batch(self.run_id, &self.worker_id)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        let Some(claimed) = claimed else {
            return Ok(EvaluatorRunnerTick {
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
                .map_err(EvaluatorRunnerError::Store)?;
            return Err(EvaluatorRunnerError::Engine(err));
        }

        let mut observable = match (self.build_observable)(&self.observable_params) {
            Ok(observable) => observable,
            Err(err) => {
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await
                    .map_err(EvaluatorRunnerError::Store)?;
                return Err(EvaluatorRunnerError::Build(err));
            }
        };

        let started = Instant::now();
        match self
            .evaluator
            .eval_batch(&claimed.batch, observable.as_mut())
        {
            Ok(result) => {
                self.submit_result(claimed.batch_id, &claimed.batch, result, started)
                    .await
            }
            Err(err) => {
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await
                    .map_err(EvaluatorRunnerError::Store)?;
                Err(EvaluatorRunnerError::Eval(err))
            }
        }
    }

    async fn submit_result(
        &self,
        batch_id: i64,
        batch: &crate::batch::Batch,
        result: BatchResult,
        started: Instant,
    ) -> Result<EvaluatorRunnerTick, EvaluatorRunnerError> {
        if !result.matches_batch(batch) {
            let err = EngineError::engine(format!(
                "result length mismatch for batch {}: expected {}, got {}",
                batch_id,
                batch.size(),
                result.len()
            ));
            self.work_queue
                .fail_batch(batch_id, &err.to_string())
                .await
                .map_err(EvaluatorRunnerError::Store)?;
            return Err(EvaluatorRunnerError::Engine(err));
        }

        let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.work_queue
            .submit_batch_results(batch_id, &result, eval_time_ms)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        Ok(EvaluatorRunnerTick {
            claimed_batch_id: Some(batch_id),
            processed_samples: result.len(),
            eval_time_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{Batch, BatchResult};
    use crate::core::BatchClaim;
    use crate::engines::{BuildError, EngineError, EvalError, Observable};
    use crate::runners::test_support::MockWorkQueue;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;

    #[derive(Default)]
    struct TestObservable {
        snapshot: JsonValue,
    }

    impl Observable for TestObservable {
        fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
            self.snapshot = state.clone();
            Ok(())
        }

        fn merge_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
            self.snapshot = state.clone();
            Ok(())
        }

        fn snapshot(&self) -> Result<JsonValue, EngineError> {
            Ok(self.snapshot.clone())
        }
    }

    struct OkEvaluator;

    impl Evaluator for OkEvaluator {
        fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
            if point_spec.continuous_dims != 1 || point_spec.discrete_dims != 0 {
                return Err(BuildError::build(
                    "OkEvaluator expects 1 continuous and 0 discrete",
                ));
            }
            Ok(())
        }

        fn eval_batch(
            &self,
            batch: &Batch,
            observable: &mut dyn Observable,
        ) -> Result<BatchResult, EvalError> {
            let values = batch
                .continuous()
                .column(0)
                .iter()
                .copied()
                .collect::<Vec<f64>>();
            let snapshot = json!({
                "count": values.len() as i64,
                "sum": values.iter().sum::<f64>(),
                "sum_abs": values.iter().map(|v| v.abs()).sum::<f64>(),
                "sum_sq": values.iter().map(|v| v * v).sum::<f64>(),
            });
            observable
                .merge_state_from_json(&snapshot)
                .map_err(|err| EvalError::eval(err.to_string()))?;
            let snapshot = observable
                .snapshot()
                .map_err(|err| EvalError::eval(err.to_string()))?;
            Ok(BatchResult::new(values, snapshot))
        }

        fn supports_observable(&self, _observable: &crate::engines::ObservableEngine) -> bool {
            true
        }
    }

    struct FailingEvaluator;

    impl Evaluator for FailingEvaluator {
        fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
            Ok(())
        }

        fn eval_batch(
            &self,
            _batch: &Batch,
            _observable: &mut dyn Observable,
        ) -> Result<BatchResult, EvalError> {
            Err(EvalError::eval("mock failure"))
        }

        fn supports_observable(&self, _observable: &crate::engines::ObservableEngine) -> bool {
            false
        }
    }

    fn sample_batch() -> Batch {
        Batch::from_flat_data(2, 1, 0, vec![1.0, 2.0], vec![]).expect("sample batch")
    }

    fn build_test_observable(_params: &JsonValue) -> Result<Box<dyn Observable>, BuildError> {
        let _ = serde_json::from_value::<serde_json::Map<String, JsonValue>>(_params.clone())
            .map_err(|err| BuildError::build(format!("invalid test observable params: {err}")))?;
        Ok(Box::new(TestObservable {
            snapshot: json!({}),
        }))
    }

    #[tokio::test]
    async fn tick_returns_empty_when_no_batch_claimed() {
        let queue = MockWorkQueue::default();
        let mut runner = EvaluatorRunner::new(
            1,
            "worker-1",
            Box::new(OkEvaluator),
            Arc::new(build_test_observable),
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

        let mut runner = EvaluatorRunner::new(
            1,
            "worker-1",
            Box::new(OkEvaluator),
            Arc::new(build_test_observable),
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
        assert_eq!(state.submitted[0].1.values, vec![1.0, 2.0]);
        assert!(state.failed.is_empty());
    }

    #[tokio::test]
    async fn tick_marks_batch_failed_on_eval_error() {
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").next_claim = Some(BatchClaim {
            batch_id: 99,
            batch: sample_batch(),
        });

        let mut runner = EvaluatorRunner::new(
            1,
            "worker-1",
            Box::new(FailingEvaluator),
            Arc::new(build_test_observable),
            json!({}),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            queue.clone(),
        );

        let err = runner.tick().await.expect_err("expected eval error");
        let state = queue.inner.lock().expect("poison").clone();

        assert!(matches!(err, EvaluatorRunnerError::Eval(_)));
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.failed[0].0, 99);
        assert!(state.submitted.is_empty());
    }

    #[tokio::test]
    async fn tick_marks_batch_failed_on_result_len_mismatch() {
        struct BadEvaluator;
        impl Evaluator for BadEvaluator {
            fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
                Ok(())
            }
            fn eval_batch(
                &self,
                _batch: &Batch,
                _observable: &mut dyn Observable,
            ) -> Result<BatchResult, EvalError> {
                Ok(BatchResult::new(vec![1.0], json!({})))
            }
            fn supports_observable(&self, _observable: &crate::engines::ObservableEngine) -> bool {
                true
            }
        }

        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").next_claim = Some(BatchClaim {
            batch_id: 7,
            batch: sample_batch(),
        });
        let mut runner = EvaluatorRunner::new(
            1,
            "worker-1",
            Box::new(BadEvaluator),
            Arc::new(build_test_observable),
            json!({}),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            queue.clone(),
        );

        let err = runner.tick().await.expect_err("expected mismatch");
        let state = queue.inner.lock().expect("poison").clone();
        assert!(matches!(err, EvaluatorRunnerError::Engine(_)));
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.failed[0].0, 7);
    }
}
