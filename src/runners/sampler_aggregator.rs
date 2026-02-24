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
use crate::core::{AggregationStore, CompletedBatch, StoreError, WorkQueueStore};
use crate::engines::{BatchContext, EngineError, Observable, SamplerAggregator};
use std::{collections::HashMap, error::Error, fmt};

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
    engine: Box<dyn SamplerAggregator>,
    aggregated_observable: Box<dyn Observable>,
    work_queue: WQ,
    aggregation_store: AS,
    config: RunnerConfig,
    point_spec: PointSpec,
    local_batch_contexts: HashMap<i64, BatchContext>,
}

impl<WQ, AS> SamplerAggregatorRunner<WQ, AS>
where
    WQ: WorkQueueStore,
    AS: AggregationStore,
{
    pub async fn new(
        run_id: i32,
        mut engine: Box<dyn SamplerAggregator>,
        mut aggregated_observable: Box<dyn Observable>,
        work_queue: WQ,
        aggregation_store: AS,
        config: RunnerConfig,
        point_spec: PointSpec,
    ) -> Result<Self, RunnerError> {
        let persisted_snapshot = aggregation_store
            .load_latest_aggregation_snapshot(run_id)
            .await?;

        engine.init().map_err(RunnerError::Engine)?;
        if let Some(snapshot) = persisted_snapshot {
            aggregated_observable
                .load_state_from_json(&snapshot)
                .map_err(RunnerError::Engine)?;
        }

        Ok(Self {
            run_id,
            engine,
            aggregated_observable,
            work_queue,
            aggregation_store,
            config,
            point_spec,
            local_batch_contexts: HashMap::new(),
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

        let mut produced = Vec::with_capacity(produce_limit);
        for _ in 0..produce_limit {
            produced.push(
                self.engine
                    // `0` means "engine default batch size" for current implementations.
                    .produce_batch(0)
                    .map_err(RunnerError::Engine)?,
            );
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
            self.engine
                .ingest_training_weights(&batch.result.values, context)
                .map_err(RunnerError::Engine)?;
            self.aggregated_observable
                .merge_state_from_json(&batch.result.observable)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{Batch, BatchResult, PointSpec};
    use crate::core::StoreError;
    use crate::engines::{
        BatchContext, BuildError, Observable, SamplerAggregator, decode_observable_state,
        encode_observable_state,
    };
    use crate::runners::test_support::MockWorkQueue;
    use serde::{Deserialize, Serialize};
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
        initialized: bool,
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

        fn init(&mut self) -> Result<(), EngineError> {
            self.probe.lock().expect("poison").initialized = true;
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

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    struct TestObservableState {
        nr_samples: i64,
        sum: f64,
    }

    impl TestObservableState {
        fn merge_from(&mut self, other: &Self) {
            self.nr_samples += other.nr_samples;
            self.sum += other.sum;
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestObservableSnapshot {
        nr_samples: i64,
        sum: f64,
    }

    impl From<&TestObservableState> for TestObservableSnapshot {
        fn from(state: &TestObservableState) -> Self {
            Self {
                nr_samples: state.nr_samples,
                sum: state.sum,
            }
        }
    }

    impl From<TestObservableSnapshot> for TestObservableState {
        fn from(snapshot: TestObservableSnapshot) -> Self {
            Self {
                nr_samples: snapshot.nr_samples,
                sum: snapshot.sum,
            }
        }
    }

    #[derive(Default)]
    struct TestObservable {
        state: TestObservableState,
    }

    impl Observable for TestObservable {
        fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
            let decoded: TestObservableSnapshot =
                decode_observable_state(state, "test observable snapshot")?;
            self.state = decoded.into();
            Ok(())
        }

        fn merge_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
            let decoded: TestObservableSnapshot =
                decode_observable_state(state, "test batch observable")?;
            let other: TestObservableState = decoded.into();
            self.state.merge_from(&other);
            Ok(())
        }

        fn snapshot(&self) -> Result<JsonValue, EngineError> {
            encode_observable_state(
                &TestObservableSnapshot::from(&self.state),
                "test observable snapshot",
            )
        }
    }

    fn make_batch() -> Batch {
        Batch::from_flat_data(1, 1, 0, vec![1.0], vec![]).expect("batch")
    }

    fn make_completed(
        batch_id: i64,
        training_weights: Vec<f64>,
        observable_sum: f64,
    ) -> CompletedBatch {
        CompletedBatch {
            batch_id,
            batch: make_batch(),
            result: BatchResult::new(
                training_weights.clone(),
                json!({
                    "nr_samples": training_weights.len() as i64,
                    "sum": observable_sum,
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
            "nr_samples": 3,
            "sum": 1.0,
        }));

        let engine = TestEngine {
            produced: vec![(make_batch(), Some(7))],
            probe: probe.clone(),
        };

        let mut runner = SamplerAggregatorRunner::new(
            1,
            Box::new(engine),
            Box::new(TestObservable::default()),
            queue.clone(),
            aggregation_store.clone(),
            RunnerConfig {
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

        assert!(p.initialized);
        assert_eq!(q.inserted.len(), 1);
        assert_eq!(p.ingested_training_sizes, vec![2, 1]);
        assert_eq!(p.ingested_context_tokens, vec![Some(7), None]);
        assert_eq!(q.deleted_completed_batch_ids, vec![11, 12]);
        assert_eq!(tick.processed_completed_batches, 2);

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
            Box::new(engine),
            Box::new(TestObservable::default()),
            queue.clone(),
            aggregation_store.clone(),
            RunnerConfig {
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
            Box::new(engine),
            Box::new(TestObservable::default()),
            queue.clone(),
            aggregation_store,
            RunnerConfig {
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
            Box::new(engine),
            Box::new(TestObservable::default()),
            queue.clone(),
            aggregation_store,
            RunnerConfig {
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

        assert_eq!(p.produce_requested_nr_samples, vec![0]);
        assert_eq!(tick.enqueued_batches, 1);
        assert_eq!(q.inserted.len(), 1);
    }
}
