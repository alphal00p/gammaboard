//! Sampler task executor orchestration.
#![allow(clippy::too_many_arguments)]
//!
//! This module owns one active sampler task at a time:
//! - restore/build the sampler and accumulator for that task
//! - enqueue latent batches
//! - fetch completed batches and pass training weights back into the sampler
//! - merge completed batch observables into the current accumulator state
//! - persist lightweight UI sync snapshots and full resume checkpoints

#[cfg(test)]
#[path = "sampler_generation_tests.rs"]
mod generation_tests;

use crate::core::checkpoint::{AccumulatorCheckpointState, SamplerProgress};
use crate::core::{
    AccumulatorMetricSelector, BatchTransformConfig, EngineError, EvaluatorConfig,
    RunSampleProgress, RunStageSnapshot, RunTask, SamplerAggregatorPerformanceSnapshot,
    SamplerPerformanceMetrics, SamplerQueueTuning, SamplerWorkRollingAverages, SamplerWorkerStore,
    SamplingConfig, StoreError, WorkerRole,
};
use crate::core::{SamplerAggregatorCheckpoint, SamplerQueueCheckpoint};
use crate::evaluation::{AccumulatorState, relative_error};
use crate::runners::busy_time::BusyTime;
use crate::runners::process_memory::current_rss_bytes;
use crate::runners::queue::MIN_BATCH_SIZE;
use crate::runners::sampler_io::SamplerIo;
use crate::runners::wall_time_rate::WallTimeRate;
use crate::runners::window_metric::WindowMetric;
use crate::runners::{QueueTickResult, SamplerQueue, SamplerQueueConfig};
use crate::sampling::DiscreteSubspace;
use crate::sampling::{Generation, Sampler};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.25;
const TASK_CONFIG_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerAggregatorRunnerParams {
    pub performance_snapshot_interval_ms: u64,
    pub min_tick_time_ms: u64,
    pub frontend_sync_interval_ms: u64,
    /// Background sampler I/O threads, fixed for the lifetime of the runner.
    #[serde(default = "default_sampler_io_threads")]
    pub io_threads: NonZeroUsize,
    #[serde(default = "default_sampler_db_pool_size")]
    pub db_pool_size: u32,
    pub queue: SamplerQueueConfig,
}

fn default_sampler_io_threads() -> NonZeroUsize {
    NonZeroUsize::MIN
}

fn default_sampler_db_pool_size() -> u32 {
    crate::runners::MAX_SAMPLER_DB_CONNECTIONS
}

#[derive(Debug, Clone, Default)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckpointKind {
    Initial,
    Stage,
}

impl SamplerAggregatorCheckpoint {
    pub fn reduced_carryover_batch_size(&self, max_batch_size: usize) -> usize {
        let reduced =
            ((self.queue.batch_size_current as f64) * MAX_BATCH_SIZE_DOWN_FACTOR).round() as usize;
        reduced.clamp(MIN_BATCH_SIZE, max_batch_size)
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
    epoch: String,
    run_id: i32,
    node_name: String,
    task: RunTask,
    sampler: Box<dyn Sampler>,
    observable_state: AccumulatorState,
    evaluation_plan: Option<crate::core::StratificationPlan>,
    evaluator_config: EvaluatorConfig,
    sampler_config: SamplingConfig,
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
    runtime_state: SamplerProgress,
    completed_samples_per_second: f64,
    eta_seconds: Option<f64>,
    initial_round_trip_snapshot_pending: bool,
    pending_persisted_completed_batches: i32,
    window_state: SamplerWindowState,
    queue: SamplerQueue<S>,
    base_queue_config: SamplerQueueConfig,
    last_task_config_refresh_at: Instant,
    waiting_for_campaign_budget: bool,
    campaign_is_bounded: Option<bool>,
    busy: BusyTime,
    node_uuid: String,
    sampler_uptime_started_at: Instant,
    completed_rate: WallTimeRate,
    // Drop after the queue and pending task handles.
    io: SamplerIo,
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
    if let AccumulatorState::Vector(state) = state {
        for (stream_name, component) in state.streams() {
            if !discrete_projection_includes_stream(config, stream_name) {
                continue;
            }
            collect_discrete_pdf_subspaces_from_bins(
                &component.discrete_bins,
                config,
                bin_limit,
                Some(stream_name),
                &mut out,
            )?;
        }
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
    flushed_completed_batches: i32,
    cleared_initial_round_trip: bool,
    handle: tokio::task::JoinHandle<Result<Duration, StoreError>>,
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
    avg_rss_bytes: Option<i64>,
    total_rss_bytes: Option<i64>,
}

fn no_progress_is_terminal(
    stop_reached: bool,
    open_batch_count: usize,
    failed_batches: i64,
    completed_batches: usize,
    produced_batches: usize,
    sampler_wants_to_produce: bool,
    active_evaluator_count: Option<usize>,
) -> bool {
    !stop_reached
        && open_batch_count == 0
        && failed_batches == 0
        && completed_batches == 0
        && produced_batches == 0
        && !(sampler_wants_to_produce && active_evaluator_count == Some(0))
}

impl<S> SamplerAggregatorRunner<S>
where
    S: SamplerWorkerStore + Clone + Send + Sync + 'static,
{
    fn performance_metrics(
        &self,
        sampler: SamplerWorkRollingAverages,
        queue: crate::core::SamplerQueueRuntimeMetrics,
        completed_samples_total: i64,
        sampler_uptime_ms: f64,
        evaluator_fleet: EvaluatorFleetSnapshot,
        engine_diagnostics: JsonValue,
    ) -> SamplerPerformanceMetrics {
        SamplerPerformanceMetrics {
            worker: crate::core::WorkerReportContext {
                node_uuid: self.node_uuid.clone(),
                busy: self.busy.snapshot(),
                epoch: self.epoch.clone(),
                task_id: Some(self.task.id.to_string()),
            },
            engine_diagnostics,
            queue_config: self.params.queue.clone(),
            target_pending_batches: self
                .queue
                .target_pending_batches(evaluator_fleet.active_count),
            buffered_generated_samples: self.runtime_state.generation.pending_samples(),
            checkpoint_state: self.runtime_state.accumulator_checkpoint_state,
            produced_batches_total: self.runtime_state.produced_batches_total,
            produced_samples_total: self.runtime_state.produced_samples_total,
            ingested_batches_total: self.runtime_state.ingested_batches_total,
            ingested_samples_total: self.runtime_state.ingested_samples_total,
            completed_samples_total,
            sampler_uptime_ms,
            completed_samples_per_second: self.completed_samples_per_second,
            eta_seconds: self.eta_seconds,
            batch_size_current: self.queue.current_batch_size(),
            active_evaluator_count: Some(evaluator_fleet.active_count),
            avg_evaluator_rss_bytes: evaluator_fleet.avg_rss_bytes,
            total_evaluator_rss_bytes: evaluator_fleet.total_rss_bytes,
            sampler,
            queue,
        }
    }

    pub fn new(
        store: S,
        run_id: i32,
        node_name: impl Into<String>,
        node_uuid: impl Into<String>,
        mut task: RunTask,
        mut sampler: Box<dyn Sampler>,
        mut observable_state: AccumulatorState,
        evaluator_config: EvaluatorConfig,
        sampler_config: SamplingConfig,
        batch_transforms: Vec<BatchTransformConfig>,
        params: SamplerAggregatorRunnerParams,
        base_queue_config: SamplerQueueConfig,
        initial_batch_size: usize,
        run_progress: RunSampleProgress,
        resume_snapshot: Option<SamplerAggregatorCheckpoint>,
    ) -> Result<Self, StoreError> {
        evaluator_config
            .validate_accumulator_config(&observable_state.config())
            .map_err(|e| StoreError::store(e.to_string()))?;
        let evaluation_plan = crate::evaluation::stratification::prepare_estimator(
            &task.task,
            &sampler_config,
            &mut *sampler,
            &mut observable_state,
            resume_snapshot.is_some(),
        )
        .map_err(|e| StoreError::store(e.to_string()))?;
        let io = SamplerIo::new(params.io_threads)?;
        let mut runtime_state;
        let queue_checkpoint;
        let max_batch_size = params.queue.max_batch_size.max(MIN_BATCH_SIZE);
        let has_resume_snapshot = resume_snapshot.is_some();
        if let Some(snapshot) = resume_snapshot {
            task.nr_produced_samples = snapshot.produced_samples();
            task.nr_completed_samples = snapshot.completed_samples;
            runtime_state = snapshot.runtime_state;
            queue_checkpoint = snapshot.queue;
        } else {
            runtime_state = SamplerProgress::default();
            queue_checkpoint = SamplerQueueCheckpoint {
                batch_size_current: initial_batch_size.clamp(MIN_BATCH_SIZE, max_batch_size),
                ..SamplerQueueCheckpoint::default()
            };
        }
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
        let queue = SamplerQueue::new(
            store.clone(),
            run_id,
            task_id,
            params.queue.clone(),
            queue_checkpoint,
            io.handle().clone(),
        );

        Ok(Self {
            waiting_for_campaign_budget: false,
            campaign_is_bounded: None,
            run_id,
            epoch: uuid::Uuid::new_v4().to_string(),
            node_name: node_name.into(),
            task,
            sampler,
            observable_state,
            evaluation_plan,
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
            completed_samples_per_second: 0.0,
            eta_seconds: None,
            initial_round_trip_snapshot_pending: false,
            pending_persisted_completed_batches: 0,
            window_state: SamplerWindowState::default(),
            busy: queue.busy.clone(),
            queue,
            base_queue_config,
            last_task_config_refresh_at: now
                .checked_sub(TASK_CONFIG_REFRESH_INTERVAL)
                .unwrap_or(now),
            node_uuid: node_uuid.into(),
            sampler_uptime_started_at: now,
            completed_rate: WallTimeRate::new(now),
            io,
        })
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
        Ok(())
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

    fn active_sample_remaining_budget(&self) -> Result<Option<usize>, RunnerError> {
        let Some(target) = self.task.task.sample_stop_condition().map_or_else(
            || self.task.task.nr_expected_samples(),
            |condition| condition.max_samples,
        ) else {
            return Ok(None);
        };
        let remaining = target
            .saturating_sub(self.task.nr_produced_samples)
            .saturating_sub(self.runtime_state.generation.pending_samples() as i64);
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
        let projected = crate::evaluation::accumulator::metrics::estimate_for_stop(
            &self.observable_state,
            stop_condition.metric.as_ref(),
            Some(self.completed_samples_per_second),
        )
        .map_err(RunnerError::Engine)?;
        let coverage_ready = self.observable_state.uncertainty_ready();
        let min_samples_reached = coverage_ready
            && stop_condition
                .min_samples
                .is_none_or(|target| self.task.nr_completed_samples >= target);
        if min_samples_reached
            && (stop_condition.absolute_error.is_some() || stop_condition.relative_error.is_some())
            && projected.is_none()
        {
            let target = stop_condition
                .metric
                .as_ref()
                .map(metric_selector_label)
                .unwrap_or_else(|| "default mean".to_string());
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} stop_condition target {} is incompatible with accumulator {} or has no uncertainty",
                self.run_id,
                self.task.id,
                target,
                self.observable_state.kind_str()
            ))));
        }

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
        let projected = crate::evaluation::accumulator::metrics::estimate_for_stop(
            &self.observable_state,
            stop_condition.metric.as_ref(),
            Some(self.completed_samples_per_second),
        )
        .ok()
        .flatten();
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

    pub fn task_id(&self) -> i64 {
        self.task.id
    }

    pub fn task_state(&self) -> &RunTask {
        &self.task
    }

    pub(crate) async fn save_initial_checkpoint(&mut self) -> Result<(), RunnerError> {
        self.persist_sampler_checkpoint(CheckpointKind::Initial)
            .await?;
        Ok(())
    }

    pub async fn tick(&mut self) -> Result<bool, RunnerError> {
        crate::runners::activity::context(self.run_id, self.task.id);
        crate::runners::activity::set("waiting");
        self.refresh_live_queue_tuning().await?;
        let QueueTickResult {
            completed,
            queue_counts: queue_before_tick,
            queue_snapshot_duration,
            reclaim_duration,
            completed_cleanup_duration,
        } = self.queue.tick().await?;
        if let Some(duration) = reclaim_duration {
            self.window_state.reclaim_ms.observe_duration(duration);
        }
        if let Some(duration) = completed_cleanup_duration {
            self.window_state
                .completed_delete_ms
                .observe_duration(duration);
        }
        self.window_state
            .queue_counts_ms
            .observe_duration(queue_snapshot_duration);
        crate::runners::activity::set("updating sampler");
        let ingest_stats = self.process_completed_batches(completed).await?;
        if ingest_stats.completed_batches > 0 {
            crate::runners::activity::completed_batch();
        }
        crate::runners::activity::set("producing samples");
        let queue_before_produce = crate::core::BatchQueueCounts {
            pending: queue_before_tick.pending,
            claimed: queue_before_tick.claimed,
            completed: queue_before_tick
                .completed
                .saturating_sub(ingest_stats.completed_batches as i64),
            failed: queue_before_tick.failed,
        };
        self.update_completed_samples_per_second(ingest_stats.completed_samples_delta);
        let produce_started = Instant::now();
        let (produced_batches, sampler_wants_to_produce) =
            self.produce(queue_before_produce).await?;
        self.window_state
            .produce_ms
            .observe_duration(produce_started.elapsed());

        // Credit allocation depends on observed completions. Publish a drained
        // window immediately instead of idling for the normal dashboard timer.
        let publish_budget_window = self.waiting_for_campaign_budget
            && ingest_stats.completed_samples_delta > 0
            && queue_before_produce.open() == 0
            && !self.runtime_state.generation.has_outstanding();
        self.flush_aggregation(publish_budget_window).await?;

        let progress_sync_started = Instant::now();
        self.flush_progress_sync(publish_budget_window).await?;
        self.window_state
            .progress_sync_ms
            .observe_duration(progress_sync_started.elapsed());

        let performance_sync_started = Instant::now();
        self.flush_performance_snapshot(false).await?;
        let publication_time = performance_sync_started.elapsed();
        self.window_state
            .performance_sync_ms
            .observe_duration(publication_time);
        crate::runners::activity::set("waiting");
        self.check_tick_terminal_state(
            queue_before_produce,
            ingest_stats.completed_batches,
            produced_batches,
            sampler_wants_to_produce,
        )
    }

    fn check_tick_terminal_state(
        &self,
        queue_before_produce: crate::core::BatchQueueCounts,
        completed_batches: usize,
        produced_batches: usize,
        sampler_wants_to_produce: bool,
    ) -> Result<bool, RunnerError> {
        let open_batch_count = (queue_before_produce
            .open()
            .saturating_add(produced_batches as i64))
        .max(0) as usize;
        let stop_status = self.stop_condition_status()?;
        let active_evaluator_count = self
            .queue
            .diagnostics_snapshot()
            .and_then(|snapshot| snapshot.active_evaluator_count);
        if !self.runtime_state.generation.finished
            && !self.waiting_for_campaign_budget
            && no_progress_is_terminal(
                stop_status.reached,
                open_batch_count,
                queue_before_produce.failed,
                completed_batches,
                produced_batches,
                sampler_wants_to_produce,
                active_evaluator_count,
            )
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

        Ok(
            (stop_status.reached || self.runtime_state.generation.finished)
                && open_batch_count == 0
                && !self.runtime_state.generation.has_outstanding(),
        )
    }

    /// One save lifecycle for activation, pause and task completion. The initial
    /// checkpoint precedes production; subsequent saves also publish a branchable stage.
    async fn persist_sampler_checkpoint(
        &mut self,
        kind: CheckpointKind,
    ) -> Result<(), RunnerError> {
        crate::runners::activity::set("saving checkpoint");
        self.store.record_checkpoint_status(self.run_id, &json!({
            "state":"saving", "task_id":self.task.id, "save_started_at":chrono::Utc::now(), "error":null
        })).await?;
        let result: Result<(), RunnerError> = async {
            self.queue.flush().await?;
            if kind == CheckpointKind::Stage {
                self.drain_local_work_on_stop().await?;
                self.flush_aggregation(true).await?;
                self.flush_performance_snapshot(true).await?;
                self.flush_progress_sync(true).await?;
            }
            self.checkpoint_sampler_uptime_now();
            let checkpoint = SamplerAggregatorCheckpoint {
                format_version: crate::compatibility::MODEL_VERSION,
                completed_samples: self.task.nr_completed_samples,
                task_id: self.task.id,
                output_snapshot_id: None,
                batches_completed: 0,
                sampler_snapshot: self
                    .sampler_config
                    .snapshot(&mut *self.sampler)
                    .map_err(RunnerError::Engine)?,
                observable_state: self.observable_state.clone(),
                runtime_state: self.runtime_state.clone(),
                queue: self.queue.checkpoint(),
            };
            let stage = if kind == CheckpointKind::Stage {
                Some(RunStageSnapshot {
                    id: None,
                    run_id: self.run_id,
                    task_id: Some(self.task.id),
                    name: self.task.name.clone(),
                    sequence_nr: Some(self.task.sequence_nr),
                    // Retained consumed results are recovery history, not open work.
                    queue_empty: self.queue.queue_counts().await?.open() == 0
                        && !self.runtime_state.generation.has_outstanding(),
                    sampler_snapshot: Some(checkpoint.sampler_snapshot.clone()),
                    observable_state: Some(checkpoint.observable_state.clone()),
                    evaluator: Some(self.evaluator_config.clone()),

                    batch_transforms: self.batch_transforms.clone(),
                })
            } else {
                None
            };
            let _io = self.busy.io();
            self.store
                .save_sampler_checkpoint(self.run_id, &checkpoint, stage.as_ref())
                .await?;
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            let _ = self.store.record_checkpoint_status(self.run_id, &json!({
                "state":"failed", "task_id":self.task.id, "failed_at":chrono::Utc::now(), "error":error.to_string()
            })).await;
        }
        result
    }

    async fn drain_local_work_on_stop(&mut self) -> Result<(), RunnerError> {
        loop {
            let completed = self.queue.get_processed_ready().await?;
            if completed.is_empty() {
                break;
            }
            self.process_completed_batches(completed).await?;
        }
        self.queue.finish_background_maintenance().await?;
        Ok(())
    }

    pub async fn persist_state(&mut self) -> Result<(), RunnerError> {
        // Task completion already published its final checkpoint. The supervisor
        // still calls stop to release the role; do not snapshot it a second time.
        if self.task.state == crate::core::RunTaskState::Completed {
            return Ok(());
        }
        self.persist_sampler_checkpoint(CheckpointKind::Stage)
            .await?;
        // Cleanup is bounded maintenance after durable publication, not part of saving.
        if let Some(duration) = self.queue.cleanup_completed_batches().await? {
            self.window_state
                .completed_delete_ms
                .observe_duration(duration);
        }
        Ok(())
    }

    async fn flush_aggregation(&mut self, force: bool) -> Result<(), RunnerError> {
        if force {
            // A final write must follow the pending write, otherwise the older
            // accumulator can land after the checkpoint and batches get counted twice.
            if let Some(task) = self.pending_aggregation_flush.take() {
                self.consume_aggregation_flush_task(task).await?;
            }
        } else {
            self.drain_finished_aggregation_flush().await?;
        }
        if !self.aggregation_flush_due(force) {
            return Ok(());
        }
        if !force && self.pending_aggregation_flush.is_some() {
            return Ok(());
        }

        let persist_snapshot = force
            || self.initial_round_trip_snapshot_pending
            || self.pending_persisted_completed_batches > 0;
        let cleared_initial_round_trip = persist_snapshot;
        let current_accumulator = self
            .observable_state
            .to_json()
            .map_err(RunnerError::Engine)?;
        let snapshot = if persist_snapshot
            && matches!(self.observable_state, AccumulatorState::Stratified(_))
        {
            Some(
                self.observable_state
                    .to_persistent_json()
                    .map_err(RunnerError::Engine)?,
            )
        } else if persist_snapshot {
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
        let flushed_completed_batches = self.pending_persisted_completed_batches;
        let busy = self.busy.clone();
        let store = self.store.clone();
        let run_id = self.run_id;
        let task_id = self.task.id;
        let handle = self.io.handle().spawn(async move {
            let _io = busy.io();
            let started = Instant::now();
            store
                .save_aggregation(
                    run_id,
                    task_id,
                    &current_accumulator,
                    snapshot.as_ref(),
                    flushed_completed_batches,
                )
                .await?;
            Ok(started.elapsed())
        });
        self.last_frontend_sync_at = Instant::now();
        let task = PendingAggregationFlushTask {
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
            Ok(Ok(duration)) => {
                self.window_state
                    .persist_accumulator_ms
                    .observe_duration(duration);
                if task.cleared_initial_round_trip {
                    self.initial_round_trip_snapshot_pending = false;
                }
                self.pending_persisted_completed_batches = self
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

    pub async fn complete_task(&mut self) -> Result<(), RunnerError> {
        self.persist_state().await?;
        let measurement_output = match crate::services::measurement::extract_task_measurement(
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
        self.task.state = crate::core::RunTaskState::Completed;
        Ok(())
    }

    pub async fn fail_task(&mut self, reason: &str) -> Result<(), RunnerError> {
        self.store.fail_run_task(self.task.id, reason).await?;
        Ok(())
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

        // Validate the entire group before mutating the sampler, accumulator, or cursor.
        if let Some(batch) = completed.iter().find(|batch| batch.task_id != self.task.id) {
            return Err(RunnerError::Store(StoreError::store(format!(
                "completed batch {} belongs to task {}, expected task {}",
                batch.batch_id, batch.task_id, self.task.id
            ))));
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
                self.window_state
                    .eval_ms_per_batch
                    .observe(total_eval_time_ms);
                self.window_state
                    .eval_ms_per_sample
                    .observe_weighted(total_eval_time_ms / batch_samples as f64, batch_samples);
                self.queue
                    .observe_completed_eval_batch(batch_samples, total_eval_time_ms);
            }

            if batch.requires_training_values {
                let training_values = batch.result.training_values.as_deref().ok_or_else(|| {
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
                let assembled = {
                    let _compute = self.busy.compute();
                    self.runtime_state
                        .generation
                        .accept_training_values(training_values)?
                };
                if let Some(values) = assembled {
                    let ingest_started = Instant::now();
                    {
                        let _compute = self.busy.compute();
                        self.sampler
                            .feedback(&values)
                            .map_err(RunnerError::Engine)?;
                    }
                    let ingest_time_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
                    completed_training_ingest_ms += ingest_time_ms;
                    completed_training_ingest_batches += 1;
                    self.runtime_state.ingested_batches_total += 1;
                    self.runtime_state.ingested_samples_total += values.len() as i64;
                    self.window_state
                        .training_ingest_ms_per_sample
                        .observe_weighted(ingest_time_ms / values.len() as f64, values.len());
                }
            }

            let merge_started = Instant::now();
            {
                let _compute = self.busy.compute();
                self.observable_state
                    .merge(batch.result.accumulator.clone())
                    .map_err(RunnerError::Engine)?;
            }
            completed_merge_ms += merge_started.elapsed().as_secs_f64() * 1000.0;
        }

        self.nr_completed_samples += completed_samples_delta;
        self.task.nr_completed_samples += completed_samples_delta;

        self.pending_persisted_completed_batches = self
            .pending_persisted_completed_batches
            .saturating_add(completed.len() as i32);
        if completed_samples_delta > 0 {
            self.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
            if was_waiting_initial_round_trip {
                self.initial_round_trip_snapshot_pending = true;
            }
        }
        if completed_training_ingest_batches > 0 {
            self.window_state
                .completed_training_ingest_ms
                .observe(completed_training_ingest_ms);
        }
        self.window_state
            .completed_merge_ingest_ms
            .observe(completed_merge_ms);
        self.queue.mark_processed(&completed);
        Ok(CompletedIngestStats {
            completed_batches: completed.len(),
            completed_samples_delta,
        })
    }

    async fn produce(
        &mut self,
        queue_before_produce: crate::core::BatchQueueCounts,
    ) -> Result<(usize, bool), RunnerError> {
        self.waiting_for_campaign_budget = false;
        if self.runtime_state.accumulator_checkpoint_state
            == AccumulatorCheckpointState::WaitingForInitialRoundTrip
        {
            return Ok((0, true));
        }
        if !self.runtime_state.generation.has_pending() {
            if self.runtime_state.generation.finished {
                return Ok((0, false));
            }
            let budget = self.active_sample_remaining_budget()?;
            if budget == Some(0) || self.stop_condition_status()?.reached {
                return Ok((0, false));
            }
            if !self.queue.needs_generation(queue_before_produce).await? {
                return Ok((0, true));
            }
            // Parentage and the controller definition are immutable. Ordinary
            // runs pay this lookup once, rather than on every sampler draw.
            let allowance = if self.campaign_is_bounded == Some(false) {
                None
            } else {
                let value = self
                    .store
                    .campaign_sample_allowance(self.run_id, self.task.id)
                    .await?;
                self.campaign_is_bounded = Some(value.is_some());
                value
            };
            let campaign_remaining = allowance.map(|allowance| {
                allowance
                    .saturating_sub(self.task.nr_produced_samples)
                    .max(0) as usize
            });
            if campaign_remaining == Some(0) {
                // The controller grants another window after observing progress.
                // Waiting for credits is not sampler exhaustion or a failed task.
                self.waiting_for_campaign_budget = true;
                return Ok((0, true));
            }
            let max_samples = self
                .params
                .queue
                .max_generation_size
                .get()
                .min(budget.unwrap_or(usize::MAX))
                .min(campaign_remaining.unwrap_or(usize::MAX));
            let started = Instant::now();
            let generated = {
                let _compute = self.busy.compute();
                self.sampler.generate(max_samples)?
            };
            match generated {
                Generation::Waiting => return Ok((0, false)),
                Generation::Finished => {
                    self.runtime_state.generation.finished = true;
                    return Ok((0, false));
                }
                Generation::Batch {
                    batch,
                    training_remaining,
                } => {
                    let _compute = self.busy.compute();
                    if batch.nr_samples == 0
                        || batch.nr_samples > max_samples
                        || training_remaining.is_some_and(|n| batch.nr_samples > n)
                    {
                        return Err(EngineError::engine(
                            "sampler returned a draw outside its budget or training window",
                        )
                        .into());
                    }
                    let generated_samples = batch.nr_samples;
                    self.runtime_state
                        .generation
                        .buffer_draw(batch, training_remaining)?;
                    if training_remaining.is_some() {
                        self.runtime_state.generation.record_draw(generated_samples);
                    }
                    self.window_state.produce_ms_per_sample.observe_weighted(
                        started.elapsed().as_secs_f64() * 1000.0 / generated_samples as f64,
                        generated_samples,
                    );
                }
            }
        }
        let (slots, chunk_size) = if self.runtime_state.accumulator_checkpoint_state
            == AccumulatorCheckpointState::NeedsInitialRoundTrip
        {
            self.runtime_state.accumulator_checkpoint_state =
                AccumulatorCheckpointState::WaitingForInitialRoundTrip;
            (1, MIN_BATCH_SIZE)
        } else {
            (
                self.params.queue.max_batches_per_tick,
                self.queue
                    .production_batch_size(self.runtime_state.generation.training_remaining()),
            )
        };
        Ok((self.enqueue_generated(slots, chunk_size)?, true))
    }

    fn enqueue_generated(&mut self, slots: usize, chunk_size: usize) -> Result<usize, RunnerError> {
        let _compute = self.busy.compute();
        let requires_feedback = self.runtime_state.generation.requires_feedback();
        let batches = self
            .runtime_state
            .generation
            .take_batches(slots, chunk_size)?;
        let count = batches.len();
        let accumulator = self.observable_state.config();
        let samples = batches
            .iter()
            .map(|batch| batch.nr_samples as i64)
            .sum::<i64>();
        self.runtime_state.produced_batches_total += count as i64;
        self.runtime_state.produced_samples_total += samples;
        self.nr_produced_samples += samples;
        self.task.nr_produced_samples += samples;
        self.queue.ingest(
            batches
                .into_iter()
                .map(|batch| {
                    let mut work = batch.for_evaluation(accumulator.clone());
                    work.stratification = self.evaluation_plan.clone();
                    work
                })
                .collect(),
            requires_feedback,
        );
        Ok(count)
    }

    fn progress_sync_due(&self, force: bool) -> bool {
        force
            || self.frontend_sync_interval.is_zero()
            || self.last_progress_sync_at.elapsed() >= self.frontend_sync_interval
    }

    fn aggregation_flush_due(&self, force: bool) -> bool {
        force
            || self.initial_round_trip_snapshot_pending
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
        let _io = self.busy.io();
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

        let mut engine_diagnostics = self.sampler.diagnostics();
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
        let mut queue_runtime = self.queue.runtime_metrics();
        queue_runtime.rolling = self.queue.take_metrics_snapshot();
        let sampler_runtime = self.take_sampler_window_snapshot();
        let evaluator_fleet = self.evaluator_fleet_snapshot().await?;

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            node_name: self.node_name.clone(),
            metrics: self.performance_metrics(
                sampler_runtime,
                queue_runtime,
                self.nr_completed_samples,
                self.current_sampler_uptime_ms(),
                evaluator_fleet,
                engine_diagnostics,
            ),
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
        let mut active_count = 0usize;
        let mut rss_sum = 0_i64;
        let mut rss_count = 0usize;
        for worker in workers {
            if worker.current_run_id != Some(self.run_id)
                || worker.current_role != Some(WorkerRole::Evaluator)
            {
                continue;
            }
            active_count += 1;
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
        Ok(EvaluatorFleetSnapshot {
            active_count,
            avg_rss_bytes,
            total_rss_bytes,
        })
    }

    fn update_completed_samples_per_second(&mut self, completed_samples_delta: i64) {
        // Observe at the same point each tick so production, training, syncing
        // and the outer worker sleep all contribute to elapsed wall time.
        let rate = self
            .completed_rate
            .observe(Instant::now(), completed_samples_delta.max(0) as f64);
        self.completed_samples_per_second = rate;
        // The rate already averages a full wall-time window. Further smoothing
        // ETA independently would make it inconsistent with the displayed rate.
        self.eta_seconds = self.estimate_eta_seconds_for_current_state(rate);
    }
}

#[cfg(test)]
mod tests {
    use super::{SamplerAggregatorCheckpoint, SamplerProgress, no_progress_is_terminal};
    use crate::core::SamplerQueueCheckpoint;
    use crate::sampling::SavedSampler;
    use serde_json::json;

    #[test]
    fn carryover_batch_size_is_reduced_and_clamped() {
        let snapshot = SamplerAggregatorCheckpoint {
            format_version: crate::compatibility::MODEL_VERSION,
            completed_samples: 0,
            task_id: 1,
            output_snapshot_id: None,
            batches_completed: 0,
            sampler_snapshot: SavedSampler {
                version: crate::compatibility::MODEL_VERSION,
                config: crate::core::SamplingConfig {
                    stratification: None,
                    sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                        params: Default::default(),
                    },
                    materializer: None,
                    transforms: None,
                },
                state: json!({}),
            },
            observable_state: crate::evaluation::AccumulatorState::empty_scalar(),
            runtime_state: SamplerProgress::default(),
            queue: SamplerQueueCheckpoint {
                batch_size_current: 128,
                ..SamplerQueueCheckpoint::default()
            },
        };

        assert_eq!(snapshot.reduced_carryover_batch_size(512), 32);
        assert_eq!(snapshot.reduced_carryover_batch_size(24), 24);
    }

    #[test]
    fn fresh_task_does_not_skip_initial_round_trip_due_to_run_level_progress() {
        let mut runtime_state = SamplerProgress::default();
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

    #[test]
    fn no_progress_waits_for_a_newly_assigned_evaluator() {
        assert!(!no_progress_is_terminal(false, 0, 0, 0, 0, true, Some(0)));
        assert!(no_progress_is_terminal(false, 0, 0, 0, 0, true, Some(1)));
        assert!(no_progress_is_terminal(false, 0, 0, 0, 0, false, None));
    }
}
