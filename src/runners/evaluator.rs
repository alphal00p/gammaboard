//! Evaluator worker runner orchestration.

use crate::core::{
    BatchTransformConfig, EngineError, EvalError, EvaluatorIdleProfileMetrics,
    EvaluatorPerformanceMetrics, EvaluatorPerformanceSnapshot, EvaluatorWorkerStore, RunTask,
    SourceRefSpec, StoreError,
};
use crate::evaluation::{Batch, BatchResult, EvalBatchOptions, Evaluator, Materializer};
use crate::runners::rolling_metric::RollingMetric;
use crate::sampling::StageHandoffOwned;
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use std::{time::Duration, time::Instant};
use thiserror::Error;
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
        let task = self.load_task(task_id).await?;
        let source_snapshot = self.resolve_source_snapshot(&task).await?;
        let base_stage_snapshot = self
            .store
            .load_latest_stage_snapshot_before_sequence(self.run_id, task.sequence_nr)
            .await
            .map_err(EvaluatorRunnerError::Store)?;
        let batch_transforms = self.build_task_batch_transforms(
            &task,
            source_snapshot.as_ref(),
            base_stage_snapshot.as_ref(),
        )?;
        let sampler_config = self.resolve_effective_sampler_config(
            &task,
            source_snapshot.as_ref(),
            base_stage_snapshot.as_ref(),
        )?;
        let handoff = self
            .resolve_materializer_handoff(
                &task,
                &sampler_config,
                source_snapshot,
                base_stage_snapshot,
            )
            .await?;
        let materializer = sampler_config
            .build_materializer(handoff.as_ref().map(StageHandoffOwned::as_ref))
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

    async fn load_task(&self, task_id: i64) -> Result<RunTask, EvaluatorRunnerError> {
        self.store
            .load_run_task(task_id)
            .await
            .map_err(EvaluatorRunnerError::Store)?
            .ok_or_else(|| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "claimed batch references missing task {}",
                    task_id
                )))
            })
    }

    fn resolve_effective_sampler_config(
        &self,
        task: &RunTask,
        source_snapshot: Option<&crate::core::RunStageSnapshot>,
        base_stage_snapshot: Option<&crate::core::RunStageSnapshot>,
    ) -> Result<crate::core::SamplerAggregatorConfig, EvaluatorRunnerError> {
        task.task
            .sampler_config()
            .or_else(|| task.task.sample_sampler_config())
            .or_else(|| source_snapshot.map(|snapshot| snapshot.sampler_aggregator.clone()))
            .or_else(|| base_stage_snapshot.map(|snapshot| snapshot.sampler_aggregator.clone()))
            .ok_or_else(|| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "run {} task {} has no sampler configuration",
                    self.run_id, task.id
                )))
            })
    }

    fn build_task_batch_transforms(
        &self,
        task: &RunTask,
        source_snapshot: Option<&crate::core::RunStageSnapshot>,
        base_stage_snapshot: Option<&crate::core::RunStageSnapshot>,
    ) -> Result<Vec<Box<dyn crate::evaluation::BatchTransform>>, EvaluatorRunnerError> {
        let configs = task
            .task
            .batch_transforms_config()
            .or_else(|| source_snapshot.map(|snapshot| snapshot.batch_transforms.clone()))
            .or_else(|| base_stage_snapshot.map(|snapshot| snapshot.batch_transforms.clone()))
            .unwrap_or_default();
        Self::build_batch_transforms(&configs, &self.domain)
    }

    async fn resolve_source_snapshot(
        &self,
        task: &RunTask,
    ) -> Result<Option<crate::core::RunStageSnapshot>, EvaluatorRunnerError> {
        match task.task.sample_sampler_source() {
            Some(SourceRefSpec::Latest) => self
                .store
                .load_latest_stage_snapshot_before_sequence(self.run_id, task.sequence_nr)
                .await
                .map_err(EvaluatorRunnerError::Store),
            Some(SourceRefSpec::FromName(source_task_name)) => {
                let source_task = self
                    .store
                    .list_run_tasks(self.run_id)
                    .await
                    .map_err(EvaluatorRunnerError::Store)?
                    .into_iter()
                    .find(|candidate| candidate.name == source_task_name)
                    .ok_or_else(|| {
                        EvaluatorRunnerError::Store(StoreError::store(format!(
                            "task {} references source task '{}' but no such task exists in run {}",
                            task.id, source_task_name, self.run_id
                        )))
                    })?;
                if source_task.sequence_nr >= task.sequence_nr {
                    return Err(EvaluatorRunnerError::Store(StoreError::store(format!(
                        "task {} references source task '{}' which is not prior in sequence",
                        task.id, source_task_name
                    ))));
                }
                self.store
                    .load_latest_stage_snapshot_before_sequence(
                        self.run_id,
                        source_task.sequence_nr + 1,
                    )
                    .await
                    .map_err(EvaluatorRunnerError::Store)
            }
            None => Ok(None),
        }
    }

    async fn resolve_materializer_handoff(
        &self,
        task: &RunTask,
        sampler_config: &crate::core::SamplerAggregatorConfig,
        source_snapshot: Option<crate::core::RunStageSnapshot>,
        base_stage_snapshot: Option<crate::core::RunStageSnapshot>,
    ) -> Result<Option<StageHandoffOwned>, EvaluatorRunnerError> {
        let handoff_snapshot = if let Some(snapshot) = source_snapshot {
            Some(snapshot)
        } else {
            match sampler_config {
                crate::core::SamplerAggregatorConfig::HavanaInference { params } => {
                    match &params.source {
                        crate::sampling::HavanaInferenceSource::Snapshot { snapshot_id } => self
                            .store
                            .load_stage_snapshot(*snapshot_id)
                            .await
                            .map_err(EvaluatorRunnerError::Store)?,
                        crate::sampling::HavanaInferenceSource::LatestTrainingSamplerAggregator => {
                            self.find_latest_havana_snapshot_before_sequence(task.sequence_nr)
                                .await?
                        }
                    }
                }
                _ => base_stage_snapshot,
            }
        };
        let handoff_snapshot = match (sampler_config, handoff_snapshot) {
            (crate::core::SamplerAggregatorConfig::HavanaInference { .. }, Some(snapshot))
                if !snapshot.sampler_snapshot.contains_havana_grid() =>
            {
                self.find_latest_havana_snapshot_before_sequence(
                    snapshot.sequence_nr.unwrap_or(task.sequence_nr),
                )
                .await?
            }
            (_, snapshot) => snapshot,
        };

        Ok(handoff_snapshot.map(|snapshot| StageHandoffOwned {
            sampler_snapshot: Some(snapshot.sampler_snapshot),
            observable_state: snapshot.observable_state,
        }))
    }

    async fn find_latest_havana_snapshot_before_sequence(
        &self,
        sequence_nr: i32,
    ) -> Result<Option<crate::core::RunStageSnapshot>, EvaluatorRunnerError> {
        let mut search_seq = sequence_nr;
        loop {
            let Some(snapshot) = self
                .store
                .load_latest_stage_snapshot_before_sequence(self.run_id, search_seq)
                .await
                .map_err(EvaluatorRunnerError::Store)?
            else {
                return Ok(None);
            };
            if snapshot.sampler_snapshot.contains_havana_grid() {
                return Ok(Some(snapshot));
            }
            let prev_seq = snapshot.sequence_nr.unwrap_or(0);
            if prev_seq <= 0 {
                return Ok(None);
            }
            search_seq = prev_seq;
        }
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

        self.store
            .submit_batch_results(batch_id, &self.node_uuid, &result, total_time_ms)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

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
