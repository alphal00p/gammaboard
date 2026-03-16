//! Sampler-aggregator runner orchestration.
//!
//! This module intentionally focuses on process orchestration:
//! - load persisted aggregated observable snapshot
//! - call engine hooks
//! - enqueue produced batches
//! - fetch completed batches and pass training weights back into the engine
//! - aggregate completed batch observables into run-level observable snapshot
//! - delete consumed completed batches

use crate::core::{
    AggregationStore, CompletedBatch, ParametrizationVersionStore, RollingMetricSnapshot,
    RunSampleProgress, RunTask, RunTaskSpec, RunTaskStore, SamplerAggregatorPerformanceSnapshot,
    SamplerRollingAverages, SamplerRuntimeMetrics, StoreError, WorkQueueStore,
};
use crate::engines::{
    EngineError, ObservableState, ParametrizationConfig, PointSpec, SamplePlan, SamplerAggregator,
    SamplerAggregatorSnapshot,
};
use crate::runners::rolling_metric::RollingMetric;
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
    pub aggregated_observable: JsonValue,
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

    fn to_runtime_metrics(&self) -> SamplerRuntimeMetrics {
        SamplerRuntimeMetrics {
            produced_batches_total: self.produced_batches_total,
            produced_samples_total: self.produced_samples_total,
            ingested_batches_total: self.ingested_batches_total,
            ingested_samples_total: self.ingested_samples_total,
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
    aggregated_observable: ObservableState,
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
    last_pending_after_enqueue: Option<usize>,
    training_completion_marked: bool,
    auto_stop_triggered: bool,
    current_task: Option<RunTask>,
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
        mut aggregated_observable: ObservableState,
        work_queue: WQ,
        aggregation_store: AS,
        run_control: RC,
        parametrization_state_store: PS,
        task_store: TS,
        config: SamplerAggregatorRunnerParams,
        point_spec: PointSpec,
        initial_parametrization_state: ParametrizationConfig,
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
            aggregated_observable =
                ObservableState::from_json(&snapshot).map_err(RunnerError::Engine)?;
        }
        let persisted_progress = aggregation_store
            .load_run_sample_progress(run_id)
            .await?
            .unwrap_or(RunSampleProgress {
                target_nr_samples: None,
                nr_produced_samples: 0,
                nr_completed_samples: 0,
            });
        let latest_version = parametrization_state_store
            .load_latest_parametrization_version(run_id)
            .await?;
        let current_parametrization_state_version = match latest_version {
            Some(version) => version,
            None => {
                let version = 1;
                parametrization_state_store
                    .save_parametrization_version(run_id, version, &initial_parametrization_state)
                    .await?;
                version
            }
        };

        Ok(Self {
            run_id,
            node_id: node_id.into(),
            engine,
            aggregated_observable,
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
            last_pending_after_enqueue: None,
            training_completion_marked: false,
            auto_stop_triggered: false,
            current_task: None,
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
        self.aggregated_observable = ObservableState::from_json(&snapshot.aggregated_observable)
            .map_err(RunnerError::Engine)?;
        self.current_parametrization_state_version = snapshot.current_parametrization_state_version;
        self.runtime_state = snapshot.runtime_state;
        self.runtime_state.produced_samples_total = self
            .runtime_state
            .produced_samples_total
            .max(self.nr_produced_samples);
        self.last_pending_after_enqueue = snapshot.last_pending_after_enqueue;
        self.training_completion_marked = snapshot.training_completion_marked;
        self.auto_stop_triggered = snapshot.auto_stop_triggered;
        Ok(())
    }

    pub fn snapshot_state(&mut self) -> Result<SamplerAggregatorRunnerSnapshot, RunnerError> {
        Ok(SamplerAggregatorRunnerSnapshot {
            version: Self::SNAPSHOT_VERSION,
            engine: self.engine.snapshot().map_err(RunnerError::Engine)?,
            aggregated_observable: self
                .aggregated_observable
                .to_json()
                .map_err(RunnerError::Engine)?,
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

            self.aggregated_observable
                .merge(batch.result.observable.clone())
                .map_err(RunnerError::Engine)?;
        }
        self.nr_completed_samples += completed_samples_delta;
        if let Some(task) = self.current_task.as_mut()
            && matches!(task.task, RunTaskSpec::Sample { .. })
        {
            task.nr_completed_samples += completed_samples_delta;
        }

        let current_observable = self
            .aggregated_observable
            .to_json()
            .map_err(RunnerError::Engine)?;
        let snapshot = self
            .aggregated_observable
            .to_persistent_json()
            .map_err(RunnerError::Engine)?;

        self.aggregation_store
            .save_aggregation(
                self.run_id,
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

    async fn resolve_sample_plan(&mut self) -> Result<SamplePlan, RunnerError> {
        for _ in 0..8 {
            match self.engine.sample_plan().map_err(RunnerError::Engine)? {
                SamplePlan::Advance { state } => {
                    let state: ParametrizationConfig =
                        serde_json::from_value(state).map_err(|err| {
                            RunnerError::Engine(EngineError::build(format!(
                                "failed to decode parametrization config from sampler plan: {err}"
                            )))
                        })?;
                    self.current_parametrization_state_version += 1;
                    self.parametrization_state_store
                        .save_parametrization_version(
                            self.run_id,
                            self.current_parametrization_state_version,
                            &state,
                        )
                        .await?;
                }
                plan => return Ok(plan),
            }
        }

        Err(RunnerError::Engine(EngineError::engine(
            "sampler emitted too many consecutive parametrization advances",
        )))
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

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            node_id: self.node_id.clone(),
            runtime_metrics: self.runtime_state.to_runtime_metrics(),
            engine_diagnostics: self.engine.get_diagnostics(),
        };

        self.work_queue
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
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
        let RunTaskSpec::Sample { nr_samples } = &task.task else {
            return Ok(None);
        };
        let Some(target) = nr_samples else {
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

    fn aggregated_sample_count(&self) -> i64 {
        match &self.aggregated_observable {
            ObservableState::Scalar(state) => state.count,
            ObservableState::Complex(state) => state.count,
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
            aggregated_samples = self.aggregated_sample_count(),
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
        if !matches!(task.task, RunTaskSpec::Sample { .. }) {
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
        if matches!(task.task, RunTaskSpec::Sample { .. }) {
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
                RunTaskSpec::Sample { nr_samples } => {
                    if let Some(target) = nr_samples
                        && task.nr_completed_samples >= target
                    {
                        if open_batch_count == 0 {
                            self.complete_current_task().await?;
                            continue;
                        }
                        return Ok(0);
                    }
                    return self
                        .produce_for_active_sample_task(pending_before_tick)
                        .await;
                }
                RunTaskSpec::ReconfigureParametrization { config } => {
                    if open_batch_count > 0 {
                        return Ok(0);
                    }
                    self.current_parametrization_state_version += 1;
                    self.parametrization_state_store
                        .save_parametrization_version(
                            self.run_id,
                            self.current_parametrization_state_version,
                            &config,
                        )
                        .await?;
                    self.complete_current_task().await?;
                    continue;
                }
                RunTaskSpec::ReconfigureSampler { config } => {
                    if open_batch_count > 0 {
                        return Ok(0);
                    }
                    match self.engine.transition(&config, &self.point_spec) {
                        Ok(engine) => {
                            self.engine = engine;
                            self.training_completion_marked =
                                self.engine.training_samples_remaining().is_none();
                            self.complete_current_task().await?;
                            continue;
                        }
                        Err(err) => {
                            self.fail_current_task(format!(
                                "failed to transition sampler for task {}: {err}",
                                task.id
                            ))
                            .await?;
                            return Ok(0);
                        }
                    }
                }
                RunTaskSpec::Pause => {
                    if open_batch_count > 0 {
                        return Ok(0);
                    }
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

    async fn produce_for_active_sample_task(
        &mut self,
        pending_before_tick: usize,
    ) -> Result<usize, RunnerError> {
        let sample_plan = self.resolve_sample_plan().await?;
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
            SamplePlan::Advance { .. } => {
                return Err(RunnerError::Engine(EngineError::engine(
                    "unresolved SamplePlan::Advance reached batch production",
                )));
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
                batch.with_version(self.current_parametrization_state_version),
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
