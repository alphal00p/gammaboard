//! Sampler-aggregator runner orchestration.
//!
//! This module intentionally focuses on process orchestration:
//! - load persisted active-stage observable state
//! - call engine hooks
//! - enqueue produced batches
//! - fetch completed batches and pass training weights back into the engine
//! - merge completed batch observables into the active-stage observable state
//! - persist compact task-local observable snapshots
//! - delete consumed completed batches

use crate::core::{
    AggregationStore, CompletedBatch, ParametrizationState, ParametrizationVersionStore,
    RollingMetricSnapshot, RunSampleProgress, RunStageSnapshot, RunTask, RunTaskSpec, RunTaskStore,
    SamplerAggregatorPerformanceSnapshot, SamplerRollingAverages, SamplerRuntimeMetrics,
    StoreError, WorkQueueStore,
};
use crate::core::{EngineError, EvaluatorConfig, ParametrizationConfig, SamplerAggregatorConfig};
use crate::evaluation::{ObservableState, PointSpec};
use crate::runners::rolling_metric::RollingMetric;
use crate::sampling::{
    ParametrizationBuildContext, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot,
};
use crate::stores::RunControlStore;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::info;

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

#[derive(Debug, Clone)]
pub struct RunnerTick {
    pub enqueued_batches: usize,
    pub processed_completed_batches: usize,
    pub queue_depleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RollingAveragesState {
    eval_ms_per_sample: RollingMetric,
    eval_ms_per_batch: RollingMetric,
    sampler_produce_ms_per_sample: RollingMetric,
    sampler_ingest_ms_per_sample: RollingMetric,
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
    pub version: u32,
    pub engine: SamplerAggregatorSnapshot,
    pub observable_state: JsonValue,
    active_runtime_task_id: Option<i64>,
    current_parametrization_state_version: i64,
    runtime_state: SamplerRuntimeState,
    last_pending_after_enqueue: Option<usize>,
    training_completion_marked: bool,
    auto_stop_triggered: bool,
}

impl SamplerRuntimeState {
    fn rolling_metric_snapshot(metric: &RollingMetric) -> RollingMetricSnapshot {
        RollingMetricSnapshot {
            mean: metric.value(),
            std_dev: metric.std_dev(),
        }
    }

    fn to_runtime_metrics(&self, completed_samples_per_second: f64) -> SamplerRuntimeMetrics {
        SamplerRuntimeMetrics {
            produced_batches_total: self.produced_batches_total,
            produced_samples_total: self.produced_samples_total,
            ingested_batches_total: self.ingested_batches_total,
            ingested_samples_total: self.ingested_samples_total,
            completed_samples_per_second,
            batch_size_current: self.batch_size_current,
            rolling: SamplerRollingAverages {
                eval_ms_per_sample: Self::rolling_metric_snapshot(&self.rolling.eval_ms_per_sample),
                eval_ms_per_batch: Self::rolling_metric_snapshot(&self.rolling.eval_ms_per_batch),
                sampler_produce_ms_per_sample: Self::rolling_metric_snapshot(
                    &self.rolling.sampler_produce_ms_per_sample,
                ),
                sampler_ingest_ms_per_sample: Self::rolling_metric_snapshot(
                    &self.rolling.sampler_ingest_ms_per_sample,
                ),
                queue_remaining_ratio: Self::rolling_metric_snapshot(
                    &self.rolling.queue_remaining_ratio,
                ),
                batches_consumed_per_tick: Self::rolling_metric_snapshot(
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

pub struct SamplerAggregatorRunner<WQ, AS, RC, PS, TS> {
    run_id: i32,
    node_id: String,
    engine: Box<dyn SamplerAggregator>,
    observable_state: ObservableState,
    work_queue: WQ,
    aggregation_store: AS,
    run_control: RC,
    parametrization_state_store: PS,
    task_store: TS,
    config: SamplerAggregatorRunnerParams,
    point_spec: PointSpec,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    current_parametrization_state_version: i64,
    runtime_state: SamplerRuntimeState,
    last_performance_completed_samples: i64,
    last_pending_after_enqueue: Option<usize>,
    training_completion_marked: bool,
    auto_stop_triggered: bool,
    current_task: Option<RunTask>,
    active_runtime_task_id: Option<i64>,
    evaluator_config: EvaluatorConfig,
    current_sampler_config: SamplerAggregatorConfig,
    current_parametrization_state: ParametrizationState,
}

struct TransitionRuntimeState {
    sampler_snapshot: Option<SamplerAggregatorSnapshot>,
    sampler_config: SamplerAggregatorConfig,
    parametrization_state: ParametrizationState,
    observable_state: ObservableState,
}

impl<WQ, AS, RC, PS, TS> SamplerAggregatorRunner<WQ, AS, RC, PS, TS>
where
    WQ: WorkQueueStore,
    AS: AggregationStore,
    RC: RunControlStore,
    PS: ParametrizationVersionStore,
    TS: RunTaskStore,
{
    const MIN_BATCH_SIZE: usize = 1;
    const MAX_BATCH_SIZE_UP_FACTOR: f64 = 1.25;
    const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.80;
    const MIN_BATCH_SIZE_CHANGE_RATIO: f64 = 0.03;
    pub const SNAPSHOT_VERSION: u32 = 1;

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

        // Use measured queue drain to keep pending depth near a target remaining ratio.
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
                    // Build near-uniform batch sizes (difference <= 1) while
                    // respecting current batch-size cap and exact sample total.
                    let nr_batches = max_samples.div_ceil(self.runtime_state.batch_size_current);
                    let base_size = max_samples / nr_batches;
                    let remainder = max_samples % nr_batches;
                    let mut plan = Vec::with_capacity(nr_batches);
                    for i in 0..nr_batches {
                        let size = if i < remainder {
                            base_size + 1
                        } else {
                            base_size
                        };
                        plan.push(size);
                    }
                    plan
                }
            }
        }
    }

    pub async fn new(
        run_id: i32,
        node_id: impl Into<String>,
        engine: Box<dyn SamplerAggregator>,
        mut observable_state: ObservableState,
        work_queue: WQ,
        aggregation_store: AS,
        run_control: RC,
        parametrization_state_store: PS,
        task_store: TS,
        config: SamplerAggregatorRunnerParams,
        point_spec: PointSpec,
        evaluator_config: EvaluatorConfig,
        initial_sampler_config: SamplerAggregatorConfig,
        initial_parametrization_config: ParametrizationConfig,
    ) -> Result<Self, RunnerError> {
        let initial_batch_size = config.max_batch_size.min(64).max(Self::MIN_BATCH_SIZE);
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
        let performance_snapshot_interval =
            Duration::from_millis(config.performance_snapshot_interval_ms);
        let current_observable = aggregation_store.load_current_observable(run_id).await?;
        if let Some(snapshot) = current_observable {
            observable_state =
                ObservableState::from_json(&snapshot).map_err(RunnerError::Engine)?;
        }
        let persisted_progress = aggregation_store
            .load_run_sample_progress(run_id)
            .await?
            .unwrap_or(RunSampleProgress {
                nr_produced_samples: 0,
                nr_completed_samples: 0,
            });
        let latest_version = parametrization_state_store
            .load_latest_parametrization_version(run_id)
            .await?;
        let initial_parametrization_state = Self::build_parametrization_state(
            &initial_parametrization_config,
            &point_spec,
            None,
            None,
        )?;
        let (current_parametrization_state_version, current_parametrization_state) =
            match latest_version {
                Some(version) => {
                    let state = parametrization_state_store
                        .load_parametrization_version(run_id, version)
                        .await?
                        .ok_or_else(|| {
                            RunnerError::Store(StoreError::store(format!(
                                "missing parametrization state for run {} version {}",
                                run_id, version
                            )))
                        })?;
                    (version, state)
                }
                None => {
                    let version = 1;
                    parametrization_state_store
                        .save_parametrization_version(
                            run_id,
                            version,
                            &initial_parametrization_state,
                        )
                        .await?;
                    (version, initial_parametrization_state)
                }
            };

        Ok(Self {
            run_id,
            node_id: node_id.into(),
            engine,
            observable_state,
            work_queue,
            aggregation_store,
            run_control,
            parametrization_state_store,
            task_store,
            config,
            point_spec,
            nr_produced_samples: persisted_progress.nr_produced_samples,
            nr_completed_samples: persisted_progress.nr_completed_samples,
            performance_snapshot_interval,
            last_snapshot_at: Instant::now(),
            current_parametrization_state_version,
            runtime_state: SamplerRuntimeState {
                produced_samples_total: persisted_progress.nr_produced_samples,
                batch_size_current: initial_batch_size,
                ..SamplerRuntimeState::default()
            },
            last_performance_completed_samples: persisted_progress.nr_completed_samples,
            last_pending_after_enqueue: None,
            training_completion_marked: false,
            auto_stop_triggered: false,
            current_task: None,
            active_runtime_task_id: None,
            evaluator_config,
            current_sampler_config: initial_sampler_config,
            current_parametrization_state,
        })
    }

    pub fn restore_snapshot(
        &mut self,
        snapshot: SamplerAggregatorRunnerSnapshot,
    ) -> Result<(), RunnerError> {
        if snapshot.version != Self::SNAPSHOT_VERSION {
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "unsupported sampler runner snapshot version: {}",
                snapshot.version
            ))));
        }
        self.observable_state =
            ObservableState::from_json(&snapshot.observable_state).map_err(RunnerError::Engine)?;
        self.active_runtime_task_id = snapshot.active_runtime_task_id;
        self.current_parametrization_state_version = snapshot.current_parametrization_state_version;
        self.runtime_state = snapshot.runtime_state;
        self.runtime_state.produced_samples_total = self
            .runtime_state
            .produced_samples_total
            .max(self.nr_produced_samples);
        self.last_performance_completed_samples = self.nr_completed_samples;
        self.last_pending_after_enqueue = snapshot.last_pending_after_enqueue;
        self.training_completion_marked = snapshot.training_completion_marked;
        self.auto_stop_triggered = snapshot.auto_stop_triggered;
        Ok(())
    }

    pub fn snapshot_state(&mut self) -> Result<SamplerAggregatorRunnerSnapshot, RunnerError> {
        Ok(SamplerAggregatorRunnerSnapshot {
            version: Self::SNAPSHOT_VERSION,
            engine: self.engine.snapshot().map_err(RunnerError::Engine)?,
            observable_state: self
                .observable_state
                .to_json()
                .map_err(RunnerError::Engine)?,
            active_runtime_task_id: self.active_runtime_task_id,
            current_parametrization_state_version: self.current_parametrization_state_version,
            runtime_state: self.runtime_state.clone(),
            last_pending_after_enqueue: self.last_pending_after_enqueue,
            training_completion_marked: self.training_completion_marked,
            auto_stop_triggered: self.auto_stop_triggered,
        })
    }

    pub async fn persist_snapshot(&mut self) -> Result<(), RunnerError> {
        let snapshot = self.snapshot_state()?;
        let payload = serde_json::to_value(&snapshot)
            .map_err(|err| RunnerError::Engine(EngineError::from(err)))?;
        self.aggregation_store
            .save_sampler_runner_snapshot(self.run_id, &payload)
            .await?;
        Ok(())
    }

    pub async fn tick(&mut self) -> Result<RunnerTick, RunnerError> {
        let pending_before_tick = self
            .work_queue
            .get_pending_batch_count(self.run_id)
            .await?
            .max(0) as usize;

        if let Some(previous_pending_after) = self.last_pending_after_enqueue {
            if previous_pending_after > 0 {
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

        self.tune_batch_size();
        let queue_depleted = pending_before_tick == 0;

        let completed = self
            .work_queue
            .fetch_completed_batches(self.run_id, self.config.completed_batch_fetch_limit)
            .await?;
        let consumed_ids = self.process_completed(&completed).await?;
        self.work_queue
            .delete_completed_batches(&consumed_ids)
            .await?;
        self.try_mark_training_completed().await?;
        self.sync_active_task_progress().await?;
        let open_batch_count = self
            .work_queue
            .get_open_batch_count(self.run_id)
            .await?
            .max(0) as usize;
        let enqueued_batches = self
            .run_task_tick(pending_before_tick, open_batch_count)
            .await?;
        self.flush_run_sample_progress().await?;
        self.flush_performance_snapshot().await?;

        Ok(RunnerTick {
            enqueued_batches,
            processed_completed_batches: consumed_ids.len(),
            queue_depleted,
        })
    }

    async fn process_completed(
        &mut self,
        completed: &[CompletedBatch],
    ) -> Result<Vec<i64>, RunnerError> {
        if completed.is_empty() {
            return Ok(Vec::new());
        }

        let mut completed_samples_delta = 0_i64;
        for batch in completed {
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
            if batch.requires_training {
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
                self.engine
                    .ingest_training_weights(training_weights)
                    .map_err(RunnerError::Engine)?;
                let ingest_time_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
                let ingested_samples = batch_samples;
                self.runtime_state.ingested_batches_total += 1;
                self.runtime_state.ingested_samples_total += ingested_samples as i64;
                if ingested_samples > 0 {
                    self.runtime_state
                        .rolling
                        .sampler_ingest_ms_per_sample
                        .observe(ingest_time_ms / ingested_samples as f64);
                }
            }

            self.observable_state
                .merge(batch.result.observable.clone())
                .map_err(RunnerError::Engine)?;
        }
        self.nr_completed_samples += completed_samples_delta;
        if let Some(task) = self.current_task.as_mut()
            && task.task.nr_expected_samples().is_some()
        {
            task.nr_completed_samples += completed_samples_delta;
        }

        let current_observable = self
            .observable_state
            .to_json()
            .map_err(RunnerError::Engine)?;
        let snapshot = self
            .observable_state
            .to_persistent_json()
            .map_err(RunnerError::Engine)?;
        let task_id = self
            .current_task
            .as_ref()
            .ok_or_else(|| {
                RunnerError::Engine(EngineError::engine(
                    "cannot persist observable snapshot without an active task",
                ))
            })?
            .id;

        self.aggregation_store
            .save_aggregation(
                self.run_id,
                task_id,
                &current_observable,
                &snapshot,
                completed.len() as i32,
            )
            .await?;

        Ok(completed.iter().map(|batch| batch.batch_id).collect())
    }

    async fn try_mark_training_completed(&mut self) -> Result<(), RunnerError> {
        if self.training_completion_marked || self.engine.training_samples_remaining().is_some() {
            return Ok(());
        }
        let _ = self
            .work_queue
            .try_set_training_completed_at(self.run_id)
            .await?;
        self.training_completion_marked = true;
        Ok(())
    }

    fn build_parametrization_state(
        parametrization: &ParametrizationConfig,
        point_spec: &PointSpec,
        handoff_snapshot: Option<&SamplerAggregatorSnapshot>,
        previous_state: Option<&ParametrizationState>,
    ) -> Result<ParametrizationState, RunnerError> {
        parametrization
            .build(ParametrizationBuildContext {
                sampler_aggregator_snapshot: handoff_snapshot,
                parametrization_snapshot: previous_state.map(|state| &state.snapshot),
            })
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

    fn same_serialized<T: Serialize>(left: &T, right: &T) -> Result<bool, RunnerError> {
        let left = serde_json::to_value(left).map_err(|err| {
            RunnerError::Engine(EngineError::engine(format!(
                "failed to serialize runtime config for comparison: {err}"
            )))
        })?;
        let right = serde_json::to_value(right).map_err(|err| {
            RunnerError::Engine(EngineError::engine(format!(
                "failed to serialize runtime config for comparison: {err}"
            )))
        })?;
        Ok(left == right)
    }

    fn decode_transition_runtime_state(
        snapshot: RunStageSnapshot,
    ) -> Result<TransitionRuntimeState, RunnerError> {
        let runner_snapshot: SamplerAggregatorRunnerSnapshot =
            serde_json::from_value(snapshot.sampler_runner_snapshot).map_err(|err| {
                RunnerError::Engine(EngineError::engine(format!(
                    "failed to decode stage sampler runner snapshot: {err}"
                )))
            })?;
        let sampler_config: SamplerAggregatorConfig =
            serde_json::from_value(snapshot.sampler_aggregator).map_err(|err| {
                RunnerError::Engine(EngineError::engine(format!(
                    "failed to decode stage sampler config: {err}"
                )))
            })?;
        let parametrization_state: ParametrizationState =
            serde_json::from_value(snapshot.parametrization).map_err(|err| {
                RunnerError::Engine(EngineError::engine(format!(
                    "failed to decode stage parametrization state: {err}"
                )))
            })?;
        let observable_state =
            ObservableState::from_json(&snapshot.observable_state).map_err(RunnerError::Engine)?;

        Ok(TransitionRuntimeState {
            sampler_snapshot: Some(runner_snapshot.engine),
            sampler_config,
            parametrization_state,
            observable_state,
        })
    }

    async fn ensure_task_runtime(
        &mut self,
        task: &RunTask,
        sampler_aggregator: &SamplerAggregatorConfig,
        parametrization: &ParametrizationConfig,
        open_batch_count: usize,
    ) -> Result<(), RunnerError> {
        if open_batch_count > 0 {
            return Ok(());
        }

        let transition_state = if self.active_runtime_task_id != Some(task.id) {
            let snapshot = self
                .aggregation_store
                .load_latest_stage_snapshot_before_sequence(self.run_id, task.sequence_nr)
                .await?;
            snapshot
                .map(Self::decode_transition_runtime_state)
                .transpose()?
        } else {
            None
        };

        if self.active_runtime_task_id != Some(task.id) {
            if let Some(state) = transition_state.as_ref() {
                self.current_sampler_config = state.sampler_config.clone();
                self.current_parametrization_state = state.parametrization_state.clone();
            }
            if let Some(observable_config) = task
                .task
                .explicit_observable_config(&self.observable_state.config())
            {
                self.observable_state = self
                    .evaluator_config
                    .empty_observable_state(&observable_config)
                    .map_err(RunnerError::Engine)?;
            } else if let Some(state) = transition_state.as_ref() {
                self.observable_state = state.observable_state.clone();
            }
        }

        let next_parametrization_state = Self::build_parametrization_state(
            parametrization,
            &self.point_spec,
            transition_state
                .as_ref()
                .and_then(|state| state.sampler_snapshot.as_ref()),
            transition_state
                .as_ref()
                .map(|state| &state.parametrization_state)
                .or(Some(&self.current_parametrization_state)),
        )?;

        if !Self::same_serialized(
            &self.current_parametrization_state,
            &next_parametrization_state,
        )? {
            self.current_parametrization_state_version += 1;
            self.parametrization_state_store
                .save_parametrization_version(
                    self.run_id,
                    self.current_parametrization_state_version,
                    &next_parametrization_state,
                )
                .await?;
            self.current_parametrization_state = next_parametrization_state;
        }

        let current_sampler_config = transition_state
            .as_ref()
            .map(|state| &state.sampler_config)
            .unwrap_or(&self.current_sampler_config);
        if !Self::same_serialized(current_sampler_config, sampler_aggregator)? {
            let sample_budget = task
                .task
                .nr_expected_samples()
                .and_then(|n| usize::try_from(n).ok());
            self.engine = match transition_state.and_then(|state| state.sampler_snapshot) {
                Some(snapshot) => sampler_aggregator
                    .build_from_params_and_snapshot(
                        self.point_spec.clone(),
                        sample_budget,
                        snapshot,
                    )
                    .map_err(RunnerError::Engine)?,
                None => sampler_aggregator
                    .build(self.point_spec.clone(), sample_budget)
                    .map_err(RunnerError::Engine)?,
            };
            self.current_sampler_config = sampler_aggregator.clone();
        }

        self.training_completion_marked = self.engine.training_samples_remaining().is_none();
        self.active_runtime_task_id = Some(task.id);
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

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            node_id: self.node_id.clone(),
            runtime_metrics: self
                .runtime_state
                .to_runtime_metrics(completed_samples_per_second),
            engine_diagnostics: self.engine.get_diagnostics(),
        };

        self.work_queue
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
        self.last_performance_completed_samples = self.nr_completed_samples;
        self.last_snapshot_at = Instant::now();
        Ok(())
    }

    async fn flush_run_sample_progress(&mut self) -> Result<(), RunnerError> {
        self.aggregation_store
            .save_run_sample_progress(
                self.run_id,
                self.nr_produced_samples,
                self.nr_completed_samples,
            )
            .await?;
        Ok(())
    }

    async fn save_stage_snapshot(
        &mut self,
        task: Option<&RunTask>,
        queue_empty: bool,
    ) -> Result<(), RunnerError> {
        let runner_snapshot = self.snapshot_state()?;
        let sampler_runner_snapshot = serde_json::to_value(&runner_snapshot)
            .map_err(|err| RunnerError::Engine(EngineError::from(err)))?;
        let observable_state = self
            .observable_state
            .to_json()
            .map_err(RunnerError::Engine)?;
        let persisted_observable = self
            .observable_state
            .to_persistent_json()
            .map_err(RunnerError::Engine)?;
        let sampler_aggregator = serde_json::to_value(&self.current_sampler_config)
            .map_err(|err| RunnerError::Engine(EngineError::from(err)))?;
        let parametrization = serde_json::to_value(&self.current_parametrization_state)
            .map_err(|err| RunnerError::Engine(EngineError::from(err)))?;
        self.aggregation_store
            .save_run_stage_snapshot(&RunStageSnapshot {
                run_id: self.run_id,
                task_id: task.map(|task| task.id),
                sequence_nr: task.map(|task| task.sequence_nr),
                queue_empty,
                sampler_runner_snapshot,
                observable_state,
                persisted_observable,
                sampler_aggregator,
                parametrization,
            })
            .await?;
        Ok(())
    }

    fn max_samples_to_produce_this_tick(
        &self,
        engine_max_samples: Option<usize>,
        task_max_samples: Option<usize>,
    ) -> Result<Option<usize>, RunnerError> {
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
        let Some(task) = self.current_task.as_ref() else {
            return Ok(None);
        };
        let Some(target) = task.task.nr_expected_samples() else {
            return Ok(None);
        };
        let remaining = target.saturating_sub(task.nr_produced_samples);
        if remaining < 0 {
            return Err(RunnerError::Engine(EngineError::engine(format!(
                "run {} task {} produced sample count exceeded target: produced={} target={}",
                self.run_id,
                task.id,
                task.nr_produced_samples,
                remaining + task.nr_produced_samples
            ))));
        }
        Ok(usize::try_from(remaining).ok())
    }

    fn observable_sample_count(&self) -> i64 {
        match &self.observable_state {
            ObservableState::Scalar(state) => state.count,
            ObservableState::Complex(state) => state.count,
            ObservableState::FullScalar(state) => state.values.len() as i64,
            ObservableState::FullComplex(state) => state.values.len() as i64,
        }
    }

    async fn clear_run_assignments_once(
        &mut self,
        reason: &'static str,
    ) -> Result<(), RunnerError> {
        if self.auto_stop_triggered {
            return Ok(());
        }
        let assignments_cleared = self.run_control.clear_run_assignments(self.run_id).await?;
        self.auto_stop_triggered = true;
        info!(
            run_id = self.run_id,
            nr_produced_samples = self.nr_produced_samples,
            nr_completed_samples = self.nr_completed_samples,
            observable_samples = self.observable_sample_count(),
            assignments_cleared,
            reason,
            "run assignments cleared"
        );
        Ok(())
    }

    async fn sync_active_task_progress(&mut self) -> Result<(), RunnerError> {
        let Some(task) = self.current_task.as_ref() else {
            return Ok(());
        };
        if task.task.nr_expected_samples().is_none() {
            return Ok(());
        }
        self.task_store
            .update_run_task_progress(task.id, task.nr_produced_samples, task.nr_completed_samples)
            .await?;
        Ok(())
    }

    async fn ensure_active_task(&mut self) -> Result<Option<RunTask>, RunnerError> {
        if let Some(task) = self.task_store.load_active_run_task(self.run_id).await? {
            self.current_task = Some(task.clone());
            return Ok(Some(task));
        }
        let next = self.task_store.activate_next_run_task(self.run_id).await?;
        self.current_task = next.clone();
        Ok(next)
    }

    async fn complete_current_task(&mut self) -> Result<(), RunnerError> {
        let Some(task) = self.current_task.take() else {
            return Ok(());
        };
        self.active_runtime_task_id = None;
        if task.task.nr_expected_samples().is_some() {
            self.task_store
                .update_run_task_progress(
                    task.id,
                    task.nr_produced_samples,
                    task.nr_completed_samples,
                )
                .await?;
        }
        self.task_store.complete_run_task(task.id).await?;
        Ok(())
    }

    async fn fail_current_task(&mut self, reason: String) -> Result<(), RunnerError> {
        if let Some(task) = self.current_task.take() {
            self.active_runtime_task_id = None;
            let queue_empty = self.work_queue.get_open_batch_count(self.run_id).await? <= 0;
            self.save_stage_snapshot(Some(&task), queue_empty).await?;
            self.task_store.fail_run_task(task.id, &reason).await?;
        }
        self.clear_run_assignments_once("run task failed; assignments cleared")
            .await?;
        info!(run_id = self.run_id, error = %reason, "run task failed");
        Ok(())
    }

    async fn run_task_tick(
        &mut self,
        pending_before_tick: usize,
        open_batch_count: usize,
    ) -> Result<usize, RunnerError> {
        for _ in 0..8 {
            let Some(task) = self.ensure_active_task().await? else {
                self.clear_run_assignments_once("run task queue exhausted; assignments cleared")
                    .await?;
                return Ok(0);
            };
            match task.task.clone() {
                RunTaskSpec::Sample { .. }
                | RunTaskSpec::Image { .. }
                | RunTaskSpec::PlotLine { .. } => {
                    let sampler_aggregator = task.task.sampler_config().ok_or_else(|| {
                        RunnerError::Engine(EngineError::engine("task missing sampler config"))
                    })?;
                    let parametrization = task.task.parametrization_config().ok_or_else(|| {
                        RunnerError::Engine(EngineError::engine(
                            "task missing parametrization config",
                        ))
                    })?;
                    if let Err(err) = self
                        .ensure_task_runtime(
                            &task,
                            &sampler_aggregator,
                            &parametrization,
                            open_batch_count,
                        )
                        .await
                    {
                        self.fail_current_task(format!(
                            "failed to activate sample task {}: {err}",
                            task.id
                        ))
                        .await?;
                        return Ok(0);
                    }
                    if let Some(target) = task.task.nr_expected_samples()
                        && task.nr_completed_samples >= target
                    {
                        if open_batch_count == 0 {
                            self.persist_snapshot().await?;
                            self.save_stage_snapshot(Some(&task), true).await?;
                            self.complete_current_task().await?;
                            continue;
                        }
                        return Ok(0);
                    }
                    return self.produce_for_active_task(pending_before_tick).await;
                }
                RunTaskSpec::Pause => {
                    if open_batch_count > 0 {
                        return Ok(0);
                    }
                    self.persist_snapshot().await?;
                    self.save_stage_snapshot(Some(&task), true).await?;
                    self.complete_current_task().await?;
                    self.clear_run_assignments_once("pause task reached; run assignments cleared")
                        .await?;
                    return Ok(0);
                }
            }
        }

        Err(RunnerError::Engine(EngineError::engine(
            "run task executor advanced too many times in a single tick",
        )))
    }

    async fn produce_for_active_task(
        &mut self,
        pending_before_tick: usize,
    ) -> Result<usize, RunnerError> {
        let observable_config = self.observable_state.config();
        let sample_plan = self.engine.sample_plan().map_err(RunnerError::Engine)?;
        let training_samples_remaining = self.engine.training_samples_remaining();
        let batch_plan = match sample_plan {
            SamplePlan::Pause => Vec::new(),
            SamplePlan::Produce { nr_samples } => {
                let base_produce_limit = self.compute_produce_limit(pending_before_tick);
                let requested = if nr_samples == usize::MAX {
                    None
                } else {
                    Some(nr_samples)
                };
                self.build_batch_plan(
                    base_produce_limit,
                    self.max_samples_to_produce_this_tick(
                        match requested {
                            Some(requested) => Some(
                                training_samples_remaining
                                    .map_or(requested, |remaining| remaining.min(requested)),
                            ),
                            None => training_samples_remaining,
                        },
                        self.active_sample_remaining_budget()?,
                    )?,
                )
            }
        };

        let mut produced = Vec::with_capacity(batch_plan.len());
        for nr_samples in batch_plan {
            let started = Instant::now();
            let requires_training = training_samples_remaining.is_some();
            let batch = self
                .engine
                .produce_latent_batch(nr_samples)
                .map_err(RunnerError::Engine)?;
            let produce_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            let produced_samples = batch.nr_samples;
            self.runtime_state.produced_batches_total += 1;
            self.runtime_state.produced_samples_total += produced_samples as i64;
            self.nr_produced_samples += produced_samples as i64;
            if let Some(task) = self.current_task.as_mut() {
                task.nr_produced_samples += produced_samples as i64;
            }
            if produced_samples > 0 {
                self.runtime_state
                    .rolling
                    .sampler_produce_ms_per_sample
                    .observe(produce_time_ms / produced_samples as f64);
            }
            produced.push((
                batch
                    .with_observable_config(observable_config.clone())
                    .with_version(self.current_parametrization_state_version),
                requires_training,
            ));
        }
        let enqueued_batches = produced.len();
        for (batch, requires_training) in produced {
            self.work_queue
                .insert_batch(self.run_id, &batch, requires_training)
                .await?;
        }
        let pending_after_enqueue = pending_before_tick.saturating_add(enqueued_batches);
        self.last_pending_after_enqueue = Some(pending_after_enqueue);
        self.sync_active_task_progress().await?;
        Ok(enqueued_batches)
    }
}
