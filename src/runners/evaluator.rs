//! Evaluator worker runner orchestration.

use crate::core::{
    BatchTransformConfig, EngineError, EvalError, EvaluatorIdleProfileMetrics,
    EvaluatorPerformanceMetrics, EvaluatorPerformanceSnapshot, EvaluatorWorkerStore, StoreError,
};
use crate::evaluation::{Batch, BatchResult, EvalBatchOptions, Evaluator, Materializer};
use crate::runners::rolling_metric::RollingMetric;
use crate::runners::stage_context::resolve_stage_context;
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use std::{time::Duration, time::Instant};
use thiserror::Error;
use tracing::info;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluatorRunnerParams {
    pub performance_snapshot_interval_ms: u64,
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

pub struct EvaluatorRunner<S> {
    run_id: i32,
    node_name: String,
    node_uuid: String,
    evaluator: Box<dyn Evaluator>,
    domain: Domain,
    current_task_id: Option<i64>,
    materializer: Option<Box<dyn Materializer>>,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    batches_completed_total: i64,
    samples_evaluated_total: i64,
    rolling: EvaluatorRollingAverages,
    store: S,
    current_batch_transforms: Vec<Box<dyn crate::evaluation::BatchTransform>>,
}

struct TaskRuntimeContext {
    materializer: Box<dyn Materializer>,
    batch_transforms: Vec<Box<dyn crate::evaluation::BatchTransform>>,
}

#[derive(Debug, Clone, Serialize, Default)]
struct EvaluatorRollingAverages {
    total_ms_per_sample: RollingMetric,
    evaluate_ms_per_sample: RollingMetric,
    materialization_ms_per_sample: RollingMetric,
    idle_ratio: RollingMetric,
}

impl<S> EvaluatorRunner<S>
where
    S: EvaluatorWorkerStore,
{
    pub fn new(
        store: S,
        run_id: i32,
        node_name: impl Into<String>,
        node_uuid: impl Into<String>,
        evaluator: Box<dyn Evaluator>,
        domain: Domain,
        params: EvaluatorRunnerParams,
    ) -> Self {
        Self {
            run_id,
            node_name: node_name.into(),
            node_uuid: node_uuid.into(),
            evaluator,
            domain,
            current_task_id: None,
            materializer: None,
            performance_snapshot_interval: Duration::from_millis(
                params.performance_snapshot_interval_ms,
            ),
            last_snapshot_at: Instant::now(),
            batches_completed_total: 0,
            samples_evaluated_total: 0,
            rolling: EvaluatorRollingAverages::default(),
            store,
            current_batch_transforms: Vec::new(),
        }
    }

    fn build_batch_transforms(
        configs: &[BatchTransformConfig],
        domain: &Domain,
    ) -> Result<Vec<Box<dyn crate::evaluation::BatchTransform>>, EvaluatorRunnerError> {
        configs
            .iter()
            .map(|config| {
                let transform = config.build().map_err(|err| {
                    EvaluatorRunnerError::Store(StoreError::store(format!(
                        "failed to build batch transform: {err}"
                    )))
                })?;
                transform.validate_domain(domain).map_err(|err| {
                    EvaluatorRunnerError::Store(StoreError::store(format!(
                        "failed to validate batch transform domain: {err}"
                    )))
                })?;
                Ok(transform)
            })
            .collect()
    }

    async fn ensure_task_context(&mut self, task_id: i64) -> Result<(), EvaluatorRunnerError> {
        if self.current_task_id == Some(task_id) {
            return Ok(());
        }

        let TaskRuntimeContext {
            materializer,
            batch_transforms,
        } = self.load_task_context(task_id).await?;

        self.current_task_id = Some(task_id);
        self.materializer = Some(materializer);
        self.current_batch_transforms = batch_transforms;
        Ok(())
    }

    async fn load_task_context(
        &self,
        task_id: i64,
    ) -> Result<TaskRuntimeContext, EvaluatorRunnerError> {
        let task = self
            .store
            .load_run_task(task_id)
            .await
            .map_err(EvaluatorRunnerError::Store)?
            .ok_or_else(|| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "claimed batch references missing task {}",
                    task_id
                )))
            })?;
        let resolved =
            resolve_stage_context(&self.store, self.run_id, &task, task.sequence_nr, None)
                .await
                .map_err(EvaluatorRunnerError::Store)?;
        let batch_transforms =
            Self::build_batch_transforms(&resolved.batch_transforms, &self.domain)?;
        let materializer = resolved
            .sampler_config
            .build_materializer(resolved.handoff.as_ref().map(|handoff| handoff.as_ref()))
            .map_err(|err| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "failed to build materializer for task {}: {err}",
                    task_id
                )))
            })?;
        materializer.validate_domain(&self.domain).map_err(|err| {
            EvaluatorRunnerError::Store(StoreError::store(format!(
                "failed to validate materializer domain for task {}: {err}",
                task_id
            )))
        })?;
        Ok(TaskRuntimeContext {
            materializer,
            batch_transforms,
        })
    }

    async fn fail_claimed_batch(
        &mut self,
        batch_id: i64,
        err: &str,
    ) -> Result<(), EvaluatorRunnerError> {
        self.store
            .fail_batch(batch_id, err)
            .await
            .map_err(EvaluatorRunnerError::Store)
    }

    async fn fail_tick<T>(
        &mut self,
        loop_started: Instant,
        batch_id: i64,
        compute_time_ms: f64,
        err: impl Into<EvaluatorRunnerError>,
    ) -> Result<T, EvaluatorRunnerError> {
        let err = err.into();
        self.fail_claimed_batch(batch_id, &err.to_string()).await?;
        self.observe_idle_ratio(loop_started, compute_time_ms);
        self.flush_performance_snapshot_if_due(false).await?;
        Err(err)
    }

    pub async fn tick(&mut self) -> Result<(), EvaluatorRunnerError> {
        let loop_started = Instant::now();
        let claimed = self
            .store
            .claim_batch(self.run_id, &self.node_uuid)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        let Some(claimed) = claimed else {
            self.observe_idle_ratio(loop_started, 0.0);
            self.flush_performance_snapshot_if_due(false).await?;
            return Ok(());
        };

        self.ensure_task_context(claimed.task_id).await?;

        let materialization_started = Instant::now();
        let materializer = self.materializer.as_mut().ok_or_else(|| {
            EvaluatorRunnerError::Store(StoreError::store(format!(
                "evaluator task {} has no materializer",
                claimed.task_id
            )))
        })?;
        let materialized = materializer.materialize_batch(&claimed.latent_batch);
        let materialization_time_ms = materialization_started.elapsed().as_secs_f64() * 1000.0;
        let materialized_batch = match materialized {
            Ok(batch) => batch,
            Err(err) => {
                return self
                    .fail_tick(
                        loop_started,
                        claimed.batch_id,
                        materialization_time_ms,
                        EvaluatorRunnerError::Engine(err),
                    )
                    .await;
            }
        };
        let mut transformed_batch = materialized_batch;
        for transform in &self.current_batch_transforms {
            transformed_batch = match transform.apply(transformed_batch) {
                Ok(batch) => batch,
                Err(err) => {
                    return self
                        .fail_tick(
                            loop_started,
                            claimed.batch_id,
                            materialization_time_ms,
                            EvaluatorRunnerError::Engine(err),
                        )
                        .await;
                }
            };
        }
        let started = Instant::now();
        match self.evaluator.eval_batch(
            &transformed_batch,
            &claimed.latent_batch.observable,
            EvalBatchOptions {
                require_training_values: claimed.requires_training_values,
            },
        ) {
            Ok(result) => {
                let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;
                let total_time_ms = materialization_time_ms + eval_time_ms;
                self.submit_result(
                    claimed.batch_id,
                    claimed.requires_training_values,
                    &transformed_batch,
                    result,
                    total_time_ms,
                    materialization_time_ms,
                    eval_time_ms,
                )
                .await?;
                self.observe_idle_ratio(loop_started, total_time_ms);
                Ok(())
            }
            Err(err) => {
                let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;
                let total_time_ms = materialization_time_ms + eval_time_ms;
                self.fail_tick(
                    loop_started,
                    claimed.batch_id,
                    total_time_ms,
                    EvaluatorRunnerError::Eval(err),
                )
                .await
            }
        }
    }

    async fn submit_result(
        &mut self,
        batch_id: i64,
        requires_training_values: bool,
        batch: &Batch,
        result: BatchResult,
        total_time_ms: f64,
        materialization_time_ms: f64,
        eval_time_ms: f64,
    ) -> Result<(), EvaluatorRunnerError> {
        if requires_training_values && result.values.is_none() {
            let err = EngineError::engine(format!(
                "result is missing training values for training batch {}",
                batch_id
            ));
            self.fail_claimed_batch(batch_id, &err.to_string()).await?;
            return Err(EvaluatorRunnerError::Engine(err));
        }
        if !result.matches_batch(batch) {
            let err = EngineError::engine(format!(
                "result length mismatch for batch {}: expected {}, got {}",
                batch_id,
                batch.size(),
                result.len()
            ));
            self.fail_claimed_batch(batch_id, &err.to_string()).await?;
            return Err(EvaluatorRunnerError::Engine(err));
        }

        match self
            .store
            .submit_batch_results(batch_id, &self.node_uuid, &result, total_time_ms)
            .await
        {
            Ok(()) => {}
            Err(err) if err.is_batch_ownership_lost() => {
                info!(
                    run_id = self.run_id,
                    batch_id,
                    node_name = %self.node_name,
                    node_uuid = %self.node_uuid,
                    error = %err,
                    "dropping stale evaluator result after batch ownership was lost"
                );
                self.flush_performance_snapshot_if_due(false).await?;
                return Ok(());
            }
            Err(err) => return Err(EvaluatorRunnerError::Store(err)),
        }

        let processed_samples = batch.size();
        self.observe_eval_batch(
            processed_samples,
            total_time_ms,
            materialization_time_ms,
            eval_time_ms,
        );
        self.flush_performance_snapshot_if_due(false).await?;

        Ok(())
    }

    fn observe_eval_batch(
        &mut self,
        samples: usize,
        total_time_ms: f64,
        materialization_time_ms: f64,
        eval_time_ms: f64,
    ) {
        self.batches_completed_total += 1;
        self.samples_evaluated_total += samples as i64;
        if samples > 0 {
            let samples = samples as f64;
            if total_time_ms.is_finite() && total_time_ms >= 0.0 {
                self.rolling
                    .total_ms_per_sample
                    .observe(total_time_ms / samples);
            }
            if materialization_time_ms.is_finite() && materialization_time_ms >= 0.0 {
                self.rolling
                    .materialization_ms_per_sample
                    .observe(materialization_time_ms / samples);
            }
            if eval_time_ms.is_finite() && eval_time_ms >= 0.0 {
                self.rolling
                    .evaluate_ms_per_sample
                    .observe(eval_time_ms / samples);
            }
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
            node_name: self.node_name.clone(),
            metrics: EvaluatorPerformanceMetrics {
                batches_completed: self.batches_completed_total,
                samples_evaluated: self.samples_evaluated_total,
                avg_time_per_sample_ms: self.rolling.total_ms_per_sample.value().unwrap_or(0.0),
                std_time_per_sample_ms: self.rolling.total_ms_per_sample.std_dev(),
                avg_evaluate_time_per_sample_ms: self
                    .rolling
                    .evaluate_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_evaluate_time_per_sample_ms: self.rolling.evaluate_ms_per_sample.std_dev(),
                avg_materialization_time_per_sample_ms: self
                    .rolling
                    .materialization_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_materialization_time_per_sample_ms: self
                    .rolling
                    .materialization_ms_per_sample
                    .std_dev(),
                idle_profile: Some(EvaluatorIdleProfileMetrics {
                    idle_ratio: self.rolling.idle_ratio.value().unwrap_or(0.0),
                }),
            },
            //engine_diagnostics: self.evaluator.get_diagnostics(),
        };

        self.store
            .record_evaluator_performance_snapshot(&snapshot)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        self.last_snapshot_at = Instant::now();
        Ok(())
    }
}
