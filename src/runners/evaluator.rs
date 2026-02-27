//! Evaluator worker runner orchestration.

use crate::batch::{BatchResult, PointSpec};
use crate::core::{EvaluatorPerformanceSnapshot, StoreError, WorkQueueStore};
use crate::engines::observable::ObservableFactory;
use crate::engines::{EngineError, EvalError, Evaluator, Parametrization};
use crate::runners::sample_time_stats::SampleTimeStats;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, time::Duration, time::Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EvaluatorRunnerParams {
    pub min_loop_time_ms: u64,
    pub performance_snapshot_interval_ms: u64,
}

impl Default for EvaluatorRunnerParams {
    fn default() -> Self {
        Self {
            min_loop_time_ms: 0,
            performance_snapshot_interval_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EvaluatorRunnerTick {
    pub claimed_batch_id: Option<i64>,
    pub processed_samples: usize,
    pub eval_time_ms: f64,
}

#[derive(Debug)]
pub enum EvaluatorRunnerError {
    Engine(EngineError),
    Eval(EvalError),
    Store(StoreError),
}

impl fmt::Display for EvaluatorRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    parametrization: Box<dyn Parametrization>,
    observable_factory: ObservableFactory,
    point_spec: PointSpec,
    performance_snapshot_interval: Duration,
    perf_window_started_at: Instant,
    perf_window_started_ts: chrono::DateTime<Utc>,
    perf_eval: SampleTimeStats,
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
        parametrization: Box<dyn Parametrization>,
        observable_factory: ObservableFactory,
        point_spec: PointSpec,
        performance_snapshot_interval: Duration,
        work_queue: WQ,
    ) -> Self {
        let now_ts = Utc::now();
        let now_instant = Instant::now();
        Self {
            run_id,
            worker_id: worker_id.into(),
            evaluator,
            parametrization,
            observable_factory,
            point_spec,
            performance_snapshot_interval,
            perf_window_started_at: now_instant,
            perf_window_started_ts: now_ts,
            perf_eval: SampleTimeStats::default(),
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
            self.flush_performance_snapshot_if_due(false).await?;
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
            self.flush_performance_snapshot_if_due(false).await?;
            return Err(EvaluatorRunnerError::Engine(err));
        }

        let transformed_batch = match self.parametrization.transform_batch(&claimed.batch) {
            Ok(batch) => batch,
            Err(err) => {
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await
                    .map_err(EvaluatorRunnerError::Store)?;
                self.flush_performance_snapshot_if_due(false).await?;
                return Err(EvaluatorRunnerError::Engine(err));
            }
        };
        if let Err(err) = transformed_batch.validate_point_spec(&self.point_spec) {
            let err = EngineError::engine(format!("invalid transformed batch point shape: {err}"));
            self.work_queue
                .fail_batch(claimed.batch_id, &err.to_string())
                .await
                .map_err(EvaluatorRunnerError::Store)?;
            self.flush_performance_snapshot_if_due(false).await?;
            return Err(EvaluatorRunnerError::Engine(err));
        }

        let started = Instant::now();
        match self
            .evaluator
            .eval_batch(&transformed_batch, &self.observable_factory)
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
                self.flush_performance_snapshot_if_due(false).await?;
                Err(EvaluatorRunnerError::Eval(err))
            }
        }
    }

    async fn submit_result(
        &mut self,
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

        self.observe_eval_batch(result.len(), eval_time_ms);
        self.flush_performance_snapshot_if_due(false).await?;

        Ok(EvaluatorRunnerTick {
            claimed_batch_id: Some(batch_id),
            processed_samples: result.len(),
            eval_time_ms,
        })
    }

    fn observe_eval_batch(&mut self, samples: usize, eval_time_ms: f64) {
        self.perf_eval.observe(samples, eval_time_ms);
    }

    async fn flush_performance_snapshot_if_due(
        &mut self,
        force: bool,
    ) -> Result<(), EvaluatorRunnerError> {
        if !self.perf_eval.has_data() {
            return Ok(());
        }

        let due = if self.performance_snapshot_interval.is_zero() {
            true
        } else {
            self.perf_window_started_at.elapsed() >= self.performance_snapshot_interval
        };
        if !force && !due {
            return Ok(());
        }

        let snapshot = EvaluatorPerformanceSnapshot {
            run_id: self.run_id,
            worker_id: self.worker_id.clone(),
            window_start: self.perf_window_started_ts,
            window_end: Utc::now(),
            batches_completed: self.perf_eval.batches(),
            samples_evaluated: self.perf_eval.samples(),
            avg_time_per_sample_ms: self.perf_eval.mean(),
            std_time_per_sample_ms: self.perf_eval.std(),
            diagnostics: self.evaluator.get_diagnostics(),
        };

        self.work_queue
            .record_evaluator_performance_snapshot(&snapshot)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        self.perf_window_started_at = Instant::now();
        self.perf_window_started_ts = Utc::now();
        self.perf_eval = SampleTimeStats::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{Batch, BatchResult};
    use crate::core::BatchClaim;
    use crate::engines::{
        BuildError, EvalError, ObservableImplementation, ParametrizationFactory,
        ParametrizationImplementation,
    };
    use crate::runners::test_support::MockWorkQueue;
    use serde_json::json;

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
            &mut self,
            batch: &Batch,
            _observable_factory: &ObservableFactory,
        ) -> Result<BatchResult, EvalError> {
            let values = batch
                .continuous()
                .column(0)
                .iter()
                .copied()
                .collect::<Vec<f64>>();
            let batch_observable = json!({
                "count": values.len() as i64,
                "sum_weight": values
                    .iter()
                    .zip(batch.weights().iter())
                    .map(|(value, weight)| value * weight)
                    .sum::<f64>(),
                "sum_abs": values.iter().map(|v| v.abs()).sum::<f64>(),
                "sum_sq": values.iter().map(|v| v * v).sum::<f64>(),
            });
            Ok(BatchResult::new(values, batch_observable))
        }

        fn supports_observable(
            &self,
            _observable_factory: &crate::engines::observable::ObservableFactory,
        ) -> bool {
            true
        }
    }

    struct FailingEvaluator;

    impl Evaluator for FailingEvaluator {
        fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
            Ok(())
        }

        fn eval_batch(
            &mut self,
            _batch: &Batch,
            _observable_factory: &ObservableFactory,
        ) -> Result<BatchResult, EvalError> {
            Err(EvalError::eval("mock failure"))
        }

        fn supports_observable(
            &self,
            _observable_factory: &crate::engines::observable::ObservableFactory,
        ) -> bool {
            false
        }
    }

    fn sample_batch() -> Batch {
        Batch::from_flat_data(2, 1, 0, vec![1.0, 2.0], vec![]).expect("sample batch")
    }

    fn no_parametrization() -> Box<dyn Parametrization> {
        ParametrizationFactory::new(ParametrizationImplementation::None, json!({}))
            .build()
            .expect("no parametrization")
    }

    #[tokio::test]
    async fn tick_returns_empty_when_no_batch_claimed() {
        let queue = MockWorkQueue::default();
        let mut runner = EvaluatorRunner::new(
            1,
            "worker-1",
            Box::new(OkEvaluator),
            no_parametrization(),
            ObservableFactory::new(ObservableImplementation::Scalar, json!({})),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            Duration::from_millis(0),
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
            no_parametrization(),
            ObservableFactory::new(ObservableImplementation::Scalar, json!({})),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            Duration::from_millis(0),
            queue.clone(),
        );

        let tick = runner.tick().await.expect("tick");
        let state = queue.inner.lock().expect("poison").clone();

        assert_eq!(tick.claimed_batch_id, Some(42));
        assert_eq!(tick.processed_samples, 2);
        assert_eq!(state.submitted.len(), 1);
        assert_eq!(state.submitted[0].0, 42);
        assert_eq!(state.submitted[0].1.values, vec![1.0, 2.0]);
        assert_eq!(state.evaluator_perf_snapshots.len(), 1);
        assert_eq!(state.evaluator_perf_snapshots[0].batches_completed, 1);
        assert_eq!(state.evaluator_perf_snapshots[0].samples_evaluated, 2);
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
            no_parametrization(),
            ObservableFactory::new(ObservableImplementation::Scalar, json!({})),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            Duration::from_millis(0),
            queue.clone(),
        );

        let err = runner.tick().await.expect_err("expected eval error");
        let state = queue.inner.lock().expect("poison").clone();

        assert!(matches!(err, EvaluatorRunnerError::Eval(_)));
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.failed[0].0, 99);
        assert!(state.evaluator_perf_snapshots.is_empty());
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
                &mut self,
                _batch: &Batch,
                _observable_factory: &ObservableFactory,
            ) -> Result<BatchResult, EvalError> {
                Ok(BatchResult::new(vec![1.0], json!({})))
            }
            fn supports_observable(
                &self,
                _observable_factory: &crate::engines::observable::ObservableFactory,
            ) -> bool {
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
            no_parametrization(),
            ObservableFactory::new(ObservableImplementation::Scalar, json!({})),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            Duration::from_millis(0),
            queue.clone(),
        );

        let err = runner.tick().await.expect_err("expected mismatch");
        let state = queue.inner.lock().expect("poison").clone();
        assert!(matches!(err, EvaluatorRunnerError::Engine(_)));
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.failed[0].0, 7);
        assert!(state.evaluator_perf_snapshots.is_empty());
    }
}
