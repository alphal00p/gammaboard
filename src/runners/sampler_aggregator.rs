//! Sampler task executor orchestration.
//!
//! This module owns one active sampler task at a time:
//! - restore/build the sampler and observable for that task
//! - enqueue latent batches
//! - fetch completed batches and pass training weights back into the sampler
//! - merge completed batch observables into the current observable state
//! - persist snapshots for resume and task handoff

use crate::core::{
    BatchTransformConfig, EngineError, RollingMetricSnapshot, RunSampleProgress, RunStageSnapshot,
    RunTask, SamplerAggregatorConfig, SamplerAggregatorPerformanceSnapshot, SamplerRollingAverages,
    SamplerRuntimeMetrics, SamplerWorkerStore, StoreError,
};
use crate::evaluation::ObservableState;
use crate::runners::rolling_metric::RollingMetric;
use crate::sampling::{SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::time::{Duration, Instant};
use thiserror::Error;

const MIN_BATCH_SIZE: usize = 16;
const MAX_BATCH_SIZE_UP_FACTOR: f64 = 4.0;
const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.25;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerAggregatorRunnerParams {
    pub performance_snapshot_interval_ms: u64,
    pub aggregation_persist_interval_ms: u64,
    pub target_batch_eval_ms: f64,
    pub queue_buffer: f64,
    pub max_batch_size: usize,
    pub max_queue_size: usize,
    pub max_batches_per_tick: usize,
    pub completed_batch_fetch_limit: usize,
    pub strict_batch_ordering: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RollingAveragesState {
    eval_ms_per_sample: RollingMetric,
    eval_ms_per_batch: RollingMetric,
    sampler_produce_ms_per_sample: RollingMetric,
    sampler_ingest_ms_per_sample: RollingMetric,
    completed_samples_per_second: RollingMetric,
    runnable_queue_retained_ratio: RollingMetric,
    runnable_batches_consumed_per_tick: RollingMetric,
    batches_consumed_per_second: RollingMetric,
    sampler_tick_ms: RollingMetric,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
enum ObservableCheckpointState {
    #[default]
    NeedsInitialRoundTrip,
    WaitingForInitialRoundTrip,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SamplerRuntimeState {
    produced_batches_total: i64,
    produced_samples_total: i64,
    ingested_batches_total: i64,
    ingested_samples_total: i64,
    pending_persisted_completed_batches: i32,
    #[serde(default)]
    last_completed_batch_id: Option<i64>,
    batch_size_current: usize,
    observable_checkpoint_state: ObservableCheckpointState,
    rolling: RollingAveragesState,
}

impl Default for SamplerRuntimeState {
    fn default() -> Self {
        Self {
            produced_batches_total: 0,
            produced_samples_total: 0,
            ingested_batches_total: 0,
            ingested_samples_total: 0,
            pending_persisted_completed_batches: 0,
            last_completed_batch_id: None,
            batch_size_current: 0,
            observable_checkpoint_state: ObservableCheckpointState::NeedsInitialRoundTrip,
            rolling: RollingAveragesState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorRunnerSnapshot {
    pub task_id: i64,
    pub sampler_snapshot: SamplerAggregatorSnapshot,
    pub observable_state: ObservableState,
    runtime_state: SamplerRuntimeState,
    last_runnable_after_enqueue: Option<usize>,
}

impl From<&RollingMetric> for RollingMetricSnapshot {
    fn from(metric: &RollingMetric) -> Self {
        Self {
            mean: metric.value(),
            std_dev: metric.std_dev(),
        }
    }
}

impl SamplerAggregatorRunnerSnapshot {
    pub fn reduced_carryover_batch_size(&self, max_batch_size: usize) -> usize {
        let reduced = ((self.runtime_state.batch_size_current as f64) * MAX_BATCH_SIZE_DOWN_FACTOR)
            .round() as usize;
        reduced.clamp(MIN_BATCH_SIZE, max_batch_size)
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
                runnable_queue_retained_ratio: RollingMetricSnapshot::from(
                    &self.rolling.runnable_queue_retained_ratio,
                ),
                runnable_batches_consumed_per_tick: RollingMetricSnapshot::from(
                    &self.rolling.runnable_batches_consumed_per_tick,
                ),
                batches_consumed_per_second: RollingMetricSnapshot::from(
                    &self.rolling.batches_consumed_per_second,
                ),
                sampler_tick_ms: RollingMetricSnapshot::from(&self.rolling.sampler_tick_ms),
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
    node_name: String,
    task: RunTask,
    sampler: Box<dyn SamplerAggregator>,
    observable_state: ObservableState,
    sampler_config: SamplerAggregatorConfig,
    batch_transforms: Vec<BatchTransformConfig>,
    store: S,
    config: SamplerAggregatorRunnerParams,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
    performance_snapshot_interval: Duration,
    aggregation_persist_interval: Duration,
    last_snapshot_at: Instant,
    last_aggregation_persist_at: Instant,
    runtime_state: SamplerRuntimeState,
    last_performance_completed_samples: i64,
    last_runnable_after_enqueue: Option<usize>,
    last_tick_started_at: Option<Instant>,
}

impl<S> SamplerAggregatorRunner<S>
where
    S: SamplerWorkerStore,
{
    pub fn new(
        store: S,
        run_id: i32,
        node_name: impl Into<String>,
        task: RunTask,
        sampler: Box<dyn SamplerAggregator>,
        observable_state: ObservableState,
        sampler_config: SamplerAggregatorConfig,
        batch_transforms: Vec<BatchTransformConfig>,
        params: SamplerAggregatorRunnerParams,
        initial_batch_size: usize,
        resume_snapshot: Option<SamplerAggregatorRunnerSnapshot>,
        run_progress: Option<RunSampleProgress>,
    ) -> Self {
        let mut runtime_state;
        let last_runnable_after_enqueue;
        if let Some(snapshot) = resume_snapshot {
            runtime_state = snapshot.runtime_state.clone();
            last_runnable_after_enqueue = snapshot.last_runnable_after_enqueue;
        } else {
            runtime_state = SamplerRuntimeState {
                batch_size_current: initial_batch_size.clamp(MIN_BATCH_SIZE, params.max_batch_size),
                ..SamplerRuntimeState::default()
            };
            last_runnable_after_enqueue = None;
        }
        runtime_state.batch_size_current = runtime_state
            .batch_size_current
            .clamp(MIN_BATCH_SIZE, params.max_batch_size);

        let (nr_produced_samples, nr_completed_samples) = run_progress
            .map(|progress| (progress.nr_produced_samples, progress.nr_completed_samples))
            .unwrap_or((0, 0));
        if nr_completed_samples > 0 {
            runtime_state.observable_checkpoint_state = ObservableCheckpointState::Ready;
        }

        let performance_snapshot_interval =
            Duration::from_millis(params.performance_snapshot_interval_ms);
        let aggregation_persist_interval =
            Duration::from_millis(params.aggregation_persist_interval_ms);
        let now = Instant::now();

        Self {
            run_id,
            node_name: node_name.into(),
            task,
            sampler,
            observable_state,
            sampler_config,
            batch_transforms,
            store,
            config: params,
            nr_produced_samples,
            nr_completed_samples,
            performance_snapshot_interval,
            aggregation_persist_interval,
            last_snapshot_at: now,
            last_aggregation_persist_at: now,
            runtime_state,
            last_performance_completed_samples: nr_completed_samples,
            last_runnable_after_enqueue,
            last_tick_started_at: None,
        }
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
        let ratio = raw_ratio.clamp(MAX_BATCH_SIZE_DOWN_FACTOR, MAX_BATCH_SIZE_UP_FACTOR);

        let next = ((self.runtime_state.batch_size_current as f64) * ratio).round() as usize;
        self.runtime_state.batch_size_current =
            next.clamp(MIN_BATCH_SIZE, self.config.max_batch_size);
    }

    fn hard_produce_limit(&self, open_before_tick: usize) -> usize {
        let remaining_capacity = self.config.max_queue_size.saturating_sub(open_before_tick);
        remaining_capacity.min(self.config.max_batches_per_tick)
    }

    fn target_pending_batches(&self, active_evaluator_count: usize) -> Option<usize> {
        if !self.config.queue_buffer.is_finite() || self.config.queue_buffer < 0.0 {
            return None;
        }
        Some(
            ((active_evaluator_count as f64) * self.config.queue_buffer)
                .ceil()
                .max(0.0) as usize,
        )
    }

    async fn current_runner_diagnostics(&self) -> Result<JsonValue, RunnerError> {
        let queue_counts = self.store.get_batch_queue_counts(self.run_id).await?;
        let active_evaluator_count = self.active_evaluator_count().await?;
        let target_pending_batches = self.target_pending_batches(active_evaluator_count);
        Ok(json!({
            "active_evaluator_count": active_evaluator_count,
            "pending_batches": queue_counts.pending,
            "claimed_batches": queue_counts.claimed,
            "completed_batches": queue_counts.completed,
            "open_batches": queue_counts.open(),
            "queue_buffer": self.config.queue_buffer,
            "target_pending_batches": target_pending_batches,
            "pending_shortfall": target_pending_batches
                .map(|target| (target as i64).saturating_sub(queue_counts.pending as i64)),
            "last_completed_batch_id": self.runtime_state.last_completed_batch_id,
            "observable_checkpoint_state": match self.runtime_state.observable_checkpoint_state {
                ObservableCheckpointState::NeedsInitialRoundTrip => "needs_initial_round_trip",
                ObservableCheckpointState::WaitingForInitialRoundTrip => "waiting_for_initial_round_trip",
                ObservableCheckpointState::Ready => "ready",
            },
            "training_samples_remaining": self.sampler.training_samples_remaining(),
        }))
    }

    fn compute_produce_limit(
        &self,
        pending_before_tick: usize,
        open_before_tick: usize,
        active_evaluator_count: usize,
    ) -> usize {
        let hard_limit = self.hard_produce_limit(open_before_tick);
        if hard_limit == 0 {
            return 0;
        }

        let Some(target_pending_after_enqueue) =
            self.target_pending_batches(active_evaluator_count)
        else {
            return 0;
        };

        hard_limit.min(target_pending_after_enqueue.saturating_sub(pending_before_tick))
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

    fn observe_tick_timing(&mut self, tick_started_at: Instant) {
        if let Some(previous_tick_started_at) = self.last_tick_started_at {
            let tick_ms = tick_started_at
                .saturating_duration_since(previous_tick_started_at)
                .as_secs_f64()
                * 1000.0;
            if tick_ms.is_finite() && tick_ms > 0.0 {
                self.runtime_state.rolling.sampler_tick_ms.observe(tick_ms);
            }
        }
        self.last_tick_started_at = Some(tick_started_at);
    }

    fn observe_queue_metrics(&mut self, pending_before_tick: usize) {
        if let Some(previous_runnable_after) = self.last_runnable_after_enqueue
            && previous_runnable_after > 0
        {
            let observed_ratio = (pending_before_tick as f64) / (previous_runnable_after as f64);
            self.runtime_state
                .rolling
                .runnable_queue_retained_ratio
                .observe(observed_ratio);
            let consumed = previous_runnable_after.saturating_sub(pending_before_tick) as f64;
            self.runtime_state
                .rolling
                .runnable_batches_consumed_per_tick
                .observe(consumed);
            if let Some(avg_tick_ms) = self.runtime_state.rolling.sampler_tick_ms.value()
                && avg_tick_ms.is_finite()
                && avg_tick_ms > 0.0
            {
                self.runtime_state
                    .rolling
                    .batches_consumed_per_second
                    .observe(consumed * 1000.0 / avg_tick_ms);
            }
        }
    }

    async fn active_evaluator_count(&self) -> Result<usize, RunnerError> {
        Ok(self
            .store
            .count_active_evaluator_nodes(self.run_id)
            .await?
            .max(0) as usize)
    }

    pub fn task_id(&self) -> i64 {
        self.task.id
    }

    pub fn task_state(&self) -> &RunTask {
        &self.task
    }

    pub async fn tick(&mut self) -> Result<bool, RunnerError> {
        let tick_started_at = Instant::now();
        self.observe_tick_timing(tick_started_at);
        self.store.reclaim_abandoned_batches(self.run_id).await?;
        let queue_before_tick = self.store.get_batch_queue_counts(self.run_id).await?;
        self.observe_queue_metrics(queue_before_tick.pending.max(0) as usize);
        self.tune_batch_size();

        let active_evaluator_count = self.active_evaluator_count().await?;
        let completed_batches = self.process_completed().await?;
        let queue_before_produce = crate::core::BatchQueueCounts {
            pending: queue_before_tick.pending,
            claimed: queue_before_tick.claimed,
            completed: queue_before_tick
                .completed
                .saturating_sub(completed_batches as i64),
        };
        let produced_batches = self
            .produce(queue_before_produce, active_evaluator_count)
            .await?;
        self.sync_task_progress().await?;
        self.flush_run_sample_progress().await?;
        self.flush_performance_snapshot().await?;

        let open_batch_count = (queue_before_produce
            .open()
            .saturating_add(produced_batches as i64))
        .max(0) as usize;
        if let Some(target) = self.task.task.nr_expected_samples()
            && self.task.nr_completed_samples < target
            && open_batch_count == 0
            && completed_batches == 0
            && produced_batches == 0
        {
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} cannot make further progress: completed={} target={} and sampler produced no new batches",
                self.run_id, self.task.id, self.task.nr_completed_samples, target
            ))));
        }
        Ok(self.task.task.nr_expected_samples().is_some_and(|target| {
            self.task.nr_completed_samples >= target && open_batch_count == 0
        }))
    }

    async fn persist_state_with_queue_empty(
        &mut self,
        queue_empty: bool,
    ) -> Result<(), RunnerError> {
        let snapshot = SamplerAggregatorRunnerSnapshot {
            task_id: self.task.id,
            sampler_snapshot: self.sampler.snapshot().map_err(RunnerError::Engine)?,
            observable_state: self.observable_state.clone(),
            runtime_state: self.runtime_state.clone(),
            last_runnable_after_enqueue: self.last_runnable_after_enqueue,
        };
        self.store
            .save_sampler_runner_snapshot(self.run_id, &snapshot)
            .await?;

        self.store
            .save_run_stage_snapshot(&RunStageSnapshot {
                id: None,
                run_id: self.run_id,
                task_id: Some(self.task.id),
                name: self.task.name.clone(),
                sequence_nr: Some(self.task.sequence_nr),
                queue_empty,
                sampler_snapshot: Some(snapshot.sampler_snapshot.clone()),
                observable_state: Some(self.observable_state.clone()),
                sampler_aggregator: Some(self.sampler_config.clone()),
                batch_transforms: self.batch_transforms.clone(),
            })
            .await?;
        Ok(())
    }

    pub async fn persist_state(&mut self) -> Result<(), RunnerError> {
        let queue_empty = self.store.get_open_batch_count(self.run_id).await? <= 0;
        self.persist_state_with_queue_empty(queue_empty).await
    }

    async fn flush_aggregation(&mut self, force: bool) -> Result<(), RunnerError> {
        if !force && self.runtime_state.pending_persisted_completed_batches <= 0 {
            return Ok(());
        }
        let due = force
            || self.aggregation_persist_interval.is_zero()
            || self.last_aggregation_persist_at.elapsed() >= self.aggregation_persist_interval;
        if !due {
            return Ok(());
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
                self.runtime_state.pending_persisted_completed_batches,
            )
            .await?;
        self.runtime_state.pending_persisted_completed_batches = 0;
        self.last_aggregation_persist_at = Instant::now();
        Ok(())
    }

    pub async fn complete_task(&mut self) -> Result<(), RunnerError> {
        self.flush_aggregation(true).await?;
        self.persist_state_with_queue_empty(true).await?;
        self.sync_task_progress().await?;
        self.store.complete_run_task(self.task.id).await?;
        Ok(())
    }

    pub async fn fail_task(&mut self, reason: &str) -> Result<(), RunnerError> {
        self.persist_state().await?;
        self.store.fail_run_task(self.task.id, reason).await?;
        Ok(())
    }

    async fn process_completed(&mut self) -> Result<usize, RunnerError> {
        let completed = self
            .store
            .fetch_completed_batches(
                self.run_id,
                self.config.completed_batch_fetch_limit,
                self.config.strict_batch_ordering,
                self.runtime_state.last_completed_batch_id,
            )
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

            if batch.requires_training_values {
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

        self.runtime_state.pending_persisted_completed_batches = self
            .runtime_state
            .pending_persisted_completed_batches
            .saturating_add(completed.len() as i32);
        let force_persist =
            self.runtime_state.observable_checkpoint_state != ObservableCheckpointState::Ready;
        self.flush_aggregation(force_persist).await?;
        if completed_samples_delta > 0 {
            self.runtime_state.observable_checkpoint_state = ObservableCheckpointState::Ready;
        }

        let consumed_ids = completed
            .iter()
            .map(|batch| batch.batch_id)
            .collect::<Vec<_>>();
        self.store.delete_completed_batches(&consumed_ids).await?;
        self.runtime_state.last_completed_batch_id = consumed_ids.last().copied();
        Ok(consumed_ids.len())
    }

    async fn produce(
        &mut self,
        queue_before_produce: crate::core::BatchQueueCounts,
        active_evaluator_count: usize,
    ) -> Result<usize, RunnerError> {
        let observable_config = self.observable_state.config();
        let sample_plan = self.sampler.sample_plan().map_err(RunnerError::Engine)?;
        let training_samples_remaining = self.sampler.training_samples_remaining();
        let pending_before_produce = queue_before_produce.pending.max(0) as usize;
        let open_before_produce = queue_before_produce.open().max(0) as usize;
        let batch_plan = match sample_plan {
            SamplePlan::Pause => Vec::new(),
            SamplePlan::Produce { nr_samples } => {
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
                let max_samples = self.max_samples_to_produce_this_tick(engine_max_samples)?;
                match self.runtime_state.observable_checkpoint_state {
                    ObservableCheckpointState::NeedsInitialRoundTrip => {
                        let nr_samples = max_samples.unwrap_or(MIN_BATCH_SIZE);
                        if nr_samples == 0 {
                            Vec::new()
                        } else {
                            self.runtime_state.observable_checkpoint_state =
                                ObservableCheckpointState::WaitingForInitialRoundTrip;
                            vec![nr_samples.min(MIN_BATCH_SIZE)]
                        }
                    }
                    ObservableCheckpointState::WaitingForInitialRoundTrip => {
                        if open_before_produce == 0 {
                            self.runtime_state.observable_checkpoint_state =
                                ObservableCheckpointState::NeedsInitialRoundTrip;
                        }
                        Vec::new()
                    }
                    ObservableCheckpointState::Ready => {
                        let base_produce_limit = self.compute_produce_limit(
                            pending_before_produce,
                            open_before_produce,
                            active_evaluator_count,
                        );
                        self.build_batch_plan(base_produce_limit, max_samples)
                    }
                }
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

        self.store
            .insert_batches(
                self.run_id,
                self.task.id,
                self.sampler_config.requires_training(),
                &produced,
            )
            .await?;
        self.last_runnable_after_enqueue =
            Some(pending_before_produce.saturating_add(produced.len()));
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

        let mut engine_diagnostics = self.sampler.get_diagnostics();
        let runner_diagnostics = self.current_runner_diagnostics().await?;
        match &mut engine_diagnostics {
            JsonValue::Object(object) => {
                object.insert("runner".to_string(), runner_diagnostics);
            }
            other => {
                engine_diagnostics = json!({
                    "sampler": other.clone(),
                    "runner": runner_diagnostics,
                });
            }
        }

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            node_name: self.node_name.clone(),
            runtime_metrics: self.runtime_state.to_runtime_metrics(),
            engine_diagnostics,
        };
        self.store
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
        self.last_performance_completed_samples = self.nr_completed_samples;
        self.last_snapshot_at = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SamplerAggregatorRunnerSnapshot, SamplerRuntimeState};
    use crate::core::{LineRasterGeometry, Linspace, PlaneRasterGeometry, SamplerAggregatorConfig};
    use crate::sampling::{
        NaiveMonteCarloSamplerParams, RasterLineSamplerParams, RasterPlaneSamplerParams,
        SamplerAggregatorSnapshot,
    };
    use serde_json::json;

    #[test]
    fn snapshot_kind_match_requires_same_sampler_family() {
        let raster_plane = SamplerAggregatorConfig::RasterPlane {
            params: RasterPlaneSamplerParams {
                geometry: PlaneRasterGeometry {
                    offset: vec![0.0, 0.0],
                    u_vector: vec![1.0, 0.0],
                    v_vector: vec![0.0, 1.0],
                    u_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    v_linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: Vec::new(),
                },
            },
        };
        let raster_line = SamplerAggregatorConfig::RasterLine {
            params: RasterLineSamplerParams {
                geometry: LineRasterGeometry {
                    offset: vec![0.0],
                    direction: vec![1.0],
                    linspace: Linspace {
                        start: 0.0,
                        stop: 1.0,
                        count: 2,
                    },
                    discrete: Vec::new(),
                },
            },
        };
        let naive = SamplerAggregatorConfig::NaiveMonteCarlo {
            params: NaiveMonteCarloSamplerParams::default(),
        };

        assert!(
            SamplerAggregatorSnapshot::RasterPlane { raw: json!({}) }.matches_config(&raster_plane)
        );
        assert!(
            !SamplerAggregatorSnapshot::RasterLine { raw: json!({}) }.matches_config(&raster_plane)
        );
        assert!(
            !SamplerAggregatorSnapshot::RasterPlane { raw: json!({}) }.matches_config(&raster_line)
        );
        assert!(
            SamplerAggregatorSnapshot::NaiveMonteCarlo { raw: json!({}) }.matches_config(&naive)
        );
    }

    #[test]
    fn carryover_batch_size_is_reduced_and_clamped() {
        let snapshot = SamplerAggregatorRunnerSnapshot {
            task_id: 1,
            sampler_snapshot: SamplerAggregatorSnapshot::NaiveMonteCarlo { raw: json!({}) },
            observable_state: crate::evaluation::ObservableState::empty_scalar(),
            runtime_state: SamplerRuntimeState {
                batch_size_current: 128,
                ..SamplerRuntimeState::default()
            },
            last_runnable_after_enqueue: None,
        };

        assert_eq!(snapshot.reduced_carryover_batch_size(512), 32);
        assert_eq!(snapshot.reduced_carryover_batch_size(24), 24);
    }
}
