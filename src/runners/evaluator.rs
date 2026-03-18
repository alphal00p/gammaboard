//! Evaluator worker runner orchestration.

use crate::core::{EngineError, EvalError, ParametrizationState};
use crate::core::{
    EvaluatorIdleProfileMetrics, EvaluatorPerformanceMetrics, EvaluatorPerformanceSnapshot,
    ParametrizationVersionStore, StoreError, WorkQueueStore,
};
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, Parametrization, PointSpec,
};
use crate::runners::rolling_metric::RollingMetric;
use crate::sampling::ParametrizationBuildContext;
use serde::{Deserialize, Serialize};
use std::{time::Duration, time::Instant};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluatorRunnerParams {
    pub min_loop_time_ms: u64,
    pub performance_snapshot_interval_ms: u64,
}

#[derive(Debug, Clone)]
pub struct EvaluatorRunnerTick {
    pub claimed_batch_id: Option<i64>,
    pub processed_samples: usize,
    pub eval_time_ms: f64,
}

#[derive(Debug, Error)]
pub enum EvaluatorRunnerError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Eval(EvalError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub struct EvaluatorRunner<WQ, PS> {
    run_id: i32,
    node_id: String,
    evaluator: Box<dyn Evaluator>,
    point_spec: PointSpec,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    batches_completed_total: i64,
    samples_evaluated_total: i64,
    rolling: EvaluatorRollingAverages,
    work_queue: WQ,
    parametrization_states: PS,
    current_parametrization_version: Option<i64>,
    current_parametrization: Option<Box<dyn Parametrization>>,
}

#[derive(Debug, Clone, Serialize, Default)]
struct EvaluatorRollingAverages {
    eval_ms_per_sample: RollingMetric,
    idle_ratio: RollingMetric,
}

impl<WQ, PS> EvaluatorRunner<WQ, PS>
where
    WQ: WorkQueueStore,
    PS: ParametrizationVersionStore,
{
    pub fn new(
        run_id: i32,
        node_id: impl Into<String>,
        evaluator: Box<dyn Evaluator>,
        point_spec: PointSpec,
        performance_snapshot_interval: Duration,
        work_queue: WQ,
        parametrization_states: PS,
    ) -> Self {
        let now_instant = Instant::now();
        Self {
            run_id,
            node_id: node_id.into(),
            evaluator,
            point_spec,
            performance_snapshot_interval,
            last_snapshot_at: now_instant,
            batches_completed_total: 0,
            samples_evaluated_total: 0,
            rolling: EvaluatorRollingAverages::default(),
            work_queue,
            parametrization_states,
            current_parametrization_version: None,
            current_parametrization: None,
        }
    }

    pub async fn tick(&mut self) -> Result<EvaluatorRunnerTick, EvaluatorRunnerError> {
        let loop_started = Instant::now();
        let claimed = self
            .work_queue
            .claim_batch(self.run_id, &self.node_id)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        let Some(claimed) = claimed else {
            self.observe_idle_ratio(loop_started, 0.0);
            self.flush_performance_snapshot_if_due(false).await?;
            return Ok(EvaluatorRunnerTick {
                claimed_batch_id: None,
                processed_samples: 0,
                eval_time_ms: 0.0,
            });
        };

        if let Err(err) = self
            .ensure_parametrization(claimed.latent_batch.parametrization_state_version)
            .await
        {
            self.work_queue
                .fail_batch(claimed.batch_id, &err.to_string())
                .await
                .map_err(EvaluatorRunnerError::Store)?;
            self.observe_idle_ratio(loop_started, 0.0);
            self.flush_performance_snapshot_if_due(false).await?;
            return Err(EvaluatorRunnerError::Store(err));
        }

        let materialized = match self.current_parametrization.as_mut() {
            Some(parametrization) => parametrization.materialize_batch(&claimed.latent_batch),
            None => Err(EngineError::engine(
                "parametrization runtime missing after successful version load",
            )),
        };
        let transformed_batch = match materialized {
            Ok(batch) => batch,
            Err(err) => {
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await
                    .map_err(EvaluatorRunnerError::Store)?;
                self.observe_idle_ratio(loop_started, 0.0);
                self.flush_performance_snapshot_if_due(false).await?;
                return Err(EvaluatorRunnerError::Engine(err));
            }
        };
        if let Err(err) = transformed_batch.validate_point_spec(&self.point_spec) {
            let err = EngineError::engine(format!("invalid materialized batch point shape: {err}"));
            self.work_queue
                .fail_batch(claimed.batch_id, &err.to_string())
                .await
                .map_err(EvaluatorRunnerError::Store)?;
            self.observe_idle_ratio(loop_started, 0.0);
            self.flush_performance_snapshot_if_due(false).await?;
            return Err(EvaluatorRunnerError::Engine(err));
        }

        let started = Instant::now();
        match self.evaluator.eval_batch(
            &transformed_batch,
            &claimed.latent_batch.observable,
            EvalBatchOptions {
                require_training_values: claimed.requires_training,
            },
        ) {
            Ok(result) => {
                let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;
                let tick = self
                    .submit_result(
                        claimed.batch_id,
                        &transformed_batch,
                        claimed.requires_training,
                        result,
                        eval_time_ms,
                    )
                    .await?;
                self.observe_idle_ratio(loop_started, eval_time_ms);
                Ok(tick)
            }
            Err(err) => {
                let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;
                self.work_queue
                    .fail_batch(claimed.batch_id, &err.to_string())
                    .await
                    .map_err(EvaluatorRunnerError::Store)?;
                self.observe_idle_ratio(loop_started, eval_time_ms);
                self.flush_performance_snapshot_if_due(false).await?;
                Err(EvaluatorRunnerError::Eval(err))
            }
        }
    }

    async fn ensure_parametrization(&mut self, version: i64) -> Result<(), StoreError> {
        if self.current_parametrization_version == Some(version) {
            return Ok(());
        }
        let Some(state): Option<ParametrizationState> = self
            .parametrization_states
            .load_parametrization_version(self.run_id, version)
            .await?
        else {
            return Err(StoreError::store(format!(
                "missing parametrization state for run {} version {}",
                self.run_id, version
            )));
        };
        let parametrization = state
            .config
            .build(ParametrizationBuildContext {
                sampler_aggregator_snapshot: None,
                parametrization_snapshot: Some(&state.snapshot),
            })
            .map_err(|err| StoreError::store(format!("failed to build parametrization: {err}")))?;
        parametrization
            .validate_point_spec(&self.point_spec)
            .map_err(|err| {
                StoreError::store(format!(
                    "incompatible parametrization for point_spec on run {} version {}: {}",
                    self.run_id, version, err
                ))
            })?;
        self.current_parametrization_version = Some(version);
        self.current_parametrization = Some(parametrization);
        Ok(())
    }

    async fn submit_result(
        &mut self,
        batch_id: i64,
        batch: &Batch,
        requires_training: bool,
        result: BatchResult,
        eval_time_ms: f64,
    ) -> Result<EvaluatorRunnerTick, EvaluatorRunnerError> {
        if requires_training && result.values.is_none() {
            let err = EngineError::engine(format!(
                "result is missing training values for training batch {}",
                batch_id
            ));
            self.work_queue
                .fail_batch(batch_id, &err.to_string())
                .await
                .map_err(EvaluatorRunnerError::Store)?;
            return Err(EvaluatorRunnerError::Engine(err));
        }
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

        self.work_queue
            .submit_batch_results(batch_id, &result, eval_time_ms)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        let processed_samples = batch.size();
        self.observe_eval_batch(processed_samples, eval_time_ms);
        self.flush_performance_snapshot_if_due(false).await?;

        Ok(EvaluatorRunnerTick {
            claimed_batch_id: Some(batch_id),
            processed_samples,
            eval_time_ms,
        })
    }

    fn observe_eval_batch(&mut self, samples: usize, eval_time_ms: f64) {
        self.batches_completed_total += 1;
        self.samples_evaluated_total += samples as i64;
        if samples > 0 && eval_time_ms.is_finite() && eval_time_ms >= 0.0 {
            self.rolling
                .eval_ms_per_sample
                .observe(eval_time_ms / samples as f64);
        }
    }

    fn observe_idle_ratio(&mut self, loop_started: Instant, compute_time_ms: f64) {
        let elapsed_ms = loop_started.elapsed().as_secs_f64() * 1000.0;
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return;
        }
        let compute = compute_time_ms.max(0.0);
        let idle_ratio = ((elapsed_ms - compute).max(0.0) / elapsed_ms).clamp(0.0, 1.0);
        self.rolling.idle_ratio.observe(idle_ratio);
    }

    async fn flush_performance_snapshot_if_due(
        &mut self,
        force: bool,
    ) -> Result<(), EvaluatorRunnerError> {
        if self.samples_evaluated_total <= 0 {
            return Ok(());
        }

        let due = if self.performance_snapshot_interval.is_zero() {
            true
        } else {
            self.last_snapshot_at.elapsed() >= self.performance_snapshot_interval
        };
        if !force && !due {
            return Ok(());
        }

        let snapshot = EvaluatorPerformanceSnapshot {
            run_id: self.run_id,
            node_id: self.node_id.clone(),
            metrics: EvaluatorPerformanceMetrics {
                batches_completed: self.batches_completed_total,
                samples_evaluated: self.samples_evaluated_total,
                avg_time_per_sample_ms: self.rolling.eval_ms_per_sample.value().unwrap_or(0.0),
                std_time_per_sample_ms: self.rolling.eval_ms_per_sample.std_dev(),
                idle_profile: Some(EvaluatorIdleProfileMetrics {
                    idle_ratio: self.rolling.idle_ratio.value().unwrap_or(0.0),
                }),
            },
            //engine_diagnostics: self.evaluator.get_diagnostics(),
        };

        self.work_queue
            .record_evaluator_performance_snapshot(&snapshot)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        self.last_snapshot_at = Instant::now();
        Ok(())
    }
}
