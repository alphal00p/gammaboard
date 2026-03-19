//! Sampler task executor orchestration.
//!
//! This module owns one active sampler task at a time:
//! - restore/build the sampler and observable for that task
//! - enqueue latent batches
//! - fetch completed batches and pass training weights back into the sampler
//! - merge completed batch observables into the current observable state
//! - persist snapshots for resume and task handoff

use crate::core::{
    EngineError, EvaluatorConfig, ParametrizationConfig, ParametrizationState,
    RollingMetricSnapshot, RunSampleProgress, RunStageSnapshot, RunTask, SamplerAggregatorConfig,
    SamplerAggregatorPerformanceSnapshot, SamplerRollingAverages, SamplerRuntimeMetrics,
    SamplerWorkerStore, StoreError,
};
use crate::evaluation::{ObservableState, PointSpec};
use crate::runners::rolling_metric::RollingMetric;
use crate::sampling::{SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot, StageHandoff};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerAggregatorRunnerParams {
    pub min_poll_time_ms: u64,
    pub performance_snapshot_interval_ms: u64,
    pub target_batch_eval_ms: f64,
    pub target_queue_remaining: f64,
    pub max_batch_size: usize,
    pub max_queue_size: usize,
    pub max_batches_per_tick: usize,
    pub completed_batch_fetch_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RollingAveragesState {
    eval_ms_per_sample: RollingMetric,
    eval_ms_per_batch: RollingMetric,
    sampler_produce_ms_per_sample: RollingMetric,
    sampler_ingest_ms_per_sample: RollingMetric,
    completed_samples_per_second: RollingMetric,
    queue_remaining_ratio: RollingMetric,
    batches_consumed_per_tick: RollingMetric,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SamplerRuntimeState {
    produced_batches_total: i64,
    produced_samples_total: i64,
    ingested_batches_total: i64,
    ingested_samples_total: i64,
    batch_size_current: usize,
    rolling: RollingAveragesState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorRunnerSnapshot {
    pub task_id: i64,
    pub sampler_snapshot: SamplerAggregatorSnapshot,
    pub observable_state: ObservableState,
    runtime_state: SamplerRuntimeState,
    last_pending_after_enqueue: Option<usize>,
}

impl From<&RollingMetric> for RollingMetricSnapshot {
    fn from(metric: &RollingMetric) -> Self {
        Self {
            mean: metric.value(),
            std_dev: metric.std_dev(),
        }
    }
}

impl SamplerRuntimeState {
    fn to_runtime_metrics(&self) -> SamplerRuntimeMetrics {
        SamplerRuntimeMetrics {
            produced_batches_total: self.produced_batches_total,
            produced_samples_total: self.produced_samples_total,
            ingested_batches_total: self.ingested_batches_total,
            ingested_samples_total: self.ingested_samples_total,
            completed_samples_per_second: self
                .rolling
                .completed_samples_per_second
                .value()
                .unwrap_or(0.0),
            batch_size_current: self.batch_size_current,
            rolling: SamplerRollingAverages {
                eval_ms_per_sample: RollingMetricSnapshot::from(&self.rolling.eval_ms_per_sample),
                eval_ms_per_batch: RollingMetricSnapshot::from(&self.rolling.eval_ms_per_batch),
                sampler_produce_ms_per_sample: RollingMetricSnapshot::from(
                    &self.rolling.sampler_produce_ms_per_sample,
                ),
                sampler_ingest_ms_per_sample: RollingMetricSnapshot::from(
                    &self.rolling.sampler_ingest_ms_per_sample,
                ),
                queue_remaining_ratio: RollingMetricSnapshot::from(
                    &self.rolling.queue_remaining_ratio,
                ),
                batches_consumed_per_tick: RollingMetricSnapshot::from(
                    &self.rolling.batches_consumed_per_tick,
                ),
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub struct SamplerAggregatorRunner<S> {
    run_id: i32,
    node_id: String,
    task: RunTask,
    sampler: Box<dyn SamplerAggregator>,
    observable_state: ObservableState,
    sampler_config: SamplerAggregatorConfig,
    parametrization_state: ParametrizationState,
    store: S,
    config: SamplerAggregatorRunnerParams,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    runtime_state: SamplerRuntimeState,
    last_performance_completed_samples: i64,
    last_pending_after_enqueue: Option<usize>,
}

impl<S> SamplerAggregatorRunner<S>
where
    S: SamplerWorkerStore,
{
    const MIN_BATCH_SIZE: usize = 1;
    const MAX_BATCH_SIZE_UP_FACTOR: f64 = 1.25;
    const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.80;
    const MIN_BATCH_SIZE_CHANGE_RATIO: f64 = 0.03;

    fn validate_runner_config(config: &SamplerAggregatorRunnerParams) -> Result<(), RunnerError> {
        if config.max_batch_size == 0 {
            return Err(RunnerError::Engine(EngineError::engine(
                "runner config max_batch_size must be > 0",
            )));
        }
        if config.max_queue_size == 0 {
            return Err(RunnerError::Engine(EngineError::engine(
                "runner config max_queue_size must be > 0",
            )));
        }
        if !config.target_batch_eval_ms.is_finite() || config.target_batch_eval_ms <= 0.0 {
            return Err(RunnerError::Engine(EngineError::engine(
                "runner config target_batch_eval_ms must be > 0",
            )));
        }
        if !config.target_queue_remaining.is_finite()
            || config.target_queue_remaining < 0.0
            || config.target_queue_remaining > 1.0
        {
            return Err(RunnerError::Engine(EngineError::engine(
                "runner config target_queue_remaining must be in [0, 1]",
            )));
        }
        Ok(())
    }

    fn build_parametrization_state(
        parametrization: &ParametrizationConfig,
        point_spec: &PointSpec,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<ParametrizationState, RunnerError> {
        parametrization
            .build(handoff)
            .map_err(RunnerError::Engine)
            .and_then(|runtime| {
                runtime
                    .validate_point_spec(point_spec)
                    .map_err(RunnerError::Engine)?;
                runtime
                    .snapshot()
                    .map(|snapshot| ParametrizationState {
                        config: parametrization.clone(),
                        snapshot,
                    })
                    .map_err(RunnerError::Engine)
            })
    }

    fn tune_batch_size(&mut self) {
        let Some(eval_ms_per_sample) = self.runtime_state.rolling.eval_ms_per_sample.value() else {
            return;
        };
        if self.config.target_batch_eval_ms <= 0.0 || !self.config.target_batch_eval_ms.is_finite()
        {
            return;
        }
        if eval_ms_per_sample <= 0.0 {
            return;
        }
        let current_eval_batch_ms =
            eval_ms_per_sample * self.runtime_state.batch_size_current as f64;
        let raw_ratio = self.config.target_batch_eval_ms / current_eval_batch_ms;
        let ratio = raw_ratio.clamp(
            Self::MAX_BATCH_SIZE_DOWN_FACTOR,
            Self::MAX_BATCH_SIZE_UP_FACTOR,
        );
        if (ratio - 1.0).abs() < Self::MIN_BATCH_SIZE_CHANGE_RATIO {
            return;
        }

        let next = ((self.runtime_state.batch_size_current as f64) * ratio).round() as usize;
        self.runtime_state.batch_size_current =
            next.clamp(Self::MIN_BATCH_SIZE, self.config.max_batch_size);
    }

    fn compute_produce_limit(&self, pending_before_tick: usize) -> usize {
        let remaining_capacity = self
            .config
            .max_queue_size
            .saturating_sub(pending_before_tick);
        if remaining_capacity == 0 {
            return 0;
        }
        let hard_limit = remaining_capacity.min(self.config.max_batches_per_tick);
        if hard_limit == 0 {
            return 0;
        }

        let Some(consumed_per_tick) = self.runtime_state.rolling.batches_consumed_per_tick.value()
        else {
            return hard_limit;
        };
        if !consumed_per_tick.is_finite() || consumed_per_tick <= 0.0 {
            return hard_limit;
        }
        if self.config.target_queue_remaining == 1.0 {
            return hard_limit;
        }

        let target_pending_after_enqueue =
            (consumed_per_tick / (1.0 - self.config.target_queue_remaining)).ceil() as usize;
        let target_enqueue = target_pending_after_enqueue.saturating_sub(pending_before_tick);
        hard_limit.min(target_enqueue)
    }

    fn build_batch_plan(
        &self,
        base_produce_limit: usize,
        max_samples: Option<usize>,
    ) -> Vec<usize> {
        match max_samples {
            None => vec![self.runtime_state.batch_size_current; base_produce_limit],
            Some(max_samples) => {
                let base_total_samples =
                    base_produce_limit.saturating_mul(self.runtime_state.batch_size_current);
                if base_total_samples <= max_samples {
                    vec![self.runtime_state.batch_size_current; base_produce_limit]
                } else if max_samples == 0 || self.runtime_state.batch_size_current == 0 {
                    Vec::new()
                } else {
                    let nr_batches = max_samples.div_ceil(self.runtime_state.batch_size_current);
                    let base_size = max_samples / nr_batches;
                    let remainder = max_samples % nr_batches;
                    let mut plan = Vec::with_capacity(nr_batches);
                    for i in 0..nr_batches {
                        plan.push(if i < remainder {
                            base_size + 1
                        } else {
                            base_size
                        });
                    }
                    plan
                }
            }
        }
    }

    fn max_samples_to_produce_this_tick(
        &self,
        engine_max_samples: Option<usize>,
    ) -> Result<Option<usize>, RunnerError> {
        let task_max_samples = self.active_sample_remaining_budget()?;
        if let Some(task_remaining) = task_max_samples
            && task_remaining == 0
        {
            return Ok(Some(0));
        }
        Ok(match (engine_max_samples, task_max_samples) {
            (Some(engine_max), Some(task_remaining)) => Some(engine_max.min(task_remaining)),
            (Some(engine_max), None) => Some(engine_max),
            (None, Some(task_remaining)) => Some(task_remaining),
            (None, None) => None,
        })
    }

    fn active_sample_remaining_budget(&self) -> Result<Option<usize>, RunnerError> {
        let Some(target) = self.task.task.nr_expected_samples() else {
            return Ok(None);
        };
        let remaining = target.saturating_sub(self.task.nr_produced_samples);
        if remaining < 0 {
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} produced sample count exceeded target: produced={} target={}",
                self.run_id,
                self.task.id,
                self.task.nr_produced_samples,
                remaining + self.task.nr_produced_samples
            ))));
        }
        Ok(usize::try_from(remaining).ok())
    }

    fn observe_queue_metrics(&mut self, pending_before_tick: usize) {
        if let Some(previous_pending_after) = self.last_pending_after_enqueue
            && previous_pending_after > 0
        {
            let observed_ratio = (pending_before_tick as f64) / (previous_pending_after as f64);
            self.runtime_state
                .rolling
                .queue_remaining_ratio
                .observe(observed_ratio);
            let consumed = previous_pending_after.saturating_sub(pending_before_tick) as f64;
            self.runtime_state
                .rolling
                .batches_consumed_per_tick
                .observe(consumed);
        }
    }

    pub async fn new(
        run_id: i32,
        node_id: impl Into<String>,
        task: RunTask,
        store: S,
        config: SamplerAggregatorRunnerParams,
        point_spec: PointSpec,
        evaluator_config: EvaluatorConfig,
        restored_snapshot: Option<SamplerAggregatorRunnerSnapshot>,
    ) -> Result<Self, RunnerError> {
        Self::validate_runner_config(&config)?;
        let Some(sampler_config) = task.task.sampler_config() else {
            return Err(RunnerError::Engine(EngineError::engine(
                "task missing sampler config",
            )));
        };
        let Some(parametrization_config) = task.task.parametrization_config() else {
            return Err(RunnerError::Engine(EngineError::engine(
                "task missing parametrization config",
            )));
        };

        let performance_snapshot_interval =
            Duration::from_millis(config.performance_snapshot_interval_ms);
        let persisted_progress =
            store
                .load_run_sample_progress(run_id)
                .await?
                .unwrap_or(RunSampleProgress {
                    nr_produced_samples: 0,
                    nr_completed_samples: 0,
                });

        let sample_budget = task
            .task
            .nr_expected_samples()
            .and_then(|n| usize::try_from(n).ok());

        let (
            sampler,
            observable_state,
            runtime_state,
            last_pending_after_enqueue,
            parametrization_state,
        ) = if let Some(snapshot) = restored_snapshot {
            if snapshot.task_id != task.id {
                return Err(RunnerError::Engine(EngineError::engine(format!(
                    "sampler runner snapshot task mismatch: expected {}, got {}",
                    task.id, snapshot.task_id
                ))));
            }
            let activation_snapshot = store
                .load_task_activation_snapshot(run_id, task.id)
                .await?
                .ok_or_else(|| {
                    RunnerError::Store(StoreError::not_found(format!(
                        "missing activation stage snapshot for run {} task {}",
                        run_id, task.id
                    )))
                })?;
            let sampler = sampler_config
                .build(
                    point_spec.clone(),
                    sample_budget,
                    Some(StageHandoff {
                        sampler_snapshot: Some(&snapshot.sampler_snapshot),
                        parametrization_snapshot: Some(
                            &activation_snapshot.parametrization.snapshot,
                        ),
                        observable_state: Some(&snapshot.observable_state),
                    }),
                )
                .map_err(RunnerError::Engine)?;
            (
                sampler,
                snapshot.observable_state,
                snapshot.runtime_state,
                snapshot.last_pending_after_enqueue,
                activation_snapshot.parametrization,
            )
        } else {
            let (previous_snapshot, spawn_origin) = if let Some(start_from) = task.task.start_from()
            {
                let snapshot = store
                    .load_latest_stage_snapshot_for_task(start_from.run_id, start_from.task_id)
                    .await?;
                let snapshot = snapshot.ok_or_else(|| {
                    RunnerError::Store(StoreError::not_found(format!(
                        "no queue-empty stage snapshot found for run {} task {}",
                        start_from.run_id, start_from.task_id
                    )))
                })?;
                (
                    Some(snapshot),
                    Some((start_from.run_id, start_from.task_id)),
                )
            } else {
                let snapshot = store
                    .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
                    .await?;
                let origin = snapshot.as_ref().and_then(|snapshot| {
                    snapshot.task_id.map(|task_id| (snapshot.run_id, task_id))
                });
                (snapshot, origin)
            };

            store
                .set_run_task_spawn_origin(
                    task.id,
                    spawn_origin.map(|(from_run_id, _)| from_run_id),
                    spawn_origin.map(|(_, from_task_id)| from_task_id),
                )
                .await?;

            let handoff = previous_snapshot.as_ref().map(|snapshot| StageHandoff {
                sampler_snapshot: Some(&snapshot.sampler_snapshot),
                parametrization_snapshot: Some(&snapshot.parametrization.snapshot),
                observable_state: Some(&snapshot.observable_state),
            });
            let parametrization_state =
                Self::build_parametrization_state(&parametrization_config, &point_spec, handoff)?;
            let handoff = previous_snapshot.as_ref().map(|snapshot| StageHandoff {
                sampler_snapshot: Some(&snapshot.sampler_snapshot),
                parametrization_snapshot: Some(&snapshot.parametrization.snapshot),
                observable_state: Some(&snapshot.observable_state),
            });
            let sampler = sampler_config
                .build(point_spec.clone(), sample_budget, handoff)
                .map_err(RunnerError::Engine)?;
            let base_observable_config = previous_snapshot
                .as_ref()
                .map(|snapshot| snapshot.observable_state.config())
                .unwrap_or_else(|| evaluator_config.default_observable_config());
            let observable_state = if let Some(observable_config) = task
                .task
                .explicit_observable_config(&base_observable_config)
            {
                evaluator_config
                    .empty_observable_state(&observable_config)
                    .map_err(RunnerError::Engine)?
            } else if let Some(snapshot) = previous_snapshot.as_ref() {
                snapshot.observable_state.clone()
            } else {
                evaluator_config
                    .empty_observable_state(&base_observable_config)
                    .map_err(RunnerError::Engine)?
            };

            let runtime_state = SamplerRuntimeState {
                produced_samples_total: persisted_progress.nr_produced_samples,
                batch_size_current: config.max_batch_size.min(64).max(Self::MIN_BATCH_SIZE),
                ..SamplerRuntimeState::default()
            };

            (
                sampler,
                observable_state,
                runtime_state,
                None,
                parametrization_state,
            )
        };

        Ok(Self {
            run_id,
            node_id: node_id.into(),
            task,
            sampler,
            observable_state,
            sampler_config,
            parametrization_state,
            store,
            config,
            nr_produced_samples: persisted_progress.nr_produced_samples,
            nr_completed_samples: persisted_progress.nr_completed_samples,
            performance_snapshot_interval,
            last_snapshot_at: Instant::now(),
            runtime_state,
            last_performance_completed_samples: persisted_progress.nr_completed_samples,
            last_pending_after_enqueue,
        })
    }

    pub fn task_id(&self) -> i64 {
        self.task.id
    }

    pub fn task_state(&self) -> &RunTask {
        &self.task
    }

    pub async fn tick(&mut self) -> Result<bool, RunnerError> {
        let pending_before_tick = self
            .store
            .get_pending_batch_count(self.run_id)
            .await?
            .max(0) as usize;
        self.observe_queue_metrics(pending_before_tick);
        self.tune_batch_size();

        self.process_completed().await?;
        self.produce(pending_before_tick).await?;
        self.sync_task_progress().await?;
        self.flush_run_sample_progress().await?;
        self.flush_performance_snapshot().await?;

        let open_batch_count = self.store.get_open_batch_count(self.run_id).await?.max(0) as usize;
        Ok(self.task.task.nr_expected_samples().is_some_and(|target| {
            self.task.nr_completed_samples >= target && open_batch_count == 0
        }))
    }

    pub async fn persist_state(&mut self, queue_empty: bool) -> Result<(), RunnerError> {
        let snapshot = SamplerAggregatorRunnerSnapshot {
            task_id: self.task.id,
            sampler_snapshot: self.sampler.snapshot().map_err(RunnerError::Engine)?,
            observable_state: self.observable_state.clone(),
            runtime_state: self.runtime_state.clone(),
            last_pending_after_enqueue: self.last_pending_after_enqueue,
        };
        self.store
            .save_sampler_runner_snapshot(self.run_id, &snapshot)
            .await?;
        self.store
            .save_run_stage_snapshot(&RunStageSnapshot {
                run_id: self.run_id,
                task_id: Some(self.task.id),
                sequence_nr: Some(self.task.sequence_nr),
                queue_empty,
                sampler_snapshot: snapshot.sampler_snapshot.clone(),
                observable_state: self.observable_state.clone(),
                sampler_aggregator: self.sampler_config.clone(),
                parametrization: self.parametrization_state.clone(),
            })
            .await?;
        Ok(())
    }

    pub async fn complete_task(&mut self) -> Result<(), RunnerError> {
        self.sync_task_progress().await?;
        self.store.complete_run_task(self.task.id).await?;
        Ok(())
    }

    pub async fn fail_task(&mut self, reason: &str) -> Result<(), RunnerError> {
        let queue_empty = self.store.get_open_batch_count(self.run_id).await? <= 0;
        self.persist_state(queue_empty).await?;
        self.store.fail_run_task(self.task.id, reason).await?;
        Ok(())
    }

    async fn process_completed(&mut self) -> Result<usize, RunnerError> {
        let completed = self
            .store
            .fetch_completed_batches(self.run_id, self.config.completed_batch_fetch_limit)
            .await?;
        if completed.is_empty() {
            return Ok(0);
        }

        let mut completed_samples_delta = 0_i64;
        for batch in &completed {
            let batch_samples = batch.latent_batch.nr_samples;
            completed_samples_delta += batch_samples as i64;
            if let Some(total_eval_time_ms) = batch.total_eval_time_ms
                && batch_samples > 0
            {
                self.runtime_state
                    .rolling
                    .eval_ms_per_batch
                    .observe(total_eval_time_ms);
                self.runtime_state
                    .rolling
                    .eval_ms_per_sample
                    .observe(total_eval_time_ms / batch_samples as f64);
            }

            if self.sampler_config.requires_training() {
                let training_weights = batch.result.values.as_deref().ok_or_else(|| {
                    RunnerError::Engine(EngineError::engine(format!(
                        "completed batch {} requires training but has no training values",
                        batch.batch_id
                    )))
                })?;
                if training_weights.len() != batch_samples {
                    return Err(RunnerError::Engine(EngineError::engine(format!(
                        "completed batch {} training value count mismatch: expected {}, got {}",
                        batch.batch_id,
                        batch_samples,
                        training_weights.len()
                    ))));
                }
                let ingest_started = Instant::now();
                self.sampler
                    .ingest_training_weights(training_weights)
                    .map_err(RunnerError::Engine)?;
                let ingest_time_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
                self.runtime_state.ingested_batches_total += 1;
                self.runtime_state.ingested_samples_total += batch_samples as i64;
                if batch_samples > 0 {
                    self.runtime_state
                        .rolling
                        .sampler_ingest_ms_per_sample
                        .observe(ingest_time_ms / batch_samples as f64);
                }
            }

            self.observable_state
                .merge(batch.result.observable.clone())
                .map_err(RunnerError::Engine)?;
        }

        self.nr_completed_samples += completed_samples_delta;
        if self.task.task.nr_expected_samples().is_some() {
            self.task.nr_completed_samples += completed_samples_delta;
        }

        let current_observable = self
            .observable_state
            .to_json()
            .map_err(RunnerError::Engine)?;
        let snapshot = self
            .observable_state
            .to_persistent_json()
            .map_err(RunnerError::Engine)?;
        self.store
            .save_aggregation(
                self.run_id,
                self.task.id,
                &current_observable,
                &snapshot,
                completed.len() as i32,
            )
            .await?;

        let consumed_ids = completed
            .iter()
            .map(|batch| batch.batch_id)
            .collect::<Vec<_>>();
        self.store.delete_completed_batches(&consumed_ids).await?;
        Ok(consumed_ids.len())
    }

    async fn produce(&mut self, pending_before_tick: usize) -> Result<usize, RunnerError> {
        let observable_config = self.observable_state.config();
        let sample_plan = self.sampler.sample_plan().map_err(RunnerError::Engine)?;
        let training_samples_remaining = self.sampler.training_samples_remaining();
        let batch_plan = match sample_plan {
            SamplePlan::Pause => Vec::new(),
            SamplePlan::Produce { nr_samples } => {
                let base_produce_limit = self.compute_produce_limit(pending_before_tick);
                let requested = if nr_samples == usize::MAX {
                    None
                } else {
                    Some(nr_samples)
                };
                let engine_max_samples = match requested {
                    Some(requested) => Some(
                        training_samples_remaining
                            .map_or(requested, |remaining| remaining.min(requested)),
                    ),
                    None => training_samples_remaining,
                };
                self.build_batch_plan(
                    base_produce_limit,
                    self.max_samples_to_produce_this_tick(engine_max_samples)?,
                )
            }
        };

        let mut produced = Vec::with_capacity(batch_plan.len());
        for nr_samples in batch_plan {
            let started = Instant::now();
            let batch = self
                .sampler
                .produce_latent_batch(nr_samples)
                .map_err(RunnerError::Engine)?;
            let produce_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            let produced_samples = batch.nr_samples;
            self.runtime_state.produced_batches_total += 1;
            self.runtime_state.produced_samples_total += produced_samples as i64;
            self.nr_produced_samples += produced_samples as i64;
            self.task.nr_produced_samples += produced_samples as i64;
            if produced_samples > 0 {
                self.runtime_state
                    .rolling
                    .sampler_produce_ms_per_sample
                    .observe(produce_time_ms / produced_samples as f64);
            }
            produced.push(
                batch
                    .with_observable_config(observable_config.clone())
                    .build(),
            );
        }

        for batch in &produced {
            self.store
                .insert_batch(self.run_id, self.task.id, batch)
                .await?;
        }
        self.last_pending_after_enqueue = Some(pending_before_tick.saturating_add(produced.len()));
        Ok(produced.len())
    }

    async fn sync_task_progress(&mut self) -> Result<(), RunnerError> {
        if self.task.task.nr_expected_samples().is_none() {
            return Ok(());
        }
        self.store
            .update_run_task_progress(
                self.task.id,
                self.task.nr_produced_samples,
                self.task.nr_completed_samples,
            )
            .await?;
        Ok(())
    }

    async fn flush_run_sample_progress(&mut self) -> Result<(), RunnerError> {
        self.store
            .save_run_sample_progress(
                self.run_id,
                self.nr_produced_samples,
                self.nr_completed_samples,
            )
            .await?;
        Ok(())
    }

    async fn flush_performance_snapshot(&mut self) -> Result<(), RunnerError> {
        if self.runtime_state.produced_batches_total <= 0
            && self.runtime_state.ingested_batches_total <= 0
        {
            return Ok(());
        }

        let due = if self.performance_snapshot_interval.is_zero() {
            true
        } else {
            self.last_snapshot_at.elapsed() >= self.performance_snapshot_interval
        };
        if !due {
            return Ok(());
        }

        let elapsed_secs = self.last_snapshot_at.elapsed().as_secs_f64();
        let completed_delta = self
            .nr_completed_samples
            .saturating_sub(self.last_performance_completed_samples);
        let completed_samples_per_second = if elapsed_secs > 0.0 {
            (completed_delta as f64 / elapsed_secs).max(0.0)
        } else {
            0.0
        };
        self.runtime_state
            .rolling
            .completed_samples_per_second
            .observe(completed_samples_per_second);

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            node_id: self.node_id.clone(),
            runtime_metrics: self.runtime_state.to_runtime_metrics(),
            engine_diagnostics: self.sampler.get_diagnostics(),
        };
        self.store
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
        self.last_performance_completed_samples = self.nr_completed_samples;
        self.last_snapshot_at = Instant::now();
        Ok(())
    }
}
