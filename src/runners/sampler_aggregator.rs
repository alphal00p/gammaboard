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
use crate::engines::{BatchContext, EngineError, Observable, ObservableConfig, SamplerAggregator};
use crate::runners::rolling_metric::RollingMetric;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use thiserror::Error;

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

pub struct SamplerAggregatorRunner<WQ, AS> {
    run_id: i32,
    worker_id: String,
    engine: Box<dyn SamplerAggregator>,
    aggregated_observable: Box<dyn Observable>,
    observable_config: ObservableConfig,
    work_queue: WQ,
    aggregation_store: AS,
    config: SamplerAggregatorRunnerParams,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    point_spec: PointSpec,
    local_batch_contexts: HashMap<i64, BatchContext>,
    runtime_state: SamplerRuntimeState,
    last_pending_after_enqueue: Option<usize>,
    training_completion_marked: bool,
}

impl<WQ, AS> SamplerAggregatorRunner<WQ, AS>
where
    WQ: WorkQueueStore,
    AS: AggregationStore,
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
        observable_config: ObservableConfig,
        work_queue: WQ,
        aggregation_store: AS,
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
        let performance_snapshot_interval =
            Duration::from_millis(config.performance_snapshot_interval_ms);
        let persisted_snapshot = aggregation_store
            .load_latest_aggregation_snapshot(run_id)
            .await?;

        let mut aggregated_observable = observable_config
            .build()
            .map_err(|err| RunnerError::Engine(err))?;

        if let Some(snapshot) = persisted_snapshot {
            aggregated_observable
                .load_state_from_json(&snapshot)
                .map_err(RunnerError::Engine)?;
        }

        Ok(Self {
            run_id,
            worker_id: worker_id.into(),
            engine,
            aggregated_observable,
            observable_config,
            work_queue,
            aggregation_store,
            config,
            performance_snapshot_interval,
            last_snapshot_at: Instant::now(),
            point_spec,
            local_batch_contexts: HashMap::new(),
            runtime_state: SamplerRuntimeState {
                batch_size_current: initial_batch_size,
                ..SamplerRuntimeState::default()
            },
            last_pending_after_enqueue: None,
            training_completion_marked: false,
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

        let base_produce_limit = self.compute_produce_limit(pending_before_tick);
        let engine_max_samples = self.engine.get_max_samples();
        let batch_plan = self.build_batch_plan(base_produce_limit, engine_max_samples);

        let mut produced = Vec::with_capacity(batch_plan.len());
        for nr_samples in batch_plan {
            let started = Instant::now();
            let requires_training = self.engine.is_training_active();
            let (batch, context) = self
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
            produced.push((batch, context, requires_training));
        }
        for (batch, _, _) in &produced {
            batch
                .validate_point_spec(&self.point_spec)
                .map_err(|err| RunnerError::Engine(EngineError::engine(err.to_string())))?;
        }
        let enqueued_batches = produced.len();
        for (batch, context, requires_training) in produced {
            let batch_id = self
                .work_queue
                .insert_batch(self.run_id, &batch, requires_training)
                .await?;
            if let Some(context) = context {
                self.local_batch_contexts.insert(batch_id, context);
            }
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
                let context = self.local_batch_contexts.remove(&batch.batch_id);
                let ingest_started = Instant::now();
                self.engine
                    .ingest_training_weights(training_weights, context)
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
            } else {
                self.local_batch_contexts.remove(&batch.batch_id);
            }

            let mut observable = self
                .observable_config
                .build()
                .map_err(RunnerError::Engine)?;
            observable
                .load_state_from_json(&batch.result.observable)
                .map_err(RunnerError::Engine)?;
            self.aggregated_observable
                .merge(observable.as_ref())
                .map_err(RunnerError::Engine)?;
        }

        let snapshot = self
            .aggregated_observable
            .snapshot()
            .map_err(RunnerError::Engine)?;

        self.aggregation_store
            .save_aggregation_snapshot(self.run_id, &snapshot, completed.len() as i32)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::StoreError;
    use crate::core::{Batch, BatchResult, PointSpec};
    use crate::engines::{BatchContext, BuildError, ObservableConfig, SamplerAggregator};
    use crate::runners::test_support::MockWorkQueue;
    use serde_json::{Value as JsonValue, json};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct AggregationStoreData {
        initial_snapshot: Option<JsonValue>,
        saved: Vec<(i32, JsonValue, i32)>,
    }

    #[derive(Clone, Default)]
    struct TestAggregationStore {
        inner: Arc<Mutex<AggregationStoreData>>,
    }

    #[async_trait::async_trait]
    impl AggregationStore for TestAggregationStore {
        async fn load_latest_aggregation_snapshot(
            &self,
            _run_id: i32,
        ) -> Result<Option<JsonValue>, StoreError> {
            Ok(self.inner.lock().expect("poison").initial_snapshot.clone())
        }

        async fn save_aggregation_snapshot(
            &self,
            run_id: i32,
            aggregated_observable: &JsonValue,
            delta_batches_completed: i32,
        ) -> Result<(), StoreError> {
            self.inner.lock().expect("poison").saved.push((
                run_id,
                aggregated_observable.clone(),
                delta_batches_completed,
            ));
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct Probe {
        ingested_training_sizes: Vec<usize>,
        ingested_context_tokens: Vec<Option<usize>>,
        produce_requested_nr_samples: Vec<usize>,
    }

    struct TestContext {
        token: usize,
    }

    struct TestEngine {
        produced: Vec<(Batch, Option<usize>)>,
        probe: Arc<Mutex<Probe>>,
        max_samples: Option<usize>,
    }

    impl SamplerAggregator for TestEngine {
        fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
            Ok(())
        }

        fn get_max_samples(&self) -> Option<usize> {
            self.max_samples
        }

        fn produce_batch(
            &mut self,
            nr_samples: usize,
        ) -> Result<(Batch, Option<BatchContext>), EngineError> {
            self.probe
                .lock()
                .expect("poison")
                .produce_requested_nr_samples
                .push(nr_samples);
            if self.produced.is_empty() {
                return Err(EngineError::engine("test engine has no more batches"));
            }
            let (batch, context_token) = self.produced.remove(0);
            let context = context_token.map(|token| {
                let ctx: BatchContext = Box::new(TestContext { token });
                ctx
            });
            Ok((batch, context))
        }

        fn ingest_training_weights(
            &mut self,
            training_weights: &[f64],
            context: Option<BatchContext>,
        ) -> Result<(), EngineError> {
            let token = if let Some(context) = context {
                let context = context
                    .downcast::<TestContext>()
                    .map_err(|_| EngineError::engine("unexpected test context type"))?;
                Some(context.token)
            } else {
                None
            };

            let mut guard = self.probe.lock().expect("poison");
            guard.ingested_training_sizes.push(training_weights.len());
            guard.ingested_context_tokens.push(token);
            Ok(())
        }
    }

    fn make_batch() -> Batch {
        Batch::from_flat_data(1, 1, 0, vec![1.0], vec![]).expect("batch")
    }

    fn scalar_observable_config() -> ObservableConfig {
        ObservableConfig::Scalar {
            params: serde_json::Map::new(),
        }
    }

    fn make_completed(
        batch_id: i64,
        training_weights: Vec<f64>,
        observable_sum_weighted_value: f64,
    ) -> CompletedBatch {
        CompletedBatch {
            batch_id,
            batch: make_batch(),
            requires_training: true,
            result: BatchResult::new(
                Some(training_weights.clone()),
                json!({
                    "count": training_weights.len() as i64,
                    "sum_weighted_value": observable_sum_weighted_value,
                    "sum_abs": 0.0,
                    "sum_sq": 0.0,
                }),
            ),
            completed_at: None,
            total_eval_time_ms: Some(10.0),
        }
    }

    #[tokio::test]
    async fn tick_processes_completed_batches_persists_snapshot_and_deletes_consumed() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        {
            let mut q = queue.inner.lock().expect("poison");
            q.next_insert_batch_id = 10;
            q.completed = vec![
                make_completed(11, vec![0.1, 0.2], 0.3),
                make_completed(12, vec![0.4], 0.4),
            ];
        }

        let aggregation_store = TestAggregationStore::default();
        aggregation_store
            .inner
            .lock()
            .expect("poison")
            .initial_snapshot = Some(json!({
            "count": 3,
            "sum_weighted_value": 1.0,
            "sum_abs": 0.0,
            "sum_sq": 0.0,
        }));

        let engine = TestEngine {
            produced: vec![(make_batch(), Some(7))],
            probe: probe.clone(),
            max_samples: None,
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue.clone(),
            aggregation_store.clone(),
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 0.0,
                max_batch_size: 64,
                max_queue_size: 8,
                max_batches_per_tick: 1,
                completed_batch_fetch_limit: 128,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let q = queue.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();
        let agg_saved = aggregation_store
            .inner
            .lock()
            .expect("poison")
            .saved
            .clone();

        assert_eq!(q.inserted.len(), 1);
        assert_eq!(q.sampler_perf_snapshots.len(), 1);
        let perf = &q.sampler_perf_snapshots[0];
        let metrics = perf.runtime_metrics.to_performance_metrics();
        assert_eq!(perf.run_id, 1);
        assert_eq!(perf.worker_id, "worker-a");
        assert_eq!(metrics.produced_batches, 1);
        assert_eq!(metrics.produced_samples, 1);
        assert_eq!(metrics.ingested_batches, 2);
        assert_eq!(metrics.ingested_samples, 3);
        assert!(metrics.avg_produce_time_per_sample_ms >= 0.0);
        assert!(metrics.avg_ingest_time_per_sample_ms >= 0.0);
        assert_eq!(p.ingested_training_sizes, vec![2, 1]);
        assert_eq!(p.ingested_context_tokens, vec![Some(7), None]);
        assert_eq!(q.deleted_completed_batch_ids, vec![11, 12]);
        assert_eq!(tick.processed_completed_batches, 2);

        assert_eq!(agg_saved.len(), 1);
        assert_eq!(agg_saved[0].0, 1);
        assert_eq!(agg_saved[0].2, 2);
        assert_eq!(
            agg_saved[0].1.get("count").and_then(|value| value.as_i64()),
            Some(6)
        );
        let sum_weighted_value = agg_saved[0]
            .1
            .get("sum_weighted_value")
            .and_then(|value| value.as_f64())
            .expect("sum_weighted_value f64");
        assert!((sum_weighted_value - 1.7).abs() < 1e-12);
    }

    #[tokio::test]
    async fn tick_without_completed_batches_skips_snapshot_persist_and_delete() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").pending_batches = 1;
        let aggregation_store = TestAggregationStore::default();

        let engine = TestEngine {
            produced: vec![],
            probe: probe.clone(),
            max_samples: None,
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue.clone(),
            aggregation_store.clone(),
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 0.0,
                max_batch_size: 64,
                max_queue_size: 1,
                max_batches_per_tick: 1,
                completed_batch_fetch_limit: 64,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let p = probe.lock().expect("poison").clone();
        let agg_saved = aggregation_store
            .inner
            .lock()
            .expect("poison")
            .saved
            .clone();
        let q = queue.inner.lock().expect("poison").clone();

        assert_eq!(tick.processed_completed_batches, 0);
        assert!(p.ingested_training_sizes.is_empty());
        assert!(p.ingested_context_tokens.is_empty());
        assert!(q.sampler_perf_snapshots.is_empty());
        assert!(agg_saved.is_empty());
        assert!(q.deleted_completed_batch_ids.is_empty());
    }

    #[tokio::test]
    async fn tick_skips_batch_production_when_pending_queue_is_full() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").pending_batches = 5;

        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![(make_batch(), None), (make_batch(), None)],
            probe: probe.clone(),
            max_samples: None,
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue.clone(),
            aggregation_store,
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 0.0,
                max_batch_size: 64,
                max_queue_size: 5,
                max_batches_per_tick: 2,
                completed_batch_fetch_limit: 64,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let q = queue.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();

        assert_eq!(tick.enqueued_batches, 0);
        assert!(q.inserted.is_empty());
        assert!(q.sampler_perf_snapshots.is_empty());
        assert!(p.produce_requested_nr_samples.is_empty());
    }

    #[tokio::test]
    async fn tick_limits_batch_production_to_remaining_pending_capacity() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").pending_batches = 3;

        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![
                (make_batch(), None),
                (make_batch(), None),
                (make_batch(), None),
            ],
            probe: probe.clone(),
            max_samples: None,
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue.clone(),
            aggregation_store,
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 0.0,
                max_batch_size: 64,
                max_queue_size: 4,
                max_batches_per_tick: 3,
                completed_batch_fetch_limit: 64,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let q = queue.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();

        assert_eq!(p.produce_requested_nr_samples, vec![64]);
        assert_eq!(tick.enqueued_batches, 1);
        assert_eq!(q.inserted.len(), 1);
        assert_eq!(q.sampler_perf_snapshots.len(), 1);
        let metrics = q.sampler_perf_snapshots[0]
            .runtime_metrics
            .to_performance_metrics();
        assert_eq!(metrics.produced_batches, 1);
        assert_eq!(metrics.ingested_batches, 0);
    }

    #[tokio::test]
    async fn tick_balances_batch_plan_under_engine_sample_cap() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![
                (make_batch(), None),
                (make_batch(), None),
                (make_batch(), None),
                (make_batch(), None),
            ],
            probe: probe.clone(),
            max_samples: Some(16),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue,
            aggregation_store,
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 0.0,
                max_batch_size: 64,
                max_queue_size: 16,
                max_batches_per_tick: 4,
                completed_batch_fetch_limit: 64,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");
        runner.runtime_state.batch_size_current = 5;

        runner.tick().await.expect("tick");
        let p = probe.lock().expect("poison").clone();
        assert_eq!(p.produce_requested_nr_samples, vec![4, 4, 4, 4]);
    }

    #[tokio::test]
    async fn tick_throttles_enqueue_to_target_queue_remaining() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![(make_batch(), None); 16],
            probe: probe.clone(),
            max_samples: None,
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue.clone(),
            aggregation_store,
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 0.0,
                max_batch_size: 64,
                max_queue_size: 10,
                max_batches_per_tick: 8,
                completed_batch_fetch_limit: 64,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");

        let first_tick = runner.tick().await.expect("first tick");
        assert_eq!(first_tick.enqueued_batches, 8);

        {
            let mut q = queue.inner.lock().expect("poison");
            // Simulate evaluator draining 6/8 batches before next tick.
            q.pending_batches = 2;
        }

        let second_tick = runner.tick().await.expect("second tick");
        // consumed_per_tick ~= 6, target_queue_remaining=0.0 -> keep ~6 pending,
        // so enqueue only 4 when pending_before=2.
        assert_eq!(second_tick.enqueued_batches, 4);
    }

    #[tokio::test]
    async fn tick_with_target_queue_remaining_one_fills_to_hard_limit() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![(make_batch(), None); 16],
            probe: probe.clone(),
            max_samples: None,
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            scalar_observable_config(),
            queue.clone(),
            aggregation_store,
            SamplerAggregatorRunnerParams {
                lease_ttl_ms: 5_000,
                min_poll_time_ms: 500,
                performance_snapshot_interval_ms: 0,
                target_batch_eval_ms: 10.0,
                target_queue_remaining: 1.0,
                max_batch_size: 64,
                max_queue_size: 10,
                max_batches_per_tick: 8,
                completed_batch_fetch_limit: 64,
            },
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
        )
        .await
        .expect("new runner");

        let first_tick = runner.tick().await.expect("first tick");
        assert_eq!(first_tick.enqueued_batches, 8);

        {
            let mut q = queue.inner.lock().expect("poison");
            // Simulate evaluator draining 6/8 batches before next tick.
            q.pending_batches = 2;
        }

        let second_tick = runner.tick().await.expect("second tick");
        // target_queue_remaining=1.0 means no lean-throttle: fill to hard limits.
        assert_eq!(second_tick.enqueued_batches, 8);
    }
}
