//! Sampler-aggregator runner orchestration.
//!
//! This module intentionally focuses on process orchestration:
//! - load persisted aggregated observable snapshot
//! - call engine hooks
//! - enqueue produced batches
//! - fetch completed batches and pass training weights back into the engine
//! - aggregate completed batch observables into run-level observable snapshot
//! - delete consumed completed batches

use crate::batch::PointSpec;
use crate::core::{
    AggregationStore, CompletedBatch, SamplerAggregatorPerformanceSnapshot, StoreError,
    WorkQueueStore,
};
use crate::engines::observable::ObservableFactory;
use crate::engines::{BatchContext, EngineError, Observable, SamplerAggregator};
use crate::runners::sample_time_stats::SampleTimeStats;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, error::Error, fmt, time::Duration, time::Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SamplerAggregatorRunnerParams {
    pub interval_ms: u64,
    pub lease_ttl_ms: u64,
    pub nr_samples: usize,
    pub performance_snapshot_interval_ms: u64,
    pub max_pending_batches: usize,
    pub max_batches_per_tick: usize,
    pub completed_batch_fetch_limit: usize,
}

impl Default for SamplerAggregatorRunnerParams {
    fn default() -> Self {
        Self {
            interval_ms: 500,
            lease_ttl_ms: 5_000,
            nr_samples: 64,
            performance_snapshot_interval_ms: 5_000,
            max_pending_batches: 128,
            max_batches_per_tick: 1,
            completed_batch_fetch_limit: 512,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub nr_samples: usize,
    pub performance_snapshot_interval_ms: u64,
    pub max_batches_per_tick: usize,
    pub max_pending_batches: usize,
    pub completed_batch_fetch_limit: usize,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            nr_samples: 64,
            performance_snapshot_interval_ms: 5_000,
            max_batches_per_tick: 16,
            max_pending_batches: 4096,
            completed_batch_fetch_limit: 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunnerTick {
    pub enqueued_batches: usize,
    pub processed_completed_batches: usize,
}

#[derive(Debug)]
pub enum RunnerError {
    Engine(EngineError),
    Store(StoreError),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunnerError::Engine(err) => write!(f, "{err}"),
            RunnerError::Store(err) => write!(f, "{err}"),
        }
    }
}

impl Error for RunnerError {}

impl From<StoreError> for RunnerError {
    fn from(value: StoreError) -> Self {
        RunnerError::Store(value)
    }
}

pub struct SamplerAggregatorRunner<WQ, AS> {
    run_id: i32,
    worker_id: String,
    engine: Box<dyn SamplerAggregator>,
    aggregated_observable: Box<dyn Observable>,
    observable_factory: ObservableFactory,
    work_queue: WQ,
    aggregation_store: AS,
    config: RunnerConfig,
    point_spec: PointSpec,
    local_batch_contexts: HashMap<i64, BatchContext>,
    performance_snapshot_interval: Duration,
    perf_window_started_at: Instant,
    perf_window_started_ts: chrono::DateTime<Utc>,
    perf_produce: SampleTimeStats,
    perf_ingest: SampleTimeStats,
}

impl<WQ, AS> SamplerAggregatorRunner<WQ, AS>
where
    WQ: WorkQueueStore,
    AS: AggregationStore,
{
    pub async fn new(
        run_id: i32,
        worker_id: impl Into<String>,
        engine: Box<dyn SamplerAggregator>,
        observable_factory: ObservableFactory,
        work_queue: WQ,
        aggregation_store: AS,
        config: RunnerConfig,
        point_spec: PointSpec,
    ) -> Result<Self, RunnerError> {
        if config.nr_samples == 0 {
            return Err(RunnerError::Engine(EngineError::engine(
                "runner config nr_samples must be > 0",
            )));
        }

        let persisted_snapshot = aggregation_store
            .load_latest_aggregation_snapshot(run_id)
            .await?;

        let mut aggregated_observable = observable_factory
            .build()
            .map_err(|err| RunnerError::Engine(err))?;
        let performance_snapshot_interval =
            Duration::from_millis(config.performance_snapshot_interval_ms);

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
            observable_factory,
            work_queue,
            aggregation_store,
            config,
            point_spec,
            local_batch_contexts: HashMap::new(),
            performance_snapshot_interval,
            perf_window_started_at: Instant::now(),
            perf_window_started_ts: Utc::now(),
            perf_produce: SampleTimeStats::default(),
            perf_ingest: SampleTimeStats::default(),
        })
    }

    pub async fn tick(&mut self) -> Result<RunnerTick, RunnerError> {
        let pending_batches = self
            .work_queue
            .get_pending_batch_count(self.run_id)
            .await?
            .max(0) as usize;
        let remaining_capacity = self
            .config
            .max_pending_batches
            .saturating_sub(pending_batches);
        let engine_max_batches = self.engine.get_max_batches().unwrap_or(usize::MAX);
        let produce_limit = self
            .config
            .max_batches_per_tick
            .min(remaining_capacity)
            .min(engine_max_batches);

        let mut produced = Vec::with_capacity(produce_limit);
        for _ in 0..produce_limit {
            let started = Instant::now();
            let next = self
                .engine
                .produce_batch(self.config.nr_samples)
                .map_err(RunnerError::Engine)?;
            let produce_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            let produced_samples = next.0.size();
            self.perf_produce.observe(produced_samples, produce_time_ms);
            produced.push(next);
        }
        for (batch, _) in &produced {
            batch
                .validate_point_spec(&self.point_spec)
                .map_err(|err| RunnerError::Engine(EngineError::engine(err.to_string())))?;
        }
        let enqueued_batches = produced.len();
        for (batch, context) in produced {
            let batch_id = self.work_queue.insert_batch(self.run_id, &batch).await?;
            if let Some(context) = context {
                self.local_batch_contexts.insert(batch_id, context);
            }
        }

        let completed = self
            .work_queue
            .fetch_completed_batches(self.run_id, self.config.completed_batch_fetch_limit)
            .await?;
        let consumed_ids = self.process_completed(&completed).await?;
        self.work_queue
            .delete_completed_batches(&consumed_ids)
            .await?;
        self.flush_performance_snapshot_if_due(false).await?;

        Ok(RunnerTick {
            enqueued_batches,
            processed_completed_batches: consumed_ids.len(),
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
            let context = self.local_batch_contexts.remove(&batch.batch_id);
            let ingest_started = Instant::now();
            self.engine
                .ingest_training_weights(&batch.result.values, context)
                .map_err(RunnerError::Engine)?;
            let ingest_time_ms = ingest_started.elapsed().as_secs_f64() * 1000.0;
            let ingested_samples = batch.result.values.len();
            self.perf_ingest.observe(ingested_samples, ingest_time_ms);

            let mut observable = self
                .observable_factory
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

    async fn flush_performance_snapshot_if_due(&mut self, force: bool) -> Result<(), RunnerError> {
        if !self.perf_produce.has_data() && !self.perf_ingest.has_data() {
            return Ok(());
        }

        let due = if self.performance_snapshot_interval.is_zero() {
            true
        } else {
            self.perf_window_started_at.elapsed() >= self.performance_snapshot_interval
        };
        if !force && !due {
            return Ok(());
        }

        let snapshot = SamplerAggregatorPerformanceSnapshot {
            run_id: self.run_id,
            worker_id: self.worker_id.clone(),
            window_start: self.perf_window_started_ts,
            window_end: Utc::now(),
            produced_batches: self.perf_produce.batches(),
            produced_samples: self.perf_produce.samples(),
            avg_produce_time_per_sample_ms: self.perf_produce.mean(),
            std_produce_time_per_sample_ms: self.perf_produce.std(),
            ingested_batches: self.perf_ingest.batches(),
            ingested_samples: self.perf_ingest.samples(),
            avg_ingest_time_per_sample_ms: self.perf_ingest.mean(),
            std_ingest_time_per_sample_ms: self.perf_ingest.std(),
            diagnostics: self.engine.get_diagnostics(),
        };

        self.work_queue
            .record_sampler_performance_snapshot(&snapshot)
            .await?;

        self.perf_window_started_at = Instant::now();
        self.perf_window_started_ts = Utc::now();
        self.perf_produce = SampleTimeStats::default();
        self.perf_ingest = SampleTimeStats::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{Batch, BatchResult, PointSpec};
    use crate::core::StoreError;
    use crate::engines::{BatchContext, BuildError, SamplerAggregator};
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
    }

    impl SamplerAggregator for TestEngine {
        fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
            Ok(())
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

    fn make_completed(
        batch_id: i64,
        training_weights: Vec<f64>,
        observable_sum_weight: f64,
    ) -> CompletedBatch {
        CompletedBatch {
            batch_id,
            batch: make_batch(),
            result: BatchResult::new(
                training_weights.clone(),
                json!({
                    "count": training_weights.len() as i64,
                    "sum_weight": observable_sum_weight,
                    "sum_abs": 0.0,
                    "sum_sq": 0.0,
                }),
            ),
            completed_at: None,
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
            "sum_weight": 1.0,
            "sum_abs": 0.0,
            "sum_sq": 0.0,
        }));

        let engine = TestEngine {
            produced: vec![(make_batch(), Some(7))],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            ObservableFactory::new(crate::engines::ObservableImplementation::Scalar, json!({})),
            queue.clone(),
            aggregation_store.clone(),
            RunnerConfig {
                nr_samples: 64,
                performance_snapshot_interval_ms: 0,
                max_batches_per_tick: 1,
                max_pending_batches: 8,
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
        assert_eq!(perf.run_id, 1);
        assert_eq!(perf.worker_id, "worker-a");
        assert_eq!(perf.produced_batches, 1);
        assert_eq!(perf.produced_samples, 1);
        assert_eq!(perf.ingested_batches, 2);
        assert_eq!(perf.ingested_samples, 3);
        assert!(perf.avg_produce_time_per_sample_ms >= 0.0);
        assert!(perf.avg_ingest_time_per_sample_ms >= 0.0);
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
        let sum_weight = agg_saved[0]
            .1
            .get("sum_weight")
            .and_then(|value| value.as_f64())
            .expect("sum_weight f64");
        assert!((sum_weight - 1.7).abs() < 1e-12);
    }

    #[tokio::test]
    async fn tick_without_completed_batches_skips_snapshot_persist_and_delete() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        let aggregation_store = TestAggregationStore::default();

        let engine = TestEngine {
            produced: vec![],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            ObservableFactory::new(crate::engines::ObservableImplementation::Scalar, json!({})),
            queue.clone(),
            aggregation_store.clone(),
            RunnerConfig {
                nr_samples: 64,
                performance_snapshot_interval_ms: 0,
                max_batches_per_tick: 1,
                max_pending_batches: 0,
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
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            ObservableFactory::new(crate::engines::ObservableImplementation::Scalar, json!({})),
            queue.clone(),
            aggregation_store,
            RunnerConfig {
                nr_samples: 64,
                performance_snapshot_interval_ms: 0,
                max_batches_per_tick: 2,
                max_pending_batches: 5,
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
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            "worker-a",
            Box::new(engine),
            ObservableFactory::new(crate::engines::ObservableImplementation::Scalar, json!({})),
            queue.clone(),
            aggregation_store,
            RunnerConfig {
                nr_samples: 64,
                performance_snapshot_interval_ms: 0,
                max_batches_per_tick: 3,
                max_pending_batches: 4,
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
        assert_eq!(q.sampler_perf_snapshots[0].produced_batches, 1);
        assert_eq!(q.sampler_perf_snapshots[0].ingested_batches, 0);
    }
}
