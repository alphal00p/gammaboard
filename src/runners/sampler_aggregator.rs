//! Sampler-aggregator runner orchestration.
//!
//! This module intentionally focuses on process orchestration:
//! - load persisted engine state
//! - call engine hooks
//! - enqueue produced batches
//! - fetch completed batches and pass them back into the engine
//! - persist cursor state

use crate::contracts::{
    AggregationStore, CompletedBatch, EngineError, EngineState, EngineStateStore,
    SamplerAggregatorEngine, StoreError, WorkQueueStore,
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

impl From<EngineError> for RunnerError {
    fn from(value: EngineError) -> Self {
        RunnerError::Engine(value)
    }
}

impl From<StoreError> for RunnerError {
    fn from(value: StoreError) -> Self {
        RunnerError::Store(value)
    }
}

pub struct SamplerAggregatorRunner<E, WQ, ES, AS> {
    run_id: i32,
    engine: E,
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
        work_queue: WQ,
        state_store: ES,
        aggregation_store: AS,
        config: RunnerConfig,
    ) -> Result<Self, RunnerError> {
        let persisted_state = state_store.load_engine_state(run_id).await?;
        let last_processed_batch_id = persisted_state
            .as_ref()
            .and_then(|state| state.last_processed_batch_id);

        engine.init(persisted_state)?;

        Ok(Self {
            run_id,
            engine,
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
            self.engine.produce_batches(produce_limit)?
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

        self.engine.ingest_completed(completed)?;
        self.aggregation_store
            .aggregate_and_persist(self.run_id, completed)
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
    use crate::{
        Batch, BatchClaim, BatchResults, CompletedBatch, EngineState, StoreError, WeightedPoint,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct QueueState {
        inserted: Vec<Batch>,
        pending_batches: i64,
        completed: Vec<CompletedBatch>,
        fetch_last_batch_ids: Vec<Option<i64>>,
    }

    #[derive(Clone, Default)]
    struct TestQueue {
        inner: Arc<Mutex<QueueState>>,
    }

    impl WorkQueueStore for TestQueue {
        async fn insert_batch(&self, _run_id: i32, batch: &Batch) -> Result<(), StoreError> {
            let mut guard = self.inner.lock().expect("poison");
            guard.inserted.push(batch.clone());
            guard.pending_batches += 1;
            Ok(())
        }

        async fn get_pending_batch_count(&self, _run_id: i32) -> Result<i64, StoreError> {
            Ok(self.inner.lock().expect("poison").pending_batches)
        }

        async fn claim_batch(
            &self,
            _run_id: i32,
            _instance_id: &str,
        ) -> Result<Option<BatchClaim>, StoreError> {
            Ok(None)
        }

        async fn submit_batch_results(
            &self,
            _batch_id: i64,
            _results: &BatchResults,
            _eval_time_ms: f64,
        ) -> Result<(), StoreError> {
            Ok(())
        }

        async fn fail_batch(&self, _batch_id: i64, _last_error: &str) -> Result<(), StoreError> {
            Ok(())
        }

        async fn fetch_completed_batches_since(
            &self,
            _run_id: i32,
            last_batch_id: Option<i64>,
            _limit: usize,
        ) -> Result<Vec<CompletedBatch>, StoreError> {
            let mut guard = self.inner.lock().expect("poison");
            guard.fetch_last_batch_ids.push(last_batch_id);
            Ok(guard.completed.clone())
        }
    }

    #[derive(Clone, Default)]
    struct StateStoreData {
        initial: Option<EngineState>,
        saved: Vec<EngineState>,
    }

    #[derive(Clone, Default)]
    struct TestAggregationStore {
        calls: Arc<Mutex<Vec<Vec<i64>>>>,
    }

    impl AggregationStore for TestAggregationStore {
        async fn aggregate_and_persist(
            &self,
            _run_id: i32,
            completed: &[CompletedBatch],
        ) -> Result<(), StoreError> {
            let ids = completed
                .iter()
                .map(|batch| batch.batch_id)
                .collect::<Vec<_>>();
            self.calls.lock().expect("poison").push(ids);
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct TestStateStore {
        inner: Arc<Mutex<StateStoreData>>,
    }

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
        ingested_batch_ids: Vec<i64>,
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

        fn ingest_completed(&mut self, completed: &[CompletedBatch]) -> Result<(), EngineError> {
            let mut guard = self.probe.lock().expect("poison");
            for batch in completed {
                guard.ingested_batch_ids.push(batch.batch_id);
            }
            Ok(())
        }
    }

    fn make_batch() -> Batch {
        Batch::new(vec![WeightedPoint::new(serde_json::json!(1.0), 1.0)])
    }

    fn make_completed(batch_id: i64) -> CompletedBatch {
        CompletedBatch {
            batch_id,
            batch: make_batch(),
            results: BatchResults::new(vec![0.5]),
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn tick_processes_completed_batches_and_persists_cursor() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = TestQueue::default();
        {
            let mut q = queue.inner.lock().expect("poison");
            q.completed = vec![make_completed(11), make_completed(12)];
        }

        let state_store = TestStateStore::default();
        let aggregation_store = TestAggregationStore::default();
        state_store.inner.lock().expect("poison").initial = Some(EngineState {
            last_processed_batch_id: Some(10),
            state: serde_json::json!({}),
        });

        let engine = TestEngine {
            produced: vec![make_batch()],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            engine,
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
        let agg_calls = aggregation_store.calls.lock().expect("poison").clone();

        assert_eq!(p.init_last_processed_batch_id, Some(10));
        assert_eq!(q.fetch_last_batch_ids, vec![Some(10)]);
        assert_eq!(q.inserted.len(), 1);
        assert_eq!(p.ingested_batch_ids, vec![11, 12]);
        assert_eq!(agg_calls, vec![vec![11, 12]]);
        assert_eq!(tick.last_processed_batch_id, Some(12));
        assert_eq!(s.saved.len(), 1);
        assert_eq!(s.saved[0].last_processed_batch_id, Some(12));
    }

    #[tokio::test]
    async fn tick_without_completed_batches_skips_state_persist() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = TestQueue::default();
        let state_store = TestStateStore::default();
        let aggregation_store = TestAggregationStore::default();

        let engine = TestEngine {
            produced: vec![],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            engine,
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
        let agg_calls = aggregation_store.calls.lock().expect("poison").clone();

        assert_eq!(tick.processed_completed_batches, 0);
        assert!(p.ingested_batch_ids.is_empty());
        assert!(agg_calls.is_empty());
        assert!(s.saved.is_empty());
    }

    #[tokio::test]
    async fn tick_skips_batch_production_when_pending_queue_is_full() {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let queue = TestQueue::default();
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
        let queue = TestQueue::default();
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
