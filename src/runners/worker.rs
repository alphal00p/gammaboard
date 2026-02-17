//! Evaluator worker runner orchestration.

use crate::contracts::{EvalError, Evaluator, StoreError, WorkQueueStore};
use std::{
    error::Error,
    fmt,
    time::{Duration, Instant},
};
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct WorkerRunnerConfig {
    pub min_eval_time_per_sample: Duration,
}

impl Default for WorkerRunnerConfig {
    fn default() -> Self {
        Self {
            min_eval_time_per_sample: Duration::from_millis(0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerTick {
    pub claimed_batch_id: Option<i64>,
    pub processed_samples: usize,
    pub eval_time_ms: f64,
}

#[derive(Debug)]
pub enum WorkerRunnerError {
    Eval(EvalError),
    Store(StoreError),
}

impl fmt::Display for WorkerRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerRunnerError::Eval(err) => write!(f, "{err}"),
            WorkerRunnerError::Store(err) => write!(f, "{err}"),
        }
    }
}

impl Error for WorkerRunnerError {}

impl From<EvalError> for WorkerRunnerError {
    fn from(value: EvalError) -> Self {
        WorkerRunnerError::Eval(value)
    }
}

impl From<StoreError> for WorkerRunnerError {
    fn from(value: StoreError) -> Self {
        WorkerRunnerError::Store(value)
    }
}

pub struct WorkerRunner<E, WQ> {
    run_id: i32,
    instance_id: String,
    evaluator: E,
    work_queue: WQ,
    config: WorkerRunnerConfig,
}

impl<E, WQ> WorkerRunner<E, WQ>
where
    E: Evaluator,
    WQ: WorkQueueStore,
{
    pub fn new(
        run_id: i32,
        instance_id: impl Into<String>,
        evaluator: E,
        work_queue: WQ,
        config: WorkerRunnerConfig,
    ) -> Self {
        Self {
            run_id,
            instance_id: instance_id.into(),
            evaluator,
            work_queue,
            config,
        }
    }

    pub async fn tick(&mut self) -> Result<WorkerTick, WorkerRunnerError> {
        let claimed = self
            .work_queue
            .claim_batch(self.run_id, &self.instance_id)
            .await?;

        let Some(claimed) = claimed else {
            return Ok(WorkerTick {
                claimed_batch_id: None,
                processed_samples: 0,
                eval_time_ms: 0.0,
            });
        };

        let batch_size = claimed.batch.size();
        let started = Instant::now();

        match self.evaluator.eval_batch(&claimed.batch) {
            Ok(results) => {
                let elapsed = started.elapsed();
                let min_total = self
                    .config
                    .min_eval_time_per_sample
                    .mul_f64(batch_size as f64);
                if elapsed < min_total {
                    sleep(min_total - elapsed).await;
                }
                let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;

                self.work_queue
                    .submit_batch_results(claimed.batch_id, &results, eval_time_ms)
                    .await?;

                Ok(WorkerTick {
                    claimed_batch_id: Some(claimed.batch_id),
                    processed_samples: batch_size,
                    eval_time_ms,
                })
            }
            Err(err) => {
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await?;
                Err(WorkerRunnerError::Eval(err))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Batch, BatchClaim, BatchResults, CompletedBatch, EvalError, WeightedPoint};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct QueueState {
        next_claim: Option<BatchClaim>,
        submitted: Vec<(i64, BatchResults)>,
        failed: Vec<(i64, String)>,
    }

    #[derive(Clone, Default)]
    struct TestQueue {
        inner: Arc<Mutex<QueueState>>,
    }

    impl WorkQueueStore for TestQueue {
        async fn insert_batch(&self, _run_id: i32, _batch: &Batch) -> Result<(), StoreError> {
            Ok(())
        }

        async fn get_pending_batch_count(&self, _run_id: i32) -> Result<i64, StoreError> {
            Ok(0)
        }

        async fn claim_batch(
            &self,
            _run_id: i32,
            _instance_id: &str,
        ) -> Result<Option<BatchClaim>, StoreError> {
            Ok(self.inner.lock().expect("poison").next_claim.take())
        }

        async fn submit_batch_results(
            &self,
            batch_id: i64,
            results: &BatchResults,
            _eval_time_ms: f64,
        ) -> Result<(), StoreError> {
            self.inner
                .lock()
                .expect("poison")
                .submitted
                .push((batch_id, results.clone()));
            Ok(())
        }

        async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError> {
            self.inner
                .lock()
                .expect("poison")
                .failed
                .push((batch_id, last_error.to_string()));
            Ok(())
        }

        async fn fetch_completed_batches_since(
            &self,
            _run_id: i32,
            _last_batch_id: Option<i64>,
            _limit: usize,
        ) -> Result<Vec<CompletedBatch>, StoreError> {
            Ok(Vec::new())
        }
    }

    struct OkEvaluator;

    impl Evaluator for OkEvaluator {
        fn eval_point(&self, point: &serde_json::Value) -> Result<f64, EvalError> {
            point
                .as_f64()
                .ok_or_else(|| EvalError::new("expected numeric point"))
        }
    }

    struct FailingEvaluator;

    impl Evaluator for FailingEvaluator {
        fn eval_point(&self, _point: &serde_json::Value) -> Result<f64, EvalError> {
            Err(EvalError::new("mock failure"))
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
        let queue = TestQueue::default();
        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            OkEvaluator,
            queue,
            WorkerRunnerConfig::default(),
        );

        let tick = runner.tick().await.expect("tick");

        assert!(tick.claimed_batch_id.is_none());
        assert_eq!(tick.processed_samples, 0);
    }

    #[tokio::test]
    async fn tick_submits_results_on_success() {
        let queue = TestQueue::default();
        queue.inner.lock().expect("poison").next_claim = Some(BatchClaim {
            batch_id: 42,
            batch: sample_batch(),
        });

        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            OkEvaluator,
            queue.clone(),
            WorkerRunnerConfig::default(),
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
        let queue = TestQueue::default();
        queue.inner.lock().expect("poison").next_claim = Some(BatchClaim {
            batch_id: 99,
            batch: sample_batch(),
        });

        let mut runner = WorkerRunner::new(
            1,
            "worker-1",
            FailingEvaluator,
            queue.clone(),
            WorkerRunnerConfig::default(),
        );

        let err = runner.tick().await.expect_err("expected eval error");
        let state = queue.inner.lock().expect("poison").clone();

        assert!(matches!(err, WorkerRunnerError::Eval(_)));
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.failed[0].0, 99);
        assert!(state.submitted.is_empty());
    }
}
