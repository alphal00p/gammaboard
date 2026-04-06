//! Sampler task executor orchestration.
//!
//! This module owns one active sampler task at a time:
//! - restore/build the sampler and observable for that task
//! - enqueue latent batches
//! - fetch completed batches and pass training weights back into the sampler
//! - merge completed batch observables into the current observable state
//! - persist lightweight UI sync snapshots and full resume checkpoints

use crate::core::{
    BatchTransformConfig, EngineError, RollingMetricSnapshot, RunSampleProgress, RunStageSnapshot,
    RunTask, SamplerAggregatorConfig, SamplerAggregatorPerformanceSnapshot, SamplerRuntimeMetrics,
    SamplerWorkRollingAverages, SamplerWorkerStore, StoreError,
};
use crate::evaluation::ObservableState;
use crate::runners::process_memory::current_rss_bytes;
use crate::runners::rolling_metric::RollingMetric;
use crate::runners::{SamplerQueue, SamplerQueueCheckpoint, SamplerQueueConfig};
use crate::sampling::{SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::sleep;

const MIN_BATCH_SIZE: usize = 16;
const MAX_BATCH_SIZE_UP_FACTOR: f64 = 4.0;
const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.25;
const RECLAIM_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETED_CLEANUP_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETED_CLEANUP_BATCH_LIMIT: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerAggregatorRunnerParams {
    pub performance_snapshot_interval_ms: u64,
    pub min_tick_time_ms: u64,
    pub frontend_sync_interval_ms: u64,
    pub target_batch_eval_ms: f64,
    pub max_batch_size: usize,
    pub queue: SamplerQueueConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SamplerRollingState {
    eval_ms_per_sample: RollingMetric,
    eval_ms_per_batch: RollingMetric,
    sampler_produce_ms_per_sample: RollingMetric,
    sampler_ingest_ms_per_sample: RollingMetric,
    produced_batches_per_tick: RollingMetric,
    sampler_tick_ms: RollingMetric,
    reclaim_ms: RollingMetric,
    queue_counts_ms: RollingMetric,
    active_evaluator_count_ms: RollingMetric,
    completed_merge_ingest_ms: RollingMetric,
    aggregation_flush_ms: RollingMetric,
    completed_delete_ms: RollingMetric,
    produce_ms: RollingMetric,
    progress_sync_ms: RollingMetric,
    performance_sync_ms: RollingMetric,
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
    completed_samples_per_second: f64,
    pending_persisted_completed_batches: i32,
    batch_size_current: usize,
    observable_checkpoint_state: ObservableCheckpointState,
    rolling: SamplerRollingState,
}

impl Default for SamplerRuntimeState {
    fn default() -> Self {
        Self {
            produced_batches_total: 0,
            produced_samples_total: 0,
            ingested_batches_total: 0,
            ingested_samples_total: 0,
            completed_samples_per_second: 0.0,
            pending_persisted_completed_batches: 0,
            batch_size_current: 0,
            observable_checkpoint_state: ObservableCheckpointState::NeedsInitialRoundTrip,
            rolling: SamplerRollingState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorCheckpoint {
    pub task_id: i64,
    pub sampler_snapshot: SamplerAggregatorSnapshot,
    pub observable_state: ObservableState,
    runtime_state: SamplerRuntimeState,
    queue: SamplerQueueCheckpoint,
}

impl SamplerAggregatorCheckpoint {
    pub fn reduced_carryover_batch_size(&self, max_batch_size: usize) -> usize {
        let reduced = ((self.runtime_state.batch_size_current as f64) * MAX_BATCH_SIZE_DOWN_FACTOR)
            .round() as usize;
        reduced.clamp(MIN_BATCH_SIZE, max_batch_size)
    }
}

impl SamplerRuntimeState {
    fn to_runtime_metrics(
        &self,
        queue: crate::core::SamplerQueueRuntimeMetrics,
    ) -> SamplerRuntimeMetrics {
        SamplerRuntimeMetrics {
            produced_batches_total: self.produced_batches_total,
            produced_samples_total: self.produced_samples_total,
            ingested_batches_total: self.ingested_batches_total,
            ingested_samples_total: self.ingested_samples_total,
            completed_samples_per_second: self.completed_samples_per_second,
            batch_size_current: self.batch_size_current,
            sampler: SamplerWorkRollingAverages {
                eval_ms_per_sample: RollingMetricSnapshot::from(&self.rolling.eval_ms_per_sample),
                eval_ms_per_batch: RollingMetricSnapshot::from(&self.rolling.eval_ms_per_batch),
                sampler_produce_ms_per_sample: RollingMetricSnapshot::from(
                    &self.rolling.sampler_produce_ms_per_sample,
                ),
                sampler_ingest_ms_per_sample: RollingMetricSnapshot::from(
                    &self.rolling.sampler_ingest_ms_per_sample,
                ),
                produced_batches_per_tick: RollingMetricSnapshot::from(
                    &self.rolling.produced_batches_per_tick,
                ),
                sampler_tick_ms: RollingMetricSnapshot::from(&self.rolling.sampler_tick_ms),
                reclaim_ms: RollingMetricSnapshot::from(&self.rolling.reclaim_ms),
                queue_counts_ms: RollingMetricSnapshot::from(&self.rolling.queue_counts_ms),
                active_evaluator_count_ms: RollingMetricSnapshot::from(
                    &self.rolling.active_evaluator_count_ms,
                ),
                completed_merge_ingest_ms: RollingMetricSnapshot::from(
                    &self.rolling.completed_merge_ingest_ms,
                ),
                aggregation_flush_ms: RollingMetricSnapshot::from(
                    &self.rolling.aggregation_flush_ms,
                ),
                completed_delete_ms: RollingMetricSnapshot::from(&self.rolling.completed_delete_ms),
                produce_ms: RollingMetricSnapshot::from(&self.rolling.produce_ms),
                progress_sync_ms: RollingMetricSnapshot::from(&self.rolling.progress_sync_ms),
                performance_sync_ms: RollingMetricSnapshot::from(&self.rolling.performance_sync_ms),
            },
            queue,
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
    params: SamplerAggregatorRunnerParams,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
    performance_snapshot_interval: Duration,
    frontend_sync_interval: Duration,
    last_snapshot_at: Instant,
    last_frontend_sync_at: Instant,
    last_progress_sync_at: Instant,
    last_reclaim_at: Instant,
    last_completed_cleanup_at: Instant,
    runtime_state: SamplerRuntimeState,
    last_performance_completed_samples: i64,
    queue: SamplerQueue<S>,
}

impl<S> SamplerAggregatorRunner<S>
where
    S: SamplerWorkerStore + Clone + Send + Sync + 'static,
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
        resume_snapshot: Option<SamplerAggregatorCheckpoint>,
        run_progress: Option<RunSampleProgress>,
    ) -> Self {
        let mut runtime_state;
        let queue_checkpoint;
        if let Some(snapshot) = resume_snapshot {
            runtime_state = snapshot.runtime_state.clone();
            queue_checkpoint = snapshot.queue.clone();
        } else {
            runtime_state = SamplerRuntimeState {
                batch_size_current: initial_batch_size.clamp(MIN_BATCH_SIZE, params.max_batch_size),
                ..SamplerRuntimeState::default()
            };
            queue_checkpoint = SamplerQueueCheckpoint::default();
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
        let frontend_sync_interval = Duration::from_millis(params.frontend_sync_interval_ms);
        let now = Instant::now();
        let task_id = task.id;
        let requires_training_values = sampler_config.requires_training();
        let queue = SamplerQueue::new(
            store.clone(),
            run_id,
            task_id,
            requires_training_values,
            params.queue.clone(),
            queue_checkpoint,
        );

        Self {
            run_id,
            node_name: node_name.into(),
            task,
            sampler,
            observable_state,
            sampler_config,
            batch_transforms,
            store,
            params,
            nr_produced_samples,
            nr_completed_samples,
            performance_snapshot_interval,
            frontend_sync_interval,
            last_snapshot_at: now,
            last_frontend_sync_at: now,
            last_progress_sync_at: now,
            last_reclaim_at: now.checked_sub(RECLAIM_INTERVAL).unwrap_or(now),
            last_completed_cleanup_at: now.checked_sub(COMPLETED_CLEANUP_INTERVAL).unwrap_or(now),
            runtime_state,
            last_performance_completed_samples: nr_completed_samples,
            queue,
        }
    }

    pub fn params(&self) -> &SamplerAggregatorRunnerParams {
        &self.params
    }

    async fn flush_queue(&mut self) -> Result<(), RunnerError> {
        self.queue.flush().await?;
        Ok(())
    }

    fn tune_batch_size(&mut self) {
        let Some(eval_ms_per_sample) = self.runtime_state.rolling.eval_ms_per_sample.value() else {
            return;
        };
        if self.params.target_batch_eval_ms <= 0.0 || !self.params.target_batch_eval_ms.is_finite()
        {
            return;
        }
        if eval_ms_per_sample <= 0.0 {
            return;
        }
        let current_eval_batch_ms =
            eval_ms_per_sample * self.runtime_state.batch_size_current as f64;
        let raw_ratio = self.params.target_batch_eval_ms / current_eval_batch_ms;
        let ratio = raw_ratio.clamp(MAX_BATCH_SIZE_DOWN_FACTOR, MAX_BATCH_SIZE_UP_FACTOR);

        let next = ((self.runtime_state.batch_size_current as f64) * ratio).round() as usize;
        self.runtime_state.batch_size_current =
            next.clamp(MIN_BATCH_SIZE, self.params.max_batch_size);
    }

    async fn current_runner_diagnostics(&self) -> Result<JsonValue, RunnerError> {
        let queue_counts = self.queue.queue_counts_with_local_buffer(
            self.store
                .get_batch_queue_counts(self.run_id, self.queue.last_completed_batch_id())
                .await?,
        );
        let active_evaluator_count = self.active_evaluator_count().await?;
        let target_pending_batches = self.queue.target_pending_batches(active_evaluator_count);
        let queue_runtime = self.queue.runtime_metrics();
        Ok(json!({
            "active_evaluator_count": active_evaluator_count,
            "pending_batches": queue_counts.pending,
            "claimed_batches": queue_counts.claimed,
            "completed_batches": queue_counts.completed,
            "open_batches": queue_counts.open(),
            "queue_buffer": self.params.queue.queue_buffer,
            "target_pending_batches": target_pending_batches,
            "pending_shortfall": target_pending_batches
                .map(|target| (target as i64).saturating_sub(queue_counts.pending as i64)),
            "last_completed_batch_id": self.queue.last_completed_batch_id(),
            "local_pending_batches": queue_runtime.local_pending_batches,
            "local_inflight_insert_batches": queue_runtime.local_inflight_insert_batches,
            "local_ready_processed_batches": queue_runtime.local_ready_processed_batches,
            "observable_checkpoint_state": match self.runtime_state.observable_checkpoint_state {
                ObservableCheckpointState::NeedsInitialRoundTrip => "needs_initial_round_trip",
                ObservableCheckpointState::WaitingForInitialRoundTrip => "waiting_for_initial_round_trip",
                ObservableCheckpointState::Ready => "ready",
            },
            "training_samples_remaining": self.sampler.training_samples_remaining(),
        }))
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
        let tick_started = Instant::now();
        let completed = self.queue.get_processed().await?;
        self.cleanup_consumed_completed_batches(false).await?;
        if self.last_reclaim_at.elapsed() >= RECLAIM_INTERVAL {
            let reclaim_started = Instant::now();
            self.store.reclaim_abandoned_batches(self.run_id).await?;
            observe_duration_ms(
                &mut self.runtime_state.rolling.reclaim_ms,
                reclaim_started.elapsed(),
            );
            self.last_reclaim_at = Instant::now();
        }

        let queue_snapshot_started = Instant::now();
        let queue_before_tick = self.queue.queue_counts_with_local_buffer(
            self.store
                .get_batch_queue_counts(self.run_id, self.queue.last_completed_batch_id())
                .await?,
        );
        observe_duration_ms(
            &mut self.runtime_state.rolling.queue_counts_ms,
            queue_snapshot_started.elapsed(),
        );
        self.tune_batch_size();

        let active_evaluator_started = Instant::now();
        let active_evaluator_count = self.active_evaluator_count().await?;
        observe_duration_ms(
            &mut self.runtime_state.rolling.active_evaluator_count_ms,
            active_evaluator_started.elapsed(),
        );

        let completed_batches = self.process_completed_batches(completed).await?;
        let queue_before_produce = crate::core::BatchQueueCounts {
            pending: queue_before_tick.pending,
            claimed: queue_before_tick.claimed,
            completed: queue_before_tick
                .completed
                .saturating_sub(completed_batches as i64),
        };

        let produce_started = Instant::now();
        let produced_batches = self
            .produce(queue_before_produce, active_evaluator_count)
            .await?;
        observe_duration_ms(
            &mut self.runtime_state.rolling.produce_ms,
            produce_started.elapsed(),
        );

        let progress_sync_started = Instant::now();
        self.flush_progress_sync(false).await?;
        observe_duration_ms(
            &mut self.runtime_state.rolling.progress_sync_ms,
            progress_sync_started.elapsed(),
        );

        let performance_sync_started = Instant::now();
        self.flush_performance_snapshot(false).await?;
        observe_duration_ms(
            &mut self.runtime_state.rolling.performance_sync_ms,
            performance_sync_started.elapsed(),
        );

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
        observe_duration_ms(
            &mut self.runtime_state.rolling.sampler_tick_ms,
            tick_started.elapsed(),
        );
        Ok(self.task.task.nr_expected_samples().is_some_and(|target| {
            self.task.nr_completed_samples >= target && open_batch_count == 0
        }))
    }

    async fn persist_stage_state_with_queue_empty(
        &mut self,
        queue_empty: bool,
    ) -> Result<(), RunnerError> {
        self.store
            .save_run_stage_snapshot(&RunStageSnapshot {
                id: None,
                run_id: self.run_id,
                task_id: Some(self.task.id),
                name: self.task.name.clone(),
                sequence_nr: Some(self.task.sequence_nr),
                queue_empty,
                sampler_snapshot: Some(self.sampler.snapshot().map_err(RunnerError::Engine)?),
                observable_state: Some(self.observable_state.clone()),
                sampler_aggregator: Some(self.sampler_config.clone()),
                batch_transforms: self.batch_transforms.clone(),
            })
            .await?;
        Ok(())
    }

    async fn persist_sampler_checkpoint(&mut self) -> Result<(), RunnerError> {
        let checkpoint = SamplerAggregatorCheckpoint {
            task_id: self.task.id,
            sampler_snapshot: self.sampler.snapshot().map_err(RunnerError::Engine)?,
            observable_state: self.observable_state.clone(),
            runtime_state: self.runtime_state.clone(),
            queue: self.queue.checkpoint(),
        };
        self.store
            .save_sampler_checkpoint(self.run_id, &checkpoint)
            .await?;
        Ok(())
    }

    async fn drain_evaluator_work_on_stop(&mut self) -> Result<(), RunnerError> {
        self.flush_queue().await?;
        loop {
            let completed = self.queue.get_processed_blocking().await?;
            self.process_completed_batches(completed).await?;

            let queue_counts = self.queue.queue_counts_with_local_buffer(
                self.store
                    .get_batch_queue_counts(self.run_id, self.queue.last_completed_batch_id())
                    .await?,
            );
            if queue_counts.claimed <= 0
                && queue_counts.completed <= 0
                && self.queue.local_work_drained()
            {
                break;
            }

            sleep(Duration::from_millis(25)).await;
        }

        let completed = self.queue.get_processed_blocking().await?;
        self.process_completed_batches(completed).await?;
        Ok(())
    }

    pub async fn persist_state(&mut self) -> Result<(), RunnerError> {
        self.flush_queue().await?;
        self.drain_evaluator_work_on_stop().await?;
        self.cleanup_consumed_completed_batches(true).await?;
        self.flush_aggregation(true).await?;
        self.flush_performance_snapshot(true).await?;
        self.flush_progress_sync(true).await?;
        let queue_empty = self.store.get_open_batch_count(self.run_id).await? <= 0;
        self.persist_stage_state_with_queue_empty(queue_empty)
            .await?;
        self.persist_sampler_checkpoint().await
    }

    async fn flush_aggregation(&mut self, force: bool) -> Result<(), RunnerError> {
        let due = force
            || self.frontend_sync_interval.is_zero()
            || self.last_frontend_sync_at.elapsed() >= self.frontend_sync_interval;
        if !due {
            return Ok(());
        }

        let persist_snapshot = force;
        let current_observable = self
            .observable_state
            .to_json()
            .map_err(RunnerError::Engine)?;
        let snapshot = if persist_snapshot {
            Some(
                self.observable_state
                    .to_persistent_json()
                    .map_err(RunnerError::Engine)?,
            )
        } else {
            None
        };
        let aggregation_flush_started = Instant::now();
        self.store
            .save_aggregation(
                self.run_id,
                self.task.id,
                &current_observable,
                snapshot.as_ref(),
                self.runtime_state.pending_persisted_completed_batches,
            )
            .await?;
        observe_duration_ms(
            &mut self.runtime_state.rolling.aggregation_flush_ms,
            aggregation_flush_started.elapsed(),
        );
        self.runtime_state.pending_persisted_completed_batches = 0;
        self.last_frontend_sync_at = Instant::now();
        Ok(())
    }

    async fn cleanup_consumed_completed_batches(&mut self, force: bool) -> Result<(), RunnerError> {
        let Some(up_to_batch_id) = self.queue.last_completed_batch_id() else {
            return Ok(());
        };

        let due = force || self.last_completed_cleanup_at.elapsed() >= COMPLETED_CLEANUP_INTERVAL;
        if !due {
            return Ok(());
        }

        let cleanup_started = Instant::now();
        let mut deleted_any = false;
        loop {
            let deleted = self
                .store
                .cleanup_consumed_completed_batches(
                    self.run_id,
                    up_to_batch_id,
                    COMPLETED_CLEANUP_BATCH_LIMIT,
                )
                .await?;
            if deleted > 0 {
                deleted_any = true;
            }
            if !force || deleted < COMPLETED_CLEANUP_BATCH_LIMIT as u64 {
                break;
            }
        }
        if deleted_any {
            observe_duration_ms(
                &mut self.runtime_state.rolling.completed_delete_ms,
                cleanup_started.elapsed(),
            );
        }
        self.last_completed_cleanup_at = Instant::now();
        Ok(())
    }

    pub async fn complete_task(&mut self) -> Result<(), RunnerError> {
        self.flush_queue().await?;
        self.cleanup_consumed_completed_batches(true).await?;
        self.flush_aggregation(true).await?;
        self.flush_performance_snapshot(true).await?;
        self.flush_progress_sync(true).await?;
        self.persist_stage_state_with_queue_empty(true).await?;
        self.store.complete_run_task(self.task.id).await?;
        Ok(())
    }

    pub async fn fail_task(&mut self, reason: &str) -> Result<(), RunnerError> {
        self.store.fail_run_task(self.task.id, reason).await?;
        Ok(())
    }

    async fn process_completed_batches(
        &mut self,
        completed: Vec<crate::core::CompletedBatch>,
    ) -> Result<usize, RunnerError> {
        if completed.is_empty() {
            return Ok(0);
        }

        let completed_merge_ingest_started = Instant::now();
        let mut completed_samples_delta = 0_i64;
        for batch in &completed {
            let batch_samples = batch.batch_size;
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
        if completed_samples_delta > 0 {
            self.runtime_state.observable_checkpoint_state = ObservableCheckpointState::Ready;
        }
        observe_duration_ms(
            &mut self.runtime_state.rolling.completed_merge_ingest_ms,
            completed_merge_ingest_started.elapsed(),
        );
        self.flush_aggregation(false).await?;
        self.queue.mark_processed(&completed);
        Ok(completed.len())
    }

    async fn produce(
        &mut self,
        queue_before_produce: crate::core::BatchQueueCounts,
        active_evaluator_count: usize,
    ) -> Result<usize, RunnerError> {
        let observable_config = self.observable_state.config();
        let sample_plan = self.sampler.sample_plan().map_err(RunnerError::Engine)?;
        let training_samples_remaining = self.sampler.training_samples_remaining();
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
                        if self.params.queue.max_queue_size <= open_before_produce {
                            Vec::new()
                        } else {
                            let nr_samples = max_samples.unwrap_or(MIN_BATCH_SIZE);
                            if nr_samples == 0 {
                                Vec::new()
                            } else {
                                self.runtime_state.observable_checkpoint_state =
                                    ObservableCheckpointState::WaitingForInitialRoundTrip;
                                vec![nr_samples.min(MIN_BATCH_SIZE)]
                            }
                        }
                    }
                    ObservableCheckpointState::WaitingForInitialRoundTrip => {
                        if open_before_produce == 0 {
                            self.runtime_state.observable_checkpoint_state =
                                ObservableCheckpointState::NeedsInitialRoundTrip;
                        }
                        Vec::new()
                    }
                    ObservableCheckpointState::Ready => self.queue.get_sample(
                        max_samples,
                        queue_before_produce,
                        active_evaluator_count,
                        self.runtime_state.batch_size_current,
                    ),
                }
            }
        };
        if batch_plan.len() > self.params.queue.max_batches_per_tick {
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "batch plan exceeded max_batches_per_tick: planned={} max_batches_per_tick={}",
                batch_plan.len(),
                self.params.queue.max_batches_per_tick
            ))));
        }
        let mut produced = Vec::with_capacity(batch_plan.len());
        let mut produced_samples_total = 0_i64;
        for nr_samples in batch_plan {
            if nr_samples > self.params.max_batch_size {
                return Err(RunnerError::Engine(EngineError::engine(format!(
                    "batch plan exceeded max_batch_size: planned={} max_batch_size={}",
                    nr_samples, self.params.max_batch_size
                ))));
            }
            let started = Instant::now();
            let batch = self
                .sampler
                .produce_latent_batch(nr_samples)
                .map_err(RunnerError::Engine)?;
            let produce_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            let produced_samples = batch.nr_samples;
            produced_samples_total += produced_samples as i64;
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
        let produced_batches = produced.len();
        if produced_batches == 0 {
            return Ok(0);
        }
        self.runtime_state
            .rolling
            .produced_batches_per_tick
            .observe(produced_batches as f64);
        self.runtime_state.produced_batches_total += produced_batches as i64;
        self.runtime_state.produced_samples_total += produced_samples_total;
        self.nr_produced_samples += produced_samples_total;
        self.task.nr_produced_samples += produced_samples_total;
        self.queue.ingest(produced);
        Ok(produced_batches)
    }

    fn progress_sync_due(&self, force: bool) -> bool {
        force
            || self.frontend_sync_interval.is_zero()
            || self.last_progress_sync_at.elapsed() >= self.frontend_sync_interval
    }

    async fn flush_progress_sync(&mut self, force: bool) -> Result<(), RunnerError> {
        if !self.progress_sync_due(force) {
            return Ok(());
        }
        self.sync_task_progress().await?;
        self.flush_run_sample_progress().await?;
        self.last_progress_sync_at = Instant::now();
        Ok(())
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

    async fn flush_performance_snapshot(&mut self, force: bool) -> Result<(), RunnerError> {
        let due = force
            || if self.performance_snapshot_interval.is_zero() {
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
        self.runtime_state.completed_samples_per_second = completed_samples_per_second;

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
            runtime_metrics: self
                .runtime_state
                .to_runtime_metrics(self.queue.runtime_metrics()),
            engine_diagnostics,
            rss_bytes: current_rss_bytes(),
        };
        self.store
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
        self.last_performance_completed_samples = self.nr_completed_samples;
        self.last_snapshot_at = Instant::now();
        Ok(())
    }
}

fn observe_duration_ms(metric: &mut RollingMetric, duration: Duration) {
    let ms = duration.as_secs_f64() * 1000.0;
    if ms.is_finite() && ms >= 0.0 {
        metric.observe(ms);
    }
}

#[cfg(test)]
mod tests {
    use super::{SamplerAggregatorCheckpoint, SamplerRuntimeState};
    use crate::core::{LineRasterGeometry, Linspace, PlaneRasterGeometry, SamplerAggregatorConfig};
    use crate::runners::SamplerQueueCheckpoint;
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
        let snapshot = SamplerAggregatorCheckpoint {
            task_id: 1,
            sampler_snapshot: SamplerAggregatorSnapshot::NaiveMonteCarlo { raw: json!({}) },
            observable_state: crate::evaluation::ObservableState::empty_scalar(),
            runtime_state: SamplerRuntimeState {
                batch_size_current: 128,
                ..SamplerRuntimeState::default()
            },
            queue: SamplerQueueCheckpoint::default(),
        };

        assert_eq!(snapshot.reduced_carryover_batch_size(512), 32);
        assert_eq!(snapshot.reduced_carryover_batch_size(24), 24);
    }
}
