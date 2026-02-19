//! Sampler-aggregator runner orchestration.
//!
//! This module intentionally focuses on process orchestration:
//! - load persisted engine state
//! - load persisted aggregated observable snapshot
//! - call engine hooks
//! - enqueue produced batches
//! - fetch completed batches and pass training weights back into the engine
//! - aggregate completed batch observables into run-level observable snapshot
//! - persist cursor state

use crate::contracts::{
    AggregatedObservable, AggregationStore, CompletedBatch, EngineError, EngineState,
    EngineStateStore, SamplerAggregatorEngine, StoreError, WorkQueueStore,
};
use serde_json::json;
use std::{error::Error, fmt};

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub max_batches_per_tick: usize,
    pub max_pending_batches: usize,
    pub completed_batch_fetch_limit: usize,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
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
    pub last_processed_batch_id: Option<i64>,
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

pub struct SamplerAggregatorRunner<E, WQ, ES, AS> {
    run_id: i32,
    engine: E,
    aggregated_observable: Box<dyn AggregatedObservable>,
    work_queue: WQ,
    state_store: ES,
    aggregation_store: AS,
    config: RunnerConfig,
    last_processed_batch_id: Option<i64>,
}

impl<E, WQ, ES, AS> SamplerAggregatorRunner<E, WQ, ES, AS>
where
    E: SamplerAggregatorEngine,
    WQ: WorkQueueStore,
    ES: EngineStateStore,
    AS: AggregationStore,
{
    pub async fn new(
        run_id: i32,
        mut engine: E,
        mut aggregated_observable: Box<dyn AggregatedObservable>,
        work_queue: WQ,
        state_store: ES,
        aggregation_store: AS,
        config: RunnerConfig,
    ) -> Result<Self, RunnerError> {
        let persisted_state = state_store.load_engine_state(run_id).await?;
        let last_processed_batch_id = persisted_state
            .as_ref()
            .and_then(|state| state.last_processed_batch_id);
        let persisted_snapshot = aggregation_store
            .load_latest_aggregation_snapshot(run_id)
            .await?;

        engine.init(persisted_state).map_err(RunnerError::Engine)?;
        aggregated_observable
            .restore(persisted_snapshot)
            .map_err(RunnerError::Engine)?;

        Ok(Self {
            run_id,
            engine,
            aggregated_observable,
            work_queue,
            state_store,
            aggregation_store,
            config,
            last_processed_batch_id,
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
        let produce_limit = self.config.max_batches_per_tick.min(remaining_capacity);

        let mut produced = if produce_limit == 0 {
            Vec::new()
        } else {
            self.engine
                .produce_batches(produce_limit)
                .map_err(RunnerError::Engine)?
        };
        if produced.len() > produce_limit {
            produced.truncate(produce_limit);
        }
        for batch in &produced {
            self.work_queue.insert_batch(self.run_id, batch).await?;
        }

        let completed = self
            .work_queue
            .fetch_completed_batches_since(
                self.run_id,
                self.last_processed_batch_id,
                self.config.completed_batch_fetch_limit,
            )
            .await?;

        self.process_completed(&completed).await?;

        Ok(RunnerTick {
            enqueued_batches: produced.len(),
            processed_completed_batches: completed.len(),
            last_processed_batch_id: self.last_processed_batch_id,
        })
    }

    pub fn last_processed_batch_id(&self) -> Option<i64> {
        self.last_processed_batch_id
    }

    async fn process_completed(&mut self, completed: &[CompletedBatch]) -> Result<(), RunnerError> {
        if completed.is_empty() {
            return Ok(());
        }

        for batch in completed {
            self.engine
                .ingest_training_weights(&batch.results.training_weights)
                .map_err(RunnerError::Engine)?;
            self.aggregated_observable
                .ingest_batch_observable(&batch.batch_observable)
                .map_err(RunnerError::Engine)?;
        }

        let snapshot = self
            .aggregated_observable
            .snapshot()
            .map_err(RunnerError::Engine)?;

        self.aggregation_store
            .save_aggregation_snapshot(self.run_id, &snapshot, completed.len() as i32)
            .await?;

        self.last_processed_batch_id = completed.last().map(|batch| batch.batch_id);

        let state = EngineState {
            last_processed_batch_id: self.last_processed_batch_id,
            state: json!({}),
        };
        self.state_store
            .save_engine_state(self.run_id, &state)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{Batch, BatchResults, WeightedPoint};
    use crate::contracts::{EngineState, StoreError};
    use crate::runners::test_support::MockWorkQueue;
    use serde_json::{Value as JsonValue, json};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct StateStoreData {
        initial: Option<EngineState>,
        saved: Vec<EngineState>,
    }

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
    struct TestStateStore {
        inner: Arc<Mutex<StateStoreData>>,
    }

    #[async_trait::async_trait]
    impl EngineStateStore for TestStateStore {
        async fn load_engine_state(&self, _run_id: i32) -> Result<Option<EngineState>, StoreError> {
            Ok(self.inner.lock().expect("poison").initial.clone())
        }

        async fn save_engine_state(
            &self,
            _run_id: i32,
            state: &EngineState,
        ) -> Result<(), StoreError> {
            self.inner.lock().expect("poison").saved.push(state.clone());
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct Probe {
        init_last_processed_batch_id: Option<i64>,
        ingested_training_sizes: Vec<usize>,
        produce_requested: Vec<usize>,
    }

    struct TestEngine {
        produced: Vec<Batch>,
        probe: Arc<Mutex<Probe>>,
    }

    impl SamplerAggregatorEngine for TestEngine {
        fn implementation(&self) -> &'static str {
            "test_engine"
        }

        fn version(&self) -> &'static str {
            "v1"
        }

        fn init(&mut self, state: Option<EngineState>) -> Result<(), EngineError> {
            self.probe
                .lock()
                .expect("poison")
                .init_last_processed_batch_id = state.and_then(|s| s.last_processed_batch_id);
            Ok(())
        }

        fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError> {
            self.probe
                .lock()
                .expect("poison")
                .produce_requested
                .push(max_batches);
            Ok(self.produced.iter().take(max_batches).cloned().collect())
        }

        fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
            self.probe
                .lock()
                .expect("poison")
                .ingested_training_sizes
                .push(training_weights.len());
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestObservable {
        nr_samples: i64,
        sum: f64,
    }

    impl AggregatedObservable for TestObservable {
        fn implementation(&self) -> &'static str {
            "test_observable"
        }

        fn version(&self) -> &'static str {
            "v1"
        }

        fn restore(&mut self, snapshot: Option<JsonValue>) -> Result<(), EngineError> {
            if let Some(snapshot) = snapshot {
                self.nr_samples = snapshot
                    .get("nr_samples")
                    .and_then(|v| v.as_i64())
                    .ok_or_else(|| EngineError::engine("missing nr_samples"))?;
                self.sum = snapshot
                    .get("sum")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| EngineError::engine("missing sum"))?;
            }
            Ok(())
        }

        fn ingest_sample_observable(
            &mut self,
            _sample_observable: &JsonValue,
        ) -> Result<(), EngineError> {
            Ok(())
        }

        fn ingest_batch_observable(
            &mut self,
            batch_observable: &JsonValue,
        ) -> Result<(), EngineError> {
            let nr_samples = batch_observable
                .get("nr_samples")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| EngineError::engine("missing nr_samples"))?;
            let sum = batch_observable
                .get("sum")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| EngineError::engine("missing sum"))?;
            self.nr_samples += nr_samples;
            self.sum += sum;
            Ok(())
        }

        fn snapshot(&self) -> Result<JsonValue, EngineError> {
            Ok(json!({
                "nr_samples": self.nr_samples,
                "sum": self.sum,
            }))
        }
    }

    fn make_batch() -> Batch {
        Batch::new(vec![WeightedPoint::new(json!(1.0), 1.0)])
    }

    fn make_completed(
        batch_id: i64,
        training_weights: Vec<f64>,
        observable_sum: f64,
    ) -> CompletedBatch {
        CompletedBatch {
            batch_id,
            batch: make_batch(),
            results: BatchResults::new(training_weights.clone()),
            batch_observable: json!({
                "nr_samples": training_weights.len() as i64,
                "sum": observable_sum,
            }),
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn tick_processes_completed_batches_and_persists_cursor_and_snapshot() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        {
            let mut q = queue.inner.lock().expect("poison");
            q.completed = vec![
                make_completed(11, vec![0.1, 0.2], 0.3),
                make_completed(12, vec![0.4], 0.4),
            ];
        }

        let state_store = TestStateStore::default();
        let aggregation_store = TestAggregationStore::default();
        state_store.inner.lock().expect("poison").initial = Some(EngineState {
            last_processed_batch_id: Some(10),
            state: serde_json::json!({}),
        });
        aggregation_store
            .inner
            .lock()
            .expect("poison")
            .initial_snapshot = Some(json!({
            "nr_samples": 3,
            "sum": 1.0,
        }));

        let engine = TestEngine {
            produced: vec![make_batch()],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            engine,
            Box::new(TestObservable::default()),
            queue.clone(),
            state_store.clone(),
            aggregation_store.clone(),
            RunnerConfig {
                max_batches_per_tick: 1,
                max_pending_batches: 8,
                completed_batch_fetch_limit: 128,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let q = queue.inner.lock().expect("poison").clone();
        let s = state_store.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();
        let agg_saved = aggregation_store
            .inner
            .lock()
            .expect("poison")
            .saved
            .clone();

        assert_eq!(p.init_last_processed_batch_id, Some(10));
        assert_eq!(q.fetch_last_batch_ids, vec![Some(10)]);
        assert_eq!(q.inserted.len(), 1);
        assert_eq!(p.ingested_training_sizes, vec![2, 1]);
        assert_eq!(tick.last_processed_batch_id, Some(12));

        assert_eq!(agg_saved.len(), 1);
        assert_eq!(agg_saved[0].0, 1);
        assert_eq!(agg_saved[0].2, 2);
        assert_eq!(
            agg_saved[0]
                .1
                .get("nr_samples")
                .and_then(|value| value.as_i64()),
            Some(6)
        );
        let sum = agg_saved[0]
            .1
            .get("sum")
            .and_then(|value| value.as_f64())
            .expect("sum f64");
        assert!((sum - 1.7).abs() < 1e-12);

        assert_eq!(s.saved.len(), 1);
        assert_eq!(s.saved[0].last_processed_batch_id, Some(12));
    }

    #[tokio::test]
    async fn tick_without_completed_batches_skips_state_and_snapshot_persist() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        let state_store = TestStateStore::default();
        let aggregation_store = TestAggregationStore::default();

        let engine = TestEngine {
            produced: vec![],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            engine,
            Box::new(TestObservable::default()),
            queue.clone(),
            state_store.clone(),
            aggregation_store.clone(),
            RunnerConfig {
                max_batches_per_tick: 1,
                max_pending_batches: 8,
                completed_batch_fetch_limit: 64,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let s = state_store.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();
        let agg_saved = aggregation_store
            .inner
            .lock()
            .expect("poison")
            .saved
            .clone();

        assert_eq!(tick.processed_completed_batches, 0);
        assert!(p.ingested_training_sizes.is_empty());
        assert!(agg_saved.is_empty());
        assert!(s.saved.is_empty());
    }

    #[tokio::test]
    async fn tick_skips_batch_production_when_pending_queue_is_full() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").pending_batches = 5;

        let state_store = TestStateStore::default();
        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![make_batch(), make_batch()],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            engine,
            Box::new(TestObservable::default()),
            queue.clone(),
            state_store,
            aggregation_store,
            RunnerConfig {
                max_batches_per_tick: 2,
                max_pending_batches: 5,
                completed_batch_fetch_limit: 64,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let q = queue.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();

        assert_eq!(tick.enqueued_batches, 0);
        assert!(q.inserted.is_empty());
        assert!(p.produce_requested.is_empty());
    }

    #[tokio::test]
    async fn tick_limits_batch_production_to_remaining_pending_capacity() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = MockWorkQueue::default();
        queue.inner.lock().expect("poison").pending_batches = 3;

        let state_store = TestStateStore::default();
        let aggregation_store = TestAggregationStore::default();
        let engine = TestEngine {
            produced: vec![make_batch(), make_batch(), make_batch()],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            engine,
            Box::new(TestObservable::default()),
            queue.clone(),
            state_store,
            aggregation_store,
            RunnerConfig {
                max_batches_per_tick: 3,
                max_pending_batches: 4,
                completed_batch_fetch_limit: 64,
            },
        )
        .await
        .expect("new runner");

        let tick = runner.tick().await.expect("tick");
        let q = queue.inner.lock().expect("poison").clone();
        let p = probe.lock().expect("poison").clone();

        assert_eq!(p.produce_requested, vec![1]);
        assert_eq!(tick.enqueued_batches, 1);
        assert_eq!(q.inserted.len(), 1);
    }
}
