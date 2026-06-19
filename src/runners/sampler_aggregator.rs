//! Sampler task executor orchestration.
//!
//! This module owns one active sampler task at a time:
//! - restore/build the sampler and accumulator for that task
//! - enqueue latent batches
//! - fetch completed batches and pass training weights back into the sampler
//! - merge completed batch observables into the current accumulator state
//! - persist lightweight UI sync snapshots and full resume checkpoints

use crate::core::{
    AccumulatorMetricSelector, BatchTransformConfig, EngineError, EvaluatorConfig,
    RunSampleProgress, RunStageSnapshot, RunTask, SampleErrorProjection, SamplerAggregatorConfig,
    SamplerAggregatorPerformanceSnapshot, SamplerQueueTuning, SamplerRuntimeMetrics,
    SamplerWorkRollingAverages, SamplerWorkerStore, StoreError,
};
use crate::evaluation::{
    AccumulatorState, extract_accumulator_metric_with_runtime, relative_error,
};
use crate::runners::process_memory::current_rss_bytes;
use crate::runners::queue::QueueUtilizationSnapshot;
use crate::runners::rolling_metric::RollingMetric;
use crate::runners::window_metric::WindowMetric;
use crate::runners::{QueueTickResult, SamplerQueue, SamplerQueueCheckpoint, SamplerQueueConfig};
use crate::sampling::DiscreteSubspace;
use crate::sampling::{SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use thiserror::Error;

const MIN_BATCH_SIZE: usize = 16;
const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.25;
const COMPLETED_SAMPLES_PER_SECOND_EWMA_ALPHA: f64 = 0.2;
const ETA_COMPLETED_SAMPLES_PER_SECOND_EWMA_ALPHA: f64 = 0.02;
const TASK_CONFIG_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerAggregatorRunnerParams {
    pub performance_snapshot_interval_ms: u64,
    pub min_tick_time_ms: u64,
    pub frontend_sync_interval_ms: u64,
    #[serde(default = "default_sampler_db_pool_size")]
    pub db_pool_size: u32,
    pub queue: SamplerQueueConfig,
}

fn default_sampler_db_pool_size() -> u32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct SamplerRollingState {
    eval_ms_per_sample: RollingMetric,
    eval_ms_per_batch: RollingMetric,
    training_ingest_ms_per_sample: RollingMetric,
    completed_training_ingest_ms: RollingMetric,
    produce_ms_per_sample: RollingMetric,
    reclaim_ms: RollingMetric,
    queue_counts_ms: RollingMetric,
    completed_merge_ingest_ms: RollingMetric,
    persist_accumulator_ms: RollingMetric,
    completed_delete_ms: RollingMetric,
    produce_ms: RollingMetric,
    progress_sync_ms: RollingMetric,
    performance_sync_ms: RollingMetric,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct SamplerWindowState {
    eval_ms_per_sample: WindowMetric,
    eval_ms_per_batch: WindowMetric,
    training_ingest_ms_per_sample: WindowMetric,
    completed_training_ingest_ms: WindowMetric,
    produce_ms_per_sample: WindowMetric,
    reclaim_ms: WindowMetric,
    queue_counts_ms: WindowMetric,
    completed_merge_ingest_ms: WindowMetric,
    persist_accumulator_ms: WindowMetric,
    completed_delete_ms: WindowMetric,
    produce_ms: WindowMetric,
    progress_sync_ms: WindowMetric,
    performance_sync_ms: WindowMetric,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
enum AccumulatorCheckpointState {
    #[default]
    NeedsInitialRoundTrip,
    WaitingForInitialRoundTrip,
    Ready,
}

enum ProduceDecision {
    None,
    InitialRoundTrip(usize),
    PlannedByQueue(Option<usize>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SamplerRuntimeState {
    produced_batches_total: i64,
    produced_samples_total: i64,
    ingested_batches_total: i64,
    ingested_samples_total: i64,
    completed_samples_per_second: f64,
    #[serde(default)]
    eta_completed_samples_per_second: f64,
    #[serde(default)]
    eta_seconds_smoothed: Option<f64>,
    #[serde(default)]
    sampler_uptime_ms_accumulated: f64,
    #[serde(default)]
    initial_round_trip_snapshot_pending: bool,
    pending_persisted_completed_batches: i32,
    batch_size_current: usize,
    #[serde(default)]
    sampler_tick_busy_ratio: Option<f64>,
    accumulator_checkpoint_state: AccumulatorCheckpointState,
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
            eta_completed_samples_per_second: 0.0,
            eta_seconds_smoothed: None,
            sampler_uptime_ms_accumulated: 0.0,
            initial_round_trip_snapshot_pending: false,
            pending_persisted_completed_batches: 0,
            batch_size_current: 0,
            sampler_tick_busy_ratio: None,
            accumulator_checkpoint_state: AccumulatorCheckpointState::NeedsInitialRoundTrip,
            rolling: SamplerRollingState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorCheckpoint {
    pub task_id: i64,
    pub sampler_snapshot: SamplerAggregatorSnapshot,
    pub observable_state: AccumulatorState,
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
        sampler: SamplerWorkRollingAverages,
        queue: crate::core::SamplerQueueRuntimeMetrics,
        completed_samples_total: i64,
        sampler_uptime_ms: f64,
        evaluator_fleet: EvaluatorFleetSnapshot,
    ) -> SamplerRuntimeMetrics {
        SamplerRuntimeMetrics {
            produced_batches_total: self.produced_batches_total,
            produced_samples_total: self.produced_samples_total,
            ingested_batches_total: self.ingested_batches_total,
            ingested_samples_total: self.ingested_samples_total,
            completed_samples_total,
            sampler_uptime_ms,
            completed_samples_per_second: self.completed_samples_per_second,
            eta_completed_samples_per_second: self.eta_completed_samples_per_second,
            eta_seconds_smoothed: self.eta_seconds_smoothed,
            batch_size_current: self.batch_size_current,
            sampler_tick_busy_ratio: self.sampler_tick_busy_ratio,
            avg_evaluator_utilization: evaluator_fleet.avg_utilization,
            active_evaluator_count: Some(evaluator_fleet.active_count),
            avg_evaluator_rss_bytes: evaluator_fleet.avg_rss_bytes,
            total_evaluator_rss_bytes: evaluator_fleet.total_rss_bytes,
            sampler,
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
    observable_state: AccumulatorState,
    evaluator_config: EvaluatorConfig,
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
    pending_aggregation_flush: Option<PendingAggregationFlushTask>,
    runtime_state: SamplerRuntimeState,
    window_state: SamplerWindowState,
    queue: SamplerQueue<S>,
    base_queue_config: SamplerQueueConfig,
    last_task_config_refresh_at: Instant,
    utilization_window_started_at: Instant,
    sync_tick_busy_time: Duration,
    sampler_uptime_started_at: Instant,
}

struct CompletedIngestStats {
    completed_batches: usize,
    completed_samples_delta: i64,
}

fn collect_discrete_pdf_subspaces(
    state: &AccumulatorState,
    config: &crate::core::DiscreteProjectionConfig,
    bin_limit: usize,
) -> Result<Vec<(String, Vec<i64>, DiscreteSubspace)>, EngineError> {
    let mut out = Vec::new();
    match state {
        AccumulatorState::Scalar(state) => {
            collect_discrete_pdf_subspaces_from_bins(
                &state.discrete_bins,
                config,
                bin_limit,
                None,
                &mut out,
            )?;
        }
        AccumulatorState::Vector(state) => {
            for component in state
                .components
                .iter()
                .chain(std::iter::once(&state.projection))
            {
                if !discrete_projection_includes_stream(config, &component.name) {
                    continue;
                }
                collect_discrete_pdf_subspaces_from_bins(
                    &component.state.discrete_bins,
                    config,
                    bin_limit,
                    Some(&component.name),
                    &mut out,
                )?;
            }
        }
        _ => {}
    }
    Ok(out)
}

fn discrete_projection_includes_stream(
    config: &crate::core::DiscreteProjectionConfig,
    stream: &str,
) -> bool {
    config.streams.is_empty() || config.streams.iter().any(|candidate| candidate == stream)
}

fn collect_discrete_pdf_subspaces_from_bins(
    bins: &BTreeMap<String, crate::evaluation::accumulator::DiscreteProjectionBinState>,
    config: &crate::core::DiscreteProjectionConfig,
    bin_limit: usize,
    component_name: Option<&str>,
    out: &mut Vec<(String, Vec<i64>, DiscreteSubspace)>,
) -> Result<(), EngineError> {
    for item in &config.items {
        let projection_name = component_name
            .map(|component| format!("{}.{}", item.name, component))
            .unwrap_or_else(|| item.name.clone());
        let mut seen = BTreeSet::<Vec<i64>>::new();
        for bin in bins.values() {
            if !discrete_matches_fixed_dims(&bin.discrete, item)? {
                continue;
            }
            let Some(key) = discrete_projection_key(&bin.discrete, item) else {
                continue;
            };
            if !seen.insert(key.clone()) {
                continue;
            }
            if seen.len() > bin_limit {
                break;
            }
            let mut fixed_dims = BTreeMap::new();
            for (raw_dim, value) in &item.fixed_dims {
                let dim = raw_dim.parse::<usize>().map_err(|_| {
                    EngineError::engine(format!(
                        "discrete projection '{}' fixed dimension '{}' is not a non-negative integer dimension index",
                        item.name, raw_dim
                    ))
                })?;
                fixed_dims.insert(dim, *value);
            }
            for (dim, value) in item.dims.iter().zip(key.iter()) {
                fixed_dims.insert(*dim, *value);
            }
            out.push((
                projection_name.clone(),
                key,
                DiscreteSubspace { fixed_dims },
            ));
        }
    }
    Ok(())
}

fn discrete_matches_fixed_dims(
    discrete: &[i64],
    item: &crate::core::NamedDiscreteProjection,
) -> Result<bool, EngineError> {
    for (raw_dim, fixed_value) in &item.fixed_dims {
        let dim = raw_dim.parse::<usize>().map_err(|_| {
            EngineError::engine(format!(
                "discrete projection '{}' fixed dimension '{}' is not a non-negative integer dimension index",
                item.name, raw_dim
            ))
        })?;
        let Some(actual) = discrete.get(dim) else {
            return Ok(false);
        };
        if actual != fixed_value {
            return Ok(false);
        }
    }
    Ok(true)
}

fn discrete_projection_key(
    discrete: &[i64],
    item: &crate::core::NamedDiscreteProjection,
) -> Option<Vec<i64>> {
    item.dims
        .iter()
        .map(|dim| discrete.get(*dim).copied())
        .collect()
}

fn discrete_key_label(key: &[i64]) -> String {
    serde_json::to_string(key).unwrap_or_else(|_| "[]".to_string())
}

fn metric_selector_label(selector: &AccumulatorMetricSelector) -> String {
    match &selector.component {
        Some(component) => format!("{component}.{:?}", selector.name),
        None => format!("{:?}", selector.name),
    }
}

struct PendingAggregationFlushTask {
    started_at: Instant,
    flushed_completed_batches: i32,
    cleared_initial_round_trip: bool,
    handle: tokio::task::JoinHandle<Result<(), StoreError>>,
}

#[derive(Debug, Clone, Copy)]
struct ProjectedEstimate {
    value: f64,
    error: f64,
}

#[derive(Debug, Clone, Copy)]
struct StopConditionStatus {
    reached: bool,
    max_samples_reached: bool,
    absolute_error_reached: bool,
    relative_error_reached: bool,
    min_samples_reached: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct EvaluatorFleetSnapshot {
    active_count: usize,
    avg_utilization: Option<f64>,
    avg_rss_bytes: Option<i64>,
    total_rss_bytes: Option<i64>,
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
        observable_state: AccumulatorState,
        evaluator_config: EvaluatorConfig,
        sampler_config: SamplerAggregatorConfig,
        batch_transforms: Vec<BatchTransformConfig>,
        params: SamplerAggregatorRunnerParams,
        base_queue_config: SamplerQueueConfig,
        initial_batch_size: usize,
        run_progress: RunSampleProgress,
        resume_snapshot: Option<SamplerAggregatorCheckpoint>,
    ) -> Self {
        let mut runtime_state;
        let queue_checkpoint;
        let max_batch_size = params.queue.max_batch_size.max(MIN_BATCH_SIZE);
        let has_resume_snapshot = resume_snapshot.is_some();
        if let Some(snapshot) = resume_snapshot {
            runtime_state = snapshot.runtime_state.clone();
            queue_checkpoint = snapshot.queue.clone();
        } else {
            runtime_state = SamplerRuntimeState {
                batch_size_current: initial_batch_size.clamp(MIN_BATCH_SIZE, max_batch_size),
                ..SamplerRuntimeState::default()
            };
            queue_checkpoint = SamplerQueueCheckpoint::default();
        }
        runtime_state.batch_size_current = runtime_state
            .batch_size_current
            .clamp(MIN_BATCH_SIZE, max_batch_size);
        runtime_state.sampler_uptime_ms_accumulated = runtime_state
            .sampler_uptime_ms_accumulated
            .max(run_progress.sampler_runner_uptime_ms);

        let nr_produced_samples = task.nr_produced_samples;
        let nr_completed_samples = task.nr_completed_samples;
        if !has_resume_snapshot && nr_completed_samples > 0 {
            runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
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
            runtime_state.batch_size_current,
        );
        runtime_state.batch_size_current = queue.current_batch_size();

        Self {
            run_id,
            node_name: node_name.into(),
            task,
            sampler,
            observable_state,
            evaluator_config,
            sampler_config,
            batch_transforms,
            store,
            params,
            nr_produced_samples: run_progress.nr_produced_samples.max(nr_produced_samples),
            nr_completed_samples: run_progress.nr_completed_samples.max(nr_completed_samples),
            performance_snapshot_interval,
            frontend_sync_interval,
            last_snapshot_at: now,
            last_frontend_sync_at: now,
            last_progress_sync_at: now,
            pending_aggregation_flush: None,
            runtime_state,
            window_state: SamplerWindowState::default(),
            queue,
            base_queue_config,
            last_task_config_refresh_at: now
                .checked_sub(TASK_CONFIG_REFRESH_INTERVAL)
                .unwrap_or(now),
            utilization_window_started_at: now,
            sync_tick_busy_time: Duration::ZERO,
            sampler_uptime_started_at: now,
        }
    }

    fn current_sampler_uptime_ms(&self) -> f64 {
        self.runtime_state.sampler_uptime_ms_accumulated
            + self
                .sampler_uptime_started_at
                .elapsed()
                .as_secs_f64()
                .mul_add(1000.0, 0.0)
    }

    fn checkpoint_sampler_uptime_now(&mut self) -> f64 {
        let uptime_ms = self.current_sampler_uptime_ms();
        self.runtime_state.sampler_uptime_ms_accumulated = uptime_ms;
        self.sampler_uptime_started_at = Instant::now();
        uptime_ms
    }

    pub fn params(&self) -> &SamplerAggregatorRunnerParams {
        &self.params
    }

    fn effective_queue_config_for_tuning(
        &self,
        queue_tuning: Option<&SamplerQueueTuning>,
    ) -> SamplerQueueConfig {
        let mut effective = self.base_queue_config.clone();
        if let Some(queue_tuning) = queue_tuning {
            effective.apply_tuning(queue_tuning);
        }
        effective
    }

    async fn refresh_live_queue_tuning(&mut self) -> Result<(), RunnerError> {
        if self.last_task_config_refresh_at.elapsed() < TASK_CONFIG_REFRESH_INTERVAL {
            return Ok(());
        }
        self.last_task_config_refresh_at = Instant::now();
        let Some(active_task) = self.store.load_active_run_task(self.run_id).await? else {
            return Ok(());
        };
        if active_task.id != self.task.id {
            return Ok(());
        }
        let next_tuning = active_task.task.sample_queue_tuning().cloned();
        let current_tuning = self.task.task.sample_queue_tuning().cloned();
        if next_tuning == current_tuning {
            return Ok(());
        }
        self.task
            .task
            .set_sample_queue_tuning(next_tuning.clone())
            .map_err(|err| RunnerError::Engine(EngineError::invalid_input(err)))?;
        let next_queue_config = self.effective_queue_config_for_tuning(next_tuning.as_ref());
        self.params.queue = next_queue_config.clone();
        self.queue.apply_config(next_queue_config);
        self.runtime_state.batch_size_current = self.queue.current_batch_size();
        Ok(())
    }

    fn current_runner_diagnostics(&mut self) -> JsonValue {
        let diagnostics_snapshot = self.queue.diagnostics_snapshot();
        let queue_counts = diagnostics_snapshot.map(|snapshot| snapshot.queue_counts);
        let active_evaluator_count =
            diagnostics_snapshot.and_then(|snapshot| snapshot.active_evaluator_count);
        let target_pending_batches =
            active_evaluator_count.and_then(|count| self.queue.target_pending_batches(count));
        let target_pending_low_batches =
            active_evaluator_count.and_then(|count| self.queue.target_pending_low_batches(count));
        let target_pending_high_batches =
            active_evaluator_count.and_then(|count| self.queue.target_pending_high_batches(count));
        let target_local_pending_batches =
            active_evaluator_count.and_then(|count| self.queue.target_local_pending_batches(count));
        let target_local_pending_high_batches = active_evaluator_count
            .and_then(|count| self.queue.target_local_pending_high_batches(count));
        let queue_runtime = self.queue.runtime_metrics();
        let pending_shortfall = target_pending_high_batches
            .zip(queue_runtime.db_pending_batches)
            .map(|(target, pending)| (target as i64).saturating_sub(pending.max(0)));
        json!({
            "active_evaluator_count": active_evaluator_count,
            "target_batch_eval_ms": self.params.queue.target_batch_eval_ms,
            "batch_size_deadband_ratio": self.params.queue.batch_size_deadband_ratio,
            "batch_size_cooldown_ticks": self.params.queue.batch_size_cooldown_ticks,
            "pending_batches": queue_counts.map(|counts| counts.pending),
            "claimed_batches": queue_counts.map(|counts| counts.claimed),
            "completed_batches": queue_counts.map(|counts| counts.completed),
            "open_batches": queue_counts.map(|counts| counts.open()),
            "queue_buffer": self.params.queue.queue_buffer,
            "pending_refill_low_ratio": self.params.queue.pending_refill_low_ratio,
            "pending_refill_high_ratio": self.params.queue.pending_refill_high_ratio,
            "local_pending_buffer_multiplier": self.params.queue.local_pending_buffer_multiplier,
            "target_pending_batches": target_pending_batches,
            "target_pending_low_batches": target_pending_low_batches,
            "target_pending_high_batches": target_pending_high_batches,
            "target_local_pending_batches": target_local_pending_batches,
            "target_local_pending_high_batches": target_local_pending_high_batches,
            "pending_shortfall": pending_shortfall,
            "last_completed_batch_id": self.queue.last_completed_batch_id(),
            "db_pending_batches": queue_runtime.db_pending_batches,
            "db_claimed_batches": queue_runtime.db_claimed_batches,
            "db_completed_batches": queue_runtime.db_completed_batches,
            "local_pending_batches": queue_runtime.local_pending_batches,
            "local_inflight_insert_tasks": queue_runtime.local_inflight_insert_tasks,
            "local_inflight_insert_batches": queue_runtime.local_inflight_insert_batches,
            "local_ready_processed_batches": queue_runtime.local_ready_processed_batches,
            "accumulator_checkpoint_state": match self.runtime_state.accumulator_checkpoint_state {
                AccumulatorCheckpointState::NeedsInitialRoundTrip => "needs_initial_round_trip",
                AccumulatorCheckpointState::WaitingForInitialRoundTrip => "waiting_for_initial_round_trip",
                AccumulatorCheckpointState::Ready => "ready",
            },
            "training_samples_remaining": self.sampler.training_samples_remaining(),
        })
    }

    fn take_sampler_window_snapshot(&mut self) -> SamplerWorkRollingAverages {
        SamplerWorkRollingAverages {
            eval_ms_per_sample: self.window_state.eval_ms_per_sample.snapshot_and_reset(),
            eval_ms_per_batch: self.window_state.eval_ms_per_batch.snapshot_and_reset(),
            training_ingest_ms_per_sample: self
                .window_state
                .training_ingest_ms_per_sample
                .snapshot_and_reset(),
            completed_training_ingest_ms: self
                .window_state
                .completed_training_ingest_ms
                .snapshot_and_reset(),
            produce_ms_per_sample: self.window_state.produce_ms_per_sample.snapshot_and_reset(),
            reclaim_ms: self.window_state.reclaim_ms.snapshot_and_reset(),
            queue_counts_ms: self.window_state.queue_counts_ms.snapshot_and_reset(),
            completed_merge_ingest_ms: self
                .window_state
                .completed_merge_ingest_ms
                .snapshot_and_reset(),
            persist_accumulator_ms: self
                .window_state
                .persist_accumulator_ms
                .snapshot_and_reset(),
            completed_delete_ms: self.window_state.completed_delete_ms.snapshot_and_reset(),
            produce_ms: self.window_state.produce_ms.snapshot_and_reset(),
            progress_sync_ms: self.window_state.progress_sync_ms.snapshot_and_reset(),
            performance_sync_ms: self.window_state.performance_sync_ms.snapshot_and_reset(),
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
        let Some(target) = self.task.task.sample_stop_condition().map_or_else(
            || self.task.task.nr_expected_samples(),
            |condition| condition.max_samples,
        ) else {
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

    fn effective_stop_projection(&self) -> SampleErrorProjection {
        self.task
            .task
            .sample_stop_condition()
            .and_then(|condition| condition.projection)
            .unwrap_or_else(|| match self.observable_state {
                AccumulatorState::Gammaloop(_) => SampleErrorProjection::Abs,
                _ => SampleErrorProjection::Real,
            })
    }

    fn projected_estimate_for_stop(
        &self,
        projection: SampleErrorProjection,
    ) -> Option<ProjectedEstimate> {
        match &self.observable_state {
            AccumulatorState::Scalar(state) => match projection {
                SampleErrorProjection::Real => Some(ProjectedEstimate {
                    value: state.mean(),
                    error: state.stderr(),
                }),
                SampleErrorProjection::Imag | SampleErrorProjection::Abs => None,
            },
            AccumulatorState::Vector(state) => Some(ProjectedEstimate {
                value: state.projection.state.mean(),
                error: state.projection.state.stderr(),
            }),
            AccumulatorState::Gammaloop(state) => Self::project_component_estimate(
                projection,
                (state.real_mean(), state.real_stderr()),
                (state.imag_mean(), state.imag_stderr()),
                (state.abs_mean(), state.abs_stderr()),
            ),
            AccumulatorState::Empty(_) | AccumulatorState::FullVector(_) => None,
        }
    }

    fn project_component_estimate(
        projection: SampleErrorProjection,
        real: (f64, f64),
        imag: (f64, f64),
        abs: (f64, f64),
    ) -> Option<ProjectedEstimate> {
        let (value, error) = match projection {
            SampleErrorProjection::Real => real,
            SampleErrorProjection::Imag => imag,
            SampleErrorProjection::Abs => abs,
        };
        Some(ProjectedEstimate { value, error })
    }

    fn metric_estimate_for_stop(
        &self,
        selector: &AccumulatorMetricSelector,
    ) -> Result<Option<ProjectedEstimate>, RunnerError> {
        let completed_samples_per_second =
            if self.runtime_state.completed_samples_per_second.is_finite()
                && self.runtime_state.completed_samples_per_second > 0.0
            {
                Some(self.runtime_state.completed_samples_per_second)
            } else {
                None
            };
        let Some(metric) = extract_accumulator_metric_with_runtime(
            &self.observable_state,
            selector,
            completed_samples_per_second,
        )
        .map_err(RunnerError::Engine)?
        else {
            return Ok(None);
        };
        let Some(error) = metric.uncertainty else {
            return Ok(None);
        };
        Ok(Some(ProjectedEstimate {
            value: metric.value,
            error,
        }))
    }

    fn stop_condition_status(&self) -> Result<StopConditionStatus, RunnerError> {
        if self.task.task.sample_stop_condition().is_none() {
            let reached = self
                .task
                .task
                .nr_expected_samples()
                .is_some_and(|target| self.task.nr_completed_samples >= target);
            return Ok(StopConditionStatus {
                reached,
                max_samples_reached: reached,
                absolute_error_reached: false,
                relative_error_reached: false,
                min_samples_reached: true,
            });
        }
        let stop_condition = self.task.task.sample_stop_condition().ok_or_else(|| {
            RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} missing sample stop_condition",
                self.run_id, self.task.id
            )))
        })?;
        let projected = if let Some(metric) = &stop_condition.metric {
            self.metric_estimate_for_stop(metric)?
        } else {
            let projection = self.effective_stop_projection();
            self.projected_estimate_for_stop(projection)
        };
        if (stop_condition.absolute_error.is_some() || stop_condition.relative_error.is_some())
            && projected.is_none()
        {
            let target = stop_condition
                .metric
                .as_ref()
                .map(metric_selector_label)
                .unwrap_or_else(|| {
                    let projection = self.effective_stop_projection();
                    match projection {
                        SampleErrorProjection::Real => "real",
                        SampleErrorProjection::Imag => "imag",
                        SampleErrorProjection::Abs => "abs",
                    }
                    .to_string()
                });
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} stop_condition target {} is incompatible with accumulator {} or has no uncertainty",
                self.run_id,
                self.task.id,
                target,
                self.observable_state.kind_str()
            ))));
        }

        let min_samples_reached = stop_condition
            .min_samples
            .is_none_or(|target| self.task.nr_completed_samples >= target);
        let max_samples_reached = stop_condition
            .max_samples
            .is_some_and(|target| self.task.nr_completed_samples >= target);
        let absolute_error_reached =
            stop_condition
                .absolute_error
                .zip(projected)
                .is_some_and(|(target, estimate)| {
                    min_samples_reached && estimate.error.is_finite() && estimate.error <= target
                });
        let relative_error_reached =
            stop_condition
                .relative_error
                .zip(projected)
                .is_some_and(|(target, estimate)| {
                    let relative = relative_error(estimate.value, estimate.error);
                    min_samples_reached && relative.is_finite() && relative <= target
                });
        Ok(StopConditionStatus {
            reached: max_samples_reached || absolute_error_reached || relative_error_reached,
            max_samples_reached,
            absolute_error_reached,
            relative_error_reached,
            min_samples_reached,
        })
    }

    fn estimate_eta_seconds_for_current_state(
        &self,
        completed_samples_per_second: f64,
    ) -> Option<f64> {
        let stop_condition = self.task.task.sample_stop_condition()?;
        if !completed_samples_per_second.is_finite() || completed_samples_per_second <= 0.0 {
            return None;
        }
        let projected = if let Some(metric) = &stop_condition.metric {
            self.metric_estimate_for_stop(metric).ok().flatten()
        } else {
            let projection = self.effective_stop_projection();
            self.projected_estimate_for_stop(projection)
        };
        let completed_samples = self.task.nr_completed_samples.max(0) as f64;
        let mut etas = Vec::new();
        if let Some(max_samples) = stop_condition.max_samples {
            let remaining = (max_samples as f64 - completed_samples).max(0.0);
            etas.push(remaining / completed_samples_per_second);
        }
        if let (Some(target), Some(projected)) = (stop_condition.absolute_error, projected) {
            if projected.error <= target {
                etas.push(0.0);
            } else if completed_samples > 0.0
                && projected.error.is_finite()
                && target.is_finite()
                && target > 0.0
            {
                let required_total = completed_samples * (projected.error / target).powi(2);
                let remaining = (required_total - completed_samples).max(0.0);
                etas.push(remaining / completed_samples_per_second);
            }
        }
        if let (Some(target), Some(projected)) = (stop_condition.relative_error, projected) {
            let current_relative = relative_error(projected.value, projected.error);
            if current_relative <= target {
                etas.push(0.0);
            } else if completed_samples > 0.0
                && current_relative.is_finite()
                && target.is_finite()
                && target > 0.0
            {
                let required_total = completed_samples * (current_relative / target).powi(2);
                let remaining = (required_total - completed_samples).max(0.0);
                etas.push(remaining / completed_samples_per_second);
            }
        }
        etas.into_iter().reduce(f64::min)
    }

    fn eta_smoothing_alpha(elapsed_secs: f64, eta_seconds: f64) -> f64 {
        // Continuous EMA:
        // alpha = 1 - exp(-dt / tau(eta))
        // Larger ETA -> substantially larger tau -> much stronger smoothing.
        // Smaller ETA -> smaller tau -> faster response.
        let eta_seconds = eta_seconds.max(0.0);
        let tau_seconds = (8.0 + 4.0 * eta_seconds.powf(0.6)).clamp(8.0, 86_400.0);
        let elapsed_secs = elapsed_secs.max(0.0);
        if elapsed_secs <= 0.0 {
            return 0.0;
        }
        (1.0 - (-elapsed_secs / tau_seconds).exp()).clamp(0.0, 1.0)
    }

    fn update_smoothed_eta_seconds(&mut self, elapsed: Duration) {
        let raw_eta = self.estimate_eta_seconds_for_current_state(
            self.runtime_state.eta_completed_samples_per_second,
        );
        let Some(raw_eta) = raw_eta.filter(|value| value.is_finite() && *value >= 0.0) else {
            return;
        };
        let alpha = Self::eta_smoothing_alpha(elapsed.as_secs_f64(), raw_eta);
        self.runtime_state.eta_seconds_smoothed = match self.runtime_state.eta_seconds_smoothed {
            Some(previous) if previous.is_finite() && previous > 0.0 && raw_eta > 0.0 => {
                // Log-domain smoothing damps multiplicative swings (common on ETA).
                let prev_log = previous.ln();
                let raw_log = raw_eta.ln();
                Some((prev_log + alpha * (raw_log - prev_log)).exp())
            }
            Some(previous) if previous.is_finite() && previous >= 0.0 => {
                Some(previous * (1.0 - alpha) + raw_eta * alpha)
            }
            _ => Some(raw_eta),
        };
    }

    pub fn task_id(&self) -> i64 {
        self.task.id
    }

    pub fn task_state(&self) -> &RunTask {
        &self.task
    }

    pub async fn tick(&mut self) -> Result<bool, RunnerError> {
        self.refresh_live_queue_tuning().await?;
        let tick_started = Instant::now();
        let QueueTickResult {
            completed,
            queue_counts: queue_before_tick,
            queue_snapshot_duration,
            reclaim_duration,
            completed_cleanup_duration,
        } = self.queue.tick().await?;
        if let Some(duration) = reclaim_duration {
            observe_duration_pair(
                &mut self.runtime_state.rolling.reclaim_ms,
                &mut self.window_state.reclaim_ms,
                duration,
            );
        }
        if let Some(duration) = completed_cleanup_duration {
            observe_duration_pair(
                &mut self.runtime_state.rolling.completed_delete_ms,
                &mut self.window_state.completed_delete_ms,
                duration,
            );
        }
        observe_duration_pair(
            &mut self.runtime_state.rolling.queue_counts_ms,
            &mut self.window_state.queue_counts_ms,
            queue_snapshot_duration,
        );
        let ingest_stats = self.process_completed_batches(completed).await?;
        let queue_before_produce = crate::core::BatchQueueCounts {
            pending: queue_before_tick.pending,
            claimed: queue_before_tick.claimed,
            completed: queue_before_tick
                .completed
                .saturating_sub(ingest_stats.completed_batches as i64),
            failed: queue_before_tick.failed,
        };
        self.update_completed_samples_per_second(
            tick_started.elapsed(),
            ingest_stats.completed_samples_delta,
        );
        let produce_started = Instant::now();
        let produced_batches = self.produce(queue_before_produce).await?;
        observe_duration_pair(
            &mut self.runtime_state.rolling.produce_ms,
            &mut self.window_state.produce_ms,
            produce_started.elapsed(),
        );

        self.flush_aggregation(false).await?;

        let progress_sync_started = Instant::now();
        self.flush_progress_sync(false).await?;
        observe_duration_pair(
            &mut self.runtime_state.rolling.progress_sync_ms,
            &mut self.window_state.progress_sync_ms,
            progress_sync_started.elapsed(),
        );

        let performance_sync_started = Instant::now();
        self.flush_performance_snapshot(false).await?;
        observe_duration_pair(
            &mut self.runtime_state.rolling.performance_sync_ms,
            &mut self.window_state.performance_sync_ms,
            performance_sync_started.elapsed(),
        );
        self.sync_tick_busy_time += tick_started.elapsed();
        self.check_tick_terminal_state(
            queue_before_produce,
            ingest_stats.completed_batches,
            produced_batches,
        )
    }

    fn take_sampler_tick_busy_ratio_snapshot(&mut self) -> Option<f64> {
        let now = Instant::now();
        let elapsed_secs = now
            .saturating_duration_since(self.utilization_window_started_at)
            .as_secs_f64();
        let ratio = if elapsed_secs <= 0.0 {
            None
        } else {
            Some((self.sync_tick_busy_time.as_secs_f64() / elapsed_secs).clamp(0.0, 1.0))
        };
        self.utilization_window_started_at = now;
        self.sync_tick_busy_time = Duration::ZERO;
        ratio
    }

    fn check_tick_terminal_state(
        &self,
        queue_before_produce: crate::core::BatchQueueCounts,
        completed_batches: usize,
        produced_batches: usize,
    ) -> Result<bool, RunnerError> {
        let open_batch_count = (queue_before_produce
            .open()
            .saturating_add(produced_batches as i64))
        .max(0) as usize;
        let stop_status = self.stop_condition_status()?;
        if !stop_status.reached
            && open_batch_count == 0
            && queue_before_produce.failed == 0
            && completed_batches == 0
            && produced_batches == 0
        {
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} cannot make further progress: stop condition not reached (min_samples_reached={}, max_samples_reached={}, absolute_error_reached={}, relative_error_reached={}) and sampler produced no new batches",
                self.run_id,
                self.task.id,
                stop_status.min_samples_reached,
                stop_status.max_samples_reached,
                stop_status.absolute_error_reached,
                stop_status.relative_error_reached
            ))));
        }

        Ok(stop_status.reached && open_batch_count == 0)
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
                evaluator: Some(self.evaluator_config.clone()),
                sampler_aggregator: Some(self.sampler_config.clone()),
                batch_transforms: self.batch_transforms.clone(),
            })
            .await?;
        Ok(())
    }

    async fn persist_sampler_checkpoint(&mut self) -> Result<(), RunnerError> {
        self.checkpoint_sampler_uptime_now();
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

    async fn drain_local_work_on_stop(&mut self) -> Result<(), RunnerError> {
        loop {
            let completed = self.queue.get_processed_ready().await?;
            if completed.is_empty() {
                break;
            }
            self.process_completed_batches(completed).await?;
        }
        self.queue.cancel_nonessential_background_work();
        Ok(())
    }

    pub async fn persist_state(&mut self) -> Result<(), RunnerError> {
        self.finalize_for_pause().await?;
        self.persist_sampler_checkpoint().await
    }

    async fn flush_aggregation(&mut self, force: bool) -> Result<(), RunnerError> {
        self.drain_finished_aggregation_flush().await?;
        if !self.aggregation_flush_due(force) {
            return Ok(());
        }
        if !force && self.pending_aggregation_flush.is_some() {
            return Ok(());
        }

        let persist_snapshot = force
            || self.runtime_state.initial_round_trip_snapshot_pending
            || self.runtime_state.pending_persisted_completed_batches > 0;
        let current_accumulator = self
            .observable_state
            .to_json()
            .map_err(RunnerError::Engine)?;
        let snapshot = if persist_snapshot {
            Some(
                self.sampler
                    .persisted_output()
                    .map_err(RunnerError::Engine)?
                    .unwrap_or(
                        self.observable_state
                            .to_persistent_json()
                            .map_err(RunnerError::Engine)?,
                    ),
            )
        } else {
            None
        };
        let flushed_completed_batches = self.runtime_state.pending_persisted_completed_batches;
        let cleared_initial_round_trip = persist_snapshot;
        let started_at = Instant::now();
        let store = self.store.clone();
        let run_id = self.run_id;
        let task_id = self.task.id;
        let snapshot_ref = snapshot.clone();
        let handle = tokio::spawn(async move {
            store
                .save_aggregation(
                    run_id,
                    task_id,
                    &current_accumulator,
                    snapshot_ref.as_ref(),
                    flushed_completed_batches,
                )
                .await
        });
        self.last_frontend_sync_at = Instant::now();
        let task = PendingAggregationFlushTask {
            started_at,
            flushed_completed_batches,
            cleared_initial_round_trip,
            handle,
        };
        if force {
            self.consume_aggregation_flush_task(task).await?;
        } else {
            self.pending_aggregation_flush = Some(task);
        }
        Ok(())
    }

    async fn drain_finished_aggregation_flush(&mut self) -> Result<(), RunnerError> {
        let Some(task) = self.pending_aggregation_flush.as_ref() else {
            return Ok(());
        };
        if !task.handle.is_finished() {
            return Ok(());
        }
        let task = self
            .pending_aggregation_flush
            .take()
            .expect("checked pending aggregation flush");
        self.consume_aggregation_flush_task(task).await
    }

    async fn consume_aggregation_flush_task(
        &mut self,
        task: PendingAggregationFlushTask,
    ) -> Result<(), RunnerError> {
        match task.handle.await {
            Ok(Ok(())) => {
                observe_duration_pair(
                    &mut self.runtime_state.rolling.persist_accumulator_ms,
                    &mut self.window_state.persist_accumulator_ms,
                    task.started_at.elapsed(),
                );
                if task.cleared_initial_round_trip {
                    self.runtime_state.initial_round_trip_snapshot_pending = false;
                }
                self.runtime_state.pending_persisted_completed_batches = self
                    .runtime_state
                    .pending_persisted_completed_batches
                    .saturating_sub(task.flushed_completed_batches);
                Ok(())
            }
            Ok(Err(err)) => Err(RunnerError::Store(err)),
            Err(err) => Err(RunnerError::Store(StoreError::store(format!(
                "sampler aggregation flush task failed: {err}"
            )))),
        }
    }

    async fn force_cleanup_consumed_completed_batches(&mut self) -> Result<(), RunnerError> {
        if let Some(duration) = self
            .queue
            .force_cleanup_consumed_completed_batches()
            .await?
        {
            observe_duration_pair(
                &mut self.runtime_state.rolling.completed_delete_ms,
                &mut self.window_state.completed_delete_ms,
                duration,
            );
        }
        Ok(())
    }

    pub async fn complete_task(&mut self) -> Result<(), RunnerError> {
        self.finalize_completed_task().await?;
        let measurement_output = match crate::api::measurement::extract_task_measurement(
            &self.store,
            self.run_id,
            &self.task,
        )
        .await
        {
            Ok(measurement) => crate::core::TaskMeasurementOutput::Completed {
                results: measurement.results,
            },
            Err(err) => crate::core::TaskMeasurementOutput::Failed {
                reason: err.to_string(),
            },
        };
        self.store
            .persist_task_measurement_output(self.task.id, &measurement_output)
            .await?;
        self.store.complete_run_task(self.task.id).await?;
        Ok(())
    }

    pub async fn fail_task(&mut self, reason: &str) -> Result<(), RunnerError> {
        self.store.fail_run_task(self.task.id, reason).await?;
        Ok(())
    }

    async fn finalize_for_pause(&mut self) -> Result<(), RunnerError> {
        self.queue.flush().await?;
        self.drain_local_work_on_stop().await?;
        let queue_empty = self.queue.open_batch_count().await? <= 0;
        self.persist_sampler_state(queue_empty).await
    }

    async fn finalize_completed_task(&mut self) -> Result<(), RunnerError> {
        self.queue.flush().await?;
        self.persist_sampler_state(true).await
    }

    async fn persist_sampler_state(&mut self, queue_empty: bool) -> Result<(), RunnerError> {
        self.force_cleanup_consumed_completed_batches().await?;
        self.flush_aggregation(true).await?;
        self.flush_performance_snapshot(true).await?;
        self.flush_progress_sync(true).await?;
        self.persist_stage_state_with_queue_empty(queue_empty).await
    }

    async fn process_completed_batches(
        &mut self,
        completed: Vec<crate::core::CompletedBatch>,
    ) -> Result<CompletedIngestStats, RunnerError> {
        if completed.is_empty() {
            return Ok(CompletedIngestStats {
                completed_batches: 0,
                completed_samples_delta: 0,
            });
        }

        let mut completed_samples_delta = 0_i64;
        let mut completed_training_ingest_ms = 0.0_f64;
        let mut completed_merge_ms = 0.0_f64;
        let mut completed_training_ingest_batches = 0_usize;
        let was_waiting_initial_round_trip = matches!(
            self.runtime_state.accumulator_checkpoint_state,
            AccumulatorCheckpointState::WaitingForInitialRoundTrip
        );
        for batch in &completed {
            let batch_samples = batch.batch_size;
            completed_samples_delta += batch_samples as i64;
            if let Some(total_eval_time_ms) = batch.total_eval_time_ms
                && batch_samples > 0
            {
                observe_value_pair(
                    &mut self.runtime_state.rolling.eval_ms_per_batch,
                    &mut self.window_state.eval_ms_per_batch,
                    total_eval_time_ms,
                );
                observe_value_pair_weighted(
                    &mut self.runtime_state.rolling.eval_ms_per_sample,
                    &mut self.window_state.eval_ms_per_sample,
                    total_eval_time_ms / batch_samples as f64,
                    batch_samples as f64,
                );
                self.queue
                    .observe_completed_eval_batch(batch_samples, total_eval_time_ms);
                self.runtime_state.batch_size_current = self.queue.current_batch_size();
            }

            if batch.requires_training_values {
                let training_values = batch.result.values.as_deref().ok_or_else(|| {
                    RunnerError::Engine(EngineError::engine(format!(
                        "completed batch {} requires training but has no training values",
                        batch.batch_id
                    )))
                })?;
                if training_values.len() != batch_samples {
                    return Err(RunnerError::Engine(EngineError::engine(format!(
                        "completed batch {} training value count mismatch: expected {}, got {}",
                        batch.batch_id,
                        batch_samples,
                        training_values.len()
                    ))));
                }
                let ingest_started = Instant::now();
                self.sampler
                    .ingest_training_values(training_values)
                    .map_err(RunnerError::Engine)?;
                let ingest_time_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
                completed_training_ingest_ms += ingest_time_ms;
                completed_training_ingest_batches += 1;
                self.runtime_state.ingested_batches_total += 1;
                self.runtime_state.ingested_samples_total += batch_samples as i64;
                if batch_samples > 0 {
                    observe_value_pair_weighted(
                        &mut self.runtime_state.rolling.training_ingest_ms_per_sample,
                        &mut self.window_state.training_ingest_ms_per_sample,
                        ingest_time_ms / batch_samples as f64,
                        batch_samples as f64,
                    );
                }
            }

            let merge_started = Instant::now();
            self.observable_state
                .merge(batch.result.accumulator.clone())
                .map_err(RunnerError::Engine)?;
            completed_merge_ms += merge_started.elapsed().as_secs_f64() * 1000.0;
        }

        self.nr_completed_samples += completed_samples_delta;
        self.task.nr_completed_samples += completed_samples_delta;

        self.runtime_state.pending_persisted_completed_batches = self
            .runtime_state
            .pending_persisted_completed_batches
            .saturating_add(completed.len() as i32);
        if completed_samples_delta > 0 {
            self.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
            if was_waiting_initial_round_trip {
                self.runtime_state.initial_round_trip_snapshot_pending = true;
            }
        }
        if completed_training_ingest_batches > 0 {
            observe_value_pair(
                &mut self.runtime_state.rolling.completed_training_ingest_ms,
                &mut self.window_state.completed_training_ingest_ms,
                completed_training_ingest_ms,
            );
        }
        observe_value_pair(
            &mut self.runtime_state.rolling.completed_merge_ingest_ms,
            &mut self.window_state.completed_merge_ingest_ms,
            completed_merge_ms,
        );
        self.queue.mark_processed(&completed);
        Ok(CompletedIngestStats {
            completed_batches: completed.len(),
            completed_samples_delta,
        })
    }

    async fn produce(
        &mut self,
        queue_before_produce: crate::core::BatchQueueCounts,
    ) -> Result<usize, RunnerError> {
        let accumulator_config = self.observable_state.config();
        let sample_plan = self.sampler.sample_plan().map_err(RunnerError::Engine)?;
        let open_before_produce = queue_before_produce.open().max(0) as usize;
        let batch_plan = self
            .resolve_batch_plan(sample_plan, queue_before_produce, open_before_produce)
            .await?;
        let mut produced = Vec::with_capacity(batch_plan.len());
        let mut produced_samples_total = 0_i64;
        for nr_samples in batch_plan {
            let started = Instant::now();
            let batch = self
                .sampler
                .produce_latent_batch(nr_samples)
                .map_err(RunnerError::Engine)?;
            let produce_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            let produced_samples = batch.nr_samples;
            produced_samples_total += produced_samples as i64;
            if produced_samples > 0 {
                observe_value_pair_weighted(
                    &mut self.runtime_state.rolling.produce_ms_per_sample,
                    &mut self.window_state.produce_ms_per_sample,
                    produce_time_ms / produced_samples as f64,
                    produced_samples as f64,
                );
            }
            produced.push(
                batch
                    .with_accumulator_config(accumulator_config.clone())
                    .build(),
            );
        }
        let produced_batches = produced.len();
        if produced_batches == 0 {
            return Ok(0);
        }

        self.runtime_state.produced_batches_total += produced_batches as i64;
        self.runtime_state.produced_samples_total += produced_samples_total;
        self.nr_produced_samples += produced_samples_total;
        self.task.nr_produced_samples += produced_samples_total;
        self.queue.ingest(produced);
        Ok(produced_batches)
    }

    async fn resolve_batch_plan(
        &mut self,
        sample_plan: SamplePlan,
        queue_before_produce: crate::core::BatchQueueCounts,
        open_before_produce: usize,
    ) -> Result<Vec<usize>, RunnerError> {
        if self.stop_condition_status()?.reached {
            return Ok(Vec::new());
        }
        let decision = self.decide_produce(sample_plan, open_before_produce)?;
        let batch_plan = match decision {
            ProduceDecision::None => Vec::new(),
            ProduceDecision::InitialRoundTrip(nr_samples) => vec![nr_samples],
            ProduceDecision::PlannedByQueue(max_samples) => self
                .queue
                .plan_production(max_samples, queue_before_produce)
                .await
                .map_err(RunnerError::from)?,
        };
        self.queue.validate_batch_plan(&batch_plan)?;
        Ok(batch_plan)
    }

    fn decide_produce(
        &mut self,
        sample_plan: SamplePlan,
        open_before_produce: usize,
    ) -> Result<ProduceDecision, RunnerError> {
        let SamplePlan::Produce { nr_samples } = sample_plan else {
            return Ok(ProduceDecision::None);
        };
        let requested = if nr_samples == usize::MAX {
            None
        } else {
            Some(nr_samples)
        };
        let training_samples_remaining = self.sampler.training_samples_remaining();
        let engine_max_samples = match requested {
            Some(requested) => Some(
                training_samples_remaining.map_or(requested, |remaining| remaining.min(requested)),
            ),
            None => training_samples_remaining,
        };
        let max_samples = self.max_samples_to_produce_this_tick(engine_max_samples)?;
        Ok(match self.runtime_state.accumulator_checkpoint_state {
            AccumulatorCheckpointState::NeedsInitialRoundTrip => {
                if self.params.queue.max_queue_size <= open_before_produce {
                    ProduceDecision::None
                } else {
                    let nr_samples = max_samples.unwrap_or(MIN_BATCH_SIZE);
                    if nr_samples == 0 {
                        ProduceDecision::None
                    } else {
                        self.runtime_state.accumulator_checkpoint_state =
                            AccumulatorCheckpointState::WaitingForInitialRoundTrip;
                        ProduceDecision::InitialRoundTrip(nr_samples.min(MIN_BATCH_SIZE))
                    }
                }
            }
            AccumulatorCheckpointState::WaitingForInitialRoundTrip => {
                if open_before_produce == 0 {
                    self.runtime_state.accumulator_checkpoint_state =
                        AccumulatorCheckpointState::NeedsInitialRoundTrip;
                }
                ProduceDecision::None
            }
            AccumulatorCheckpointState::Ready => ProduceDecision::PlannedByQueue(max_samples),
        })
    }

    fn progress_sync_due(&self, force: bool) -> bool {
        force
            || self.frontend_sync_interval.is_zero()
            || self.last_progress_sync_at.elapsed() >= self.frontend_sync_interval
    }

    fn aggregation_flush_due(&self, force: bool) -> bool {
        force
            || self.runtime_state.initial_round_trip_snapshot_pending
            || self.frontend_sync_interval.is_zero()
            || self.last_frontend_sync_at.elapsed() >= self.frontend_sync_interval
    }

    fn performance_snapshot_due(&self, force: bool) -> bool {
        force
            || self.performance_snapshot_interval.is_zero()
            || self.last_snapshot_at.elapsed() >= self.performance_snapshot_interval
    }

    async fn flush_progress_sync(&mut self, force: bool) -> Result<(), RunnerError> {
        if !self.progress_sync_due(force) {
            return Ok(());
        }
        self.store
            .update_run_task_progress(
                self.task.id,
                self.task.nr_produced_samples,
                self.task.nr_completed_samples,
            )
            .await?;
        self.store
            .save_run_sample_progress(
                self.run_id,
                self.nr_produced_samples,
                self.nr_completed_samples,
                self.current_sampler_uptime_ms(),
            )
            .await?;
        self.last_progress_sync_at = Instant::now();
        Ok(())
    }

    async fn flush_performance_snapshot(&mut self, force: bool) -> Result<(), RunnerError> {
        if !self.performance_snapshot_due(force) {
            return Ok(());
        }

        let sampler_tick_busy_ratio = self.take_sampler_tick_busy_ratio_snapshot();
        self.runtime_state.sampler_tick_busy_ratio = sampler_tick_busy_ratio;
        let QueueUtilizationSnapshot {
            insert_task_utilization,
            completed_fetch_utilization,
        } = self.queue.take_utilization_snapshot();
        let mut engine_diagnostics = self.sampler.get_diagnostics();
        if let Some(discrete_pdf) = self.discrete_pdf_diagnostics()? {
            match &mut engine_diagnostics {
                JsonValue::Object(object) => {
                    object.insert("discrete_pdf".to_string(), discrete_pdf);
                }
                other => {
                    engine_diagnostics = json!({
                        "sampler": other.clone(),
                        "discrete_pdf": discrete_pdf,
                    });
                }
            }
        }
        match &mut engine_diagnostics {
            JsonValue::Object(object) => {
                object.insert("runner".to_string(), self.current_runner_diagnostics());
            }
            other => {
                engine_diagnostics = json!({
                    "sampler": other.clone(),
                    "runner": self.current_runner_diagnostics(),
                });
            }
        }

        let mut queue_runtime = self.queue.runtime_metrics();
        queue_runtime.rolling = self.queue.take_metrics_snapshot();
        queue_runtime.insert_task_utilization = insert_task_utilization;
        queue_runtime.completed_fetch_utilization = completed_fetch_utilization;
        let sampler_runtime = self.take_sampler_window_snapshot();
        let evaluator_fleet = self.evaluator_fleet_snapshot().await?;

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            node_name: self.node_name.clone(),
            runtime_metrics: self.runtime_state.to_runtime_metrics(
                sampler_runtime,
                queue_runtime,
                self.nr_completed_samples,
                self.current_sampler_uptime_ms(),
                evaluator_fleet,
            ),
            engine_diagnostics,
            rss_bytes: current_rss_bytes(),
        };
        self.store
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
        self.last_snapshot_at = Instant::now();
        Ok(())
    }

    fn discrete_pdf_diagnostics(&mut self) -> Result<Option<JsonValue>, RunnerError> {
        let Some(config) = self
            .observable_state
            .config()
            .discrete_projections()
            .cloned()
        else {
            return Ok(None);
        };
        let projected =
            collect_discrete_pdf_subspaces(&self.observable_state, &config, config.bin_limit())?;
        if projected.is_empty() {
            return Ok(None);
        }
        let subspaces = projected
            .iter()
            .map(|(_, _, subspace)| subspace.clone())
            .collect::<Vec<_>>();
        let values = self.sampler.discrete_pdf_batch(&subspaces)?;
        let mut projections = serde_json::Map::new();
        for ((projection_name, key, _), value) in projected.into_iter().zip(values) {
            let entry = projections
                .entry(projection_name)
                .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
            if let JsonValue::Object(object) = entry {
                object.insert(
                    discrete_key_label(&key),
                    value.map(JsonValue::from).unwrap_or(JsonValue::Null),
                );
            }
        }
        Ok(Some(json!({
            "schema": "gammaboard-discrete-pdf-v1",
            "projections": projections,
        })))
    }

    async fn evaluator_fleet_snapshot(&self) -> Result<EvaluatorFleetSnapshot, RunnerError> {
        let workers = self.store.get_registered_workers(Some(self.run_id)).await?;
        let mut utilization_sum = 0.0;
        let mut utilization_count = 0usize;
        let mut active_count = 0usize;
        let mut rss_sum = 0_i64;
        let mut rss_count = 0usize;
        for worker in workers {
            if worker.current_run_id != Some(self.run_id)
                || worker.current_role.as_deref() != Some("evaluator")
            {
                continue;
            }
            active_count += 1;
            if let Some(metrics) = worker.evaluator_metrics
                && let Some(idle_ratio) = metrics.idle_profile.map(|profile| profile.idle_ratio)
            {
                utilization_sum += (1.0 - idle_ratio).clamp(0.0, 1.0);
                utilization_count += 1;
            }
            if let Some(rss_bytes) = worker.evaluator_rss_bytes
                && rss_bytes > 0
            {
                rss_sum = rss_sum.saturating_add(rss_bytes);
                rss_count += 1;
            }
        }
        let avg_rss_bytes = if rss_count > 0 {
            Some((rss_sum / rss_count as i64).max(0))
        } else {
            None
        };
        let total_rss_bytes = if rss_count > 0 {
            Some(rss_sum.max(0))
        } else {
            None
        };
        let avg_utilization = if utilization_count > 0 {
            Some(utilization_sum / utilization_count as f64)
        } else {
            None
        };
        Ok(EvaluatorFleetSnapshot {
            active_count,
            avg_utilization,
            avg_rss_bytes,
            total_rss_bytes,
        })
    }

    fn update_completed_samples_per_second(
        &mut self,
        elapsed: Duration,
        completed_samples_delta: i64,
    ) {
        let elapsed_secs = elapsed.as_secs_f64();
        if elapsed_secs > 0.0 {
            let completed_samples_delta_non_negative = completed_samples_delta.max(0);
            let instantaneous_rate =
                (completed_samples_delta_non_negative as f64 / elapsed_secs).max(0.0);
            let previous = self.runtime_state.completed_samples_per_second;
            if !previous.is_finite() || previous <= 0.0 {
                self.runtime_state.completed_samples_per_second = instantaneous_rate;
            } else {
                self.runtime_state.completed_samples_per_second = previous
                    * (1.0 - COMPLETED_SAMPLES_PER_SECOND_EWMA_ALPHA)
                    + instantaneous_rate * COMPLETED_SAMPLES_PER_SECOND_EWMA_ALPHA;
            }
            if completed_samples_delta_non_negative > 0 {
                let previous_eta_rate = self.runtime_state.eta_completed_samples_per_second;
                if !previous_eta_rate.is_finite() || previous_eta_rate <= 0.0 {
                    self.runtime_state.eta_completed_samples_per_second = instantaneous_rate;
                } else {
                    self.runtime_state.eta_completed_samples_per_second = previous_eta_rate
                        * (1.0 - ETA_COMPLETED_SAMPLES_PER_SECOND_EWMA_ALPHA)
                        + instantaneous_rate * ETA_COMPLETED_SAMPLES_PER_SECOND_EWMA_ALPHA;
                }
            }
        }
        self.update_smoothed_eta_seconds(elapsed);
    }
}

fn observe_value_pair(rolling: &mut RollingMetric, window: &mut WindowMetric, value: f64) {
    if !value.is_finite() || value < 0.0 {
        return;
    }
    rolling.observe(value);
    window.observe(value);
}

fn observe_value_pair_weighted(
    rolling: &mut RollingMetric,
    window: &mut WindowMetric,
    value: f64,
    weight: f64,
) {
    if !value.is_finite() || value < 0.0 || !weight.is_finite() || weight <= 0.0 {
        return;
    }
    rolling.observe_weighted(value, weight);
    window.observe(value);
}

fn observe_duration_pair(
    rolling: &mut RollingMetric,
    window: &mut WindowMetric,
    duration: Duration,
) {
    let ms = duration.as_secs_f64() * 1000.0;
    observe_value_pair(rolling, window, ms);
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
            observable_state: crate::evaluation::AccumulatorState::empty_scalar(),
            runtime_state: SamplerRuntimeState {
                batch_size_current: 128,
                ..SamplerRuntimeState::default()
            },
            queue: SamplerQueueCheckpoint::default(),
        };

        assert_eq!(snapshot.reduced_carryover_batch_size(512), 32);
        assert_eq!(snapshot.reduced_carryover_batch_size(24), 24);
    }

    #[test]
    fn fresh_task_does_not_skip_initial_round_trip_due_to_run_level_progress() {
        let mut runtime_state = SamplerRuntimeState::default();
        let task_completed_samples = 0_i64;
        let has_resume_snapshot = false;

        if !has_resume_snapshot && task_completed_samples > 0 {
            runtime_state.accumulator_checkpoint_state = super::AccumulatorCheckpointState::Ready;
        }

        assert_eq!(
            runtime_state.accumulator_checkpoint_state,
            super::AccumulatorCheckpointState::NeedsInitialRoundTrip
        );
    }
}
