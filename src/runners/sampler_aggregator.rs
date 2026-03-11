//! Sampler-aggregator runner orchestration.
//!
//! This module intentionally focuses on process orchestration:
//! - load persisted aggregated observable snapshot
//! - call engine hooks
//! - enqueue produced batches
//! - fetch completed batches and pass training weights back into the engine
//! - aggregate completed batch observables into run-level observable snapshot
//! - delete consumed completed batches

use crate::core::PointSpec;
use crate::core::{
    AggregationStore, CompletedBatch, RollingMetricSnapshot, SamplerAggregatorPerformanceSnapshot,
    SamplerRollingAverages, SamplerRuntimeMetrics, StoreError, WorkQueueStore,
};
use crate::engines::{EngineError, ObservableState, SamplerAggregator};
use crate::runners::rolling_metric::RollingMetric;
use crate::stores::RunControlStore;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerAggregatorRunnerParams {
    pub lease_ttl_ms: u64,
    pub min_poll_time_ms: u64,
    pub performance_snapshot_interval_ms: u64,
    pub target_batch_eval_ms: f64,
    pub target_queue_remaining: f64,
    pub max_batch_size: usize,
    pub max_queue_size: usize,
    pub max_batches_per_tick: usize,
    pub completed_batch_fetch_limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_on: Option<RunStopCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunStopCondition {
    SamplesAtLeast { samples: i64 },
}

#[derive(Debug, Clone)]
pub struct RunnerTick {
    pub enqueued_batches: usize,
    pub processed_completed_batches: usize,
    pub queue_depleted: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
struct RollingAveragesState {
    eval_ms_per_sample: RollingMetric,
    eval_ms_per_batch: RollingMetric,
    sampler_produce_ms_per_sample: RollingMetric,
    sampler_ingest_ms_per_sample: RollingMetric,
    queue_remaining_ratio: RollingMetric,
    batches_consumed_per_tick: RollingMetric,
}

#[derive(Debug, Clone, Serialize, Default)]
struct SamplerRuntimeState {
    produced_batches_total: i64,
    produced_samples_total: i64,
    ingested_batches_total: i64,
    ingested_samples_total: i64,
    batch_size_current: usize,
    rolling: RollingAveragesState,
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

pub struct SamplerAggregatorRunner<WQ, AS, RC> {
    run_id: i32,
    worker_id: String,
    engine: Box<dyn SamplerAggregator>,
    aggregated_observable: ObservableState,
    work_queue: WQ,
    aggregation_store: AS,
    run_control: RC,
    config: SamplerAggregatorRunnerParams,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    point_spec: PointSpec,
    runtime_state: SamplerRuntimeState,
    last_pending_after_enqueue: Option<usize>,
    training_completion_marked: bool,
    auto_stop_triggered: bool,
    auto_stop_waiting_for_depletion: bool,
}

impl<WQ, AS, RC> SamplerAggregatorRunner<WQ, AS, RC>
where
    WQ: WorkQueueStore,
    AS: AggregationStore,
    RC: RunControlStore,
{
    const MIN_BATCH_SIZE: usize = 1;
    const MAX_BATCH_SIZE_UP_FACTOR: f64 = 1.25;
    const MAX_BATCH_SIZE_DOWN_FACTOR: f64 = 0.80;
    const MIN_BATCH_SIZE_CHANGE_RATIO: f64 = 0.03;

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
        engine_max_samples: Option<usize>,
    ) -> Vec<usize> {
        match engine_max_samples {
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
        worker_id: impl Into<String>,
        engine: Box<dyn SamplerAggregator>,
        mut aggregated_observable: ObservableState,
        work_queue: WQ,
        aggregation_store: AS,
        run_control: RC,
        config: SamplerAggregatorRunnerParams,
        point_spec: PointSpec,
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
        if let Some(RunStopCondition::SamplesAtLeast { samples }) = config.stop_on
            && samples <= 0
        {
            return Err(RunnerError::Engine(EngineError::engine(
                "runner config stop_on.samples_at_least.samples must be > 0",
            )));
        }
        let performance_snapshot_interval =
            Duration::from_millis(config.performance_snapshot_interval_ms);
        let current_observable = aggregation_store.load_current_observable(run_id).await?;
        if let Some(snapshot) = current_observable {
            aggregated_observable =
                ObservableState::from_json(&snapshot).map_err(RunnerError::Engine)?;
        }

        Ok(Self {
            run_id,
            worker_id: worker_id.into(),
            engine,
            aggregated_observable,
            work_queue,
            aggregation_store,
            run_control,
            config,
            performance_snapshot_interval,
            last_snapshot_at: Instant::now(),
            point_spec,
            runtime_state: SamplerRuntimeState {
                batch_size_current: initial_batch_size,
                ..SamplerRuntimeState::default()
            },
            last_pending_after_enqueue: None,
            training_completion_marked: false,
            auto_stop_triggered: false,
            auto_stop_waiting_for_depletion: false,
        })
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

        let base_produce_limit = if self.stop_condition_met() {
            0
        } else {
            self.compute_produce_limit(pending_before_tick)
        };
        let engine_max_samples = self.engine.get_max_samples();
        let batch_plan = self.build_batch_plan(base_produce_limit, engine_max_samples);

        let mut produced = Vec::with_capacity(batch_plan.len());
        for nr_samples in batch_plan {
            let started = Instant::now();
            let requires_training = self.engine.is_training_active();
            let batch = self
                .engine
                .produce_batch(nr_samples)
                .map_err(RunnerError::Engine)?;
            let produce_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            let produced_samples = batch.size();
            self.runtime_state.produced_batches_total += 1;
            self.runtime_state.produced_samples_total += produced_samples as i64;
            if produced_samples > 0 {
                self.runtime_state
                    .rolling
                    .sampler_produce_ms_per_sample
                    .observe(produce_time_ms / produced_samples as f64);
            }
            produced.push((batch, requires_training));
        }
        for (batch, _) in &produced {
            batch
                .validate_point_spec(&self.point_spec)
                .map_err(|err| RunnerError::Engine(EngineError::engine(err.to_string())))?;
        }
        let enqueued_batches = produced.len();
        for (batch, requires_training) in produced {
            self.work_queue
                .insert_batch(self.run_id, &batch, requires_training)
                .await?;
        }
        let pending_after_enqueue = pending_before_tick.saturating_add(enqueued_batches);
        self.last_pending_after_enqueue = Some(pending_after_enqueue);
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
        self.flush_performance_snapshot().await?;
        self.maybe_stop_run_from_condition().await?;

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

        for batch in completed {
            if let Some(total_eval_time_ms) = batch.total_eval_time_ms
                && batch.batch.size() > 0
            {
                self.runtime_state
                    .rolling
                    .eval_ms_per_batch
                    .observe(total_eval_time_ms);
                self.runtime_state
                    .rolling
                    .eval_ms_per_sample
                    .observe(total_eval_time_ms / batch.batch.size() as f64);
            }
            if batch.requires_training {
                let training_weights = batch.result.values.as_deref().ok_or_else(|| {
                    RunnerError::Engine(EngineError::engine(format!(
                        "completed batch {} requires training but has no training values",
                        batch.batch_id
                    )))
                })?;
                let ingest_started = Instant::now();
                self.engine
                    .ingest_training_weights(training_weights)
                    .map_err(RunnerError::Engine)?;
                let ingest_time_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
                let ingested_samples = training_weights.len();
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
        if self.training_completion_marked || self.engine.is_training_active() {
            return Ok(());
        }
        let _ = self
            .work_queue
            .try_set_training_completed_at(self.run_id)
            .await?;
        self.training_completion_marked = true;
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

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            worker_id: self.worker_id.clone(),
            runtime_metrics: self.runtime_state.to_runtime_metrics(),
            engine_diagnostics: self.engine.get_diagnostics(),
        };

        self.work_queue
            .record_sampler_performance_snapshot(&snapshot)
            .await?;
        self.last_snapshot_at = Instant::now();
        Ok(())
    }

    fn stop_condition_met(&self) -> bool {
        match self.config.stop_on {
            Some(RunStopCondition::SamplesAtLeast { samples }) => {
                self.aggregated_sample_count() >= samples
            }
            None => false,
        }
    }

    fn aggregated_sample_count(&self) -> i64 {
        match &self.aggregated_observable {
            ObservableState::Scalar(state) => state.count,
            ObservableState::Complex(state) => state.count,
        }
    }

    async fn maybe_stop_run_from_condition(&mut self) -> Result<(), RunnerError> {
        if self.auto_stop_triggered || !self.stop_condition_met() {
            return Ok(());
        }

        let pending = self
            .work_queue
            .get_pending_batch_count(self.run_id)
            .await?
            .max(0);
        if pending > 0 {
            if !self.auto_stop_waiting_for_depletion {
                info!(
                    run_id = self.run_id,
                    aggregated_samples = self.aggregated_sample_count(),
                    pending_batches = pending,
                    ?self.config.stop_on,
                    "auto-stop condition reached; halting production and waiting for queue depletion"
                );
                self.auto_stop_waiting_for_depletion = true;
            }
            return Ok(());
        }

        //let assignments_cleared = self
        //    .run_control
        //    .stop_run_and_clear_assignments(self.run_id)
        //    .await?;
        self.auto_stop_triggered = true;
        //info!(
        //    run_id = self.run_id,
        //    aggregated_samples = self.aggregated_sample_count(),
        //    ?self.config.stop_on,
        //    assignments_cleared,
        //    "auto-stop condition reached"
        //);
        Ok(())
    }
}
