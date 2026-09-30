use crate::core::SamplerQueueCheckpoint;
use crate::core::{
    BatchQueueCounts, CompletedBatch, InsertBatchesMetrics, SamplerQueueRollingAverages,
    SamplerQueueRuntimeMetrics, SamplerQueueTuning, SamplerWorkerStore, StoreError, next_batch_ids,
};
use crate::runners::busy_time::BusyTime;
use crate::runners::rolling_metric::RollingMetric;
use crate::runners::window_metric::WindowMetric;
use crate::sampling::LatentBatch;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

const RECLAIM_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETED_CLEANUP_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETED_CLEANUP_BATCH_LIMIT: usize = 2048;
pub(crate) const MIN_BATCH_SIZE: usize = 16;
const DEFAULT_BATCH_SIZE_DEADBAND_RATIO: f64 = 0.15;
const DEFAULT_BATCH_SIZE_COOLDOWN_TICKS: u32 = 3;
const DEFAULT_MAX_BATCH_RETRIES: i32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerQueueConfig {
    pub target_batch_eval_ms: f64,
    pub max_batch_size: usize,
    /// Disable adaptation; training boundaries and remaining budgets still cap batches.
    #[serde(default)]
    pub fixed_batch_size: Option<usize>,
    pub max_batches_per_tick: usize,
    pub max_insert_bundle_size: usize,
    pub max_concurrent_insert_tasks: usize,
    pub completed_batch_fetch_limit: usize,
    #[serde(default = "default_max_batch_retries")]
    pub max_batch_retries: i32,
}

impl SamplerQueueConfig {
    pub fn apply_tuning(&mut self, tuning: &SamplerQueueTuning) {
        apply_option(
            &mut self.fixed_batch_size,
            tuning.fixed_batch_size.map(Some),
        );
        apply_option(&mut self.target_batch_eval_ms, tuning.target_batch_eval_ms);
        apply_option(&mut self.max_batch_size, tuning.max_batch_size);
    }
}

fn apply_option<T>(destination: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *destination = value;
    }
}

pub struct SamplerQueue<S> {
    run_id: i32,
    task_id: i64,
    store: S,
    config: SamplerQueueConfig,
    checkpoint: SamplerQueueCheckpoint,
    pending_insert: VecDeque<(LatentBatch, bool)>,
    ready_processed: VecDeque<CompletedBatch>,
    pending_insert_tasks: Vec<PendingInsertTask>,
    pending_processed_fetch: Option<PendingProcessedFetchTask>,
    pending_completed_cleanup: Option<PendingCompletedCleanupTask>,
    cached_db_queue_counts: Option<BatchQueueCounts>,
    cached_tick_queue_counts: Option<BatchQueueCounts>,
    cached_active_evaluator_count: Option<usize>,
    blocker: Option<crate::core::QueueBlocker>,
    last_reclaim_at: Instant,
    last_completed_cleanup_at: Instant,
    batch_size_tune_cooldown_remaining: u32,
    eval_ms_per_sample: RollingMetric,
    training_batch_sizing: TrainingBatchSizing,
    metrics: QueueMetricsState,
    pub(crate) busy: BusyTime,
}

/// Keep a finite training window divisible across workers, without recursively
/// shrinking chunks as its remaining sample count decreases.
#[derive(Default)]
struct TrainingBatchSizing {
    previous_remaining: Option<usize>,
    evaluators: usize,
    cap: Option<usize>,
}

impl TrainingBatchSizing {
    fn batch_size(&mut self, target: usize, remaining: Option<usize>, evaluators: usize) -> usize {
        match remaining {
            Some(remaining) if remaining > 0 && evaluators > 0 => {
                if self
                    .previous_remaining
                    .is_none_or(|previous| remaining > previous)
                    || self.evaluators != evaluators
                {
                    self.cap = Some(
                        remaining
                            .div_ceil(evaluators.saturating_mul(4))
                            .max(MIN_BATCH_SIZE),
                    );
                }
                self.previous_remaining = Some(remaining);
                self.evaluators = evaluators;
                target.min(self.cap.unwrap_or(target))
            }
            _ => {
                self.previous_remaining = None;
                self.cap = None;
                target
            }
        }
    }
}

const fn default_max_batch_retries() -> i32 {
    DEFAULT_MAX_BATCH_RETRIES
}

struct PendingInsertTask {
    first_batch_id: i64,
    batch_count: usize,
    local_pending_at_start: usize,
    db_pending_at_start: Option<i64>,
    handle: JoinHandle<Result<InsertBatchesMetrics, StoreError>>,
}

type PendingProcessedFetchTask = JoinHandle<Result<(Vec<CompletedBatch>, Duration), StoreError>>;

type PendingCompletedCleanupTask = JoinHandle<Result<Duration, StoreError>>;

pub struct QueueTickResult {
    pub completed: Vec<CompletedBatch>,
    pub queue_counts: BatchQueueCounts,
    pub queue_snapshot_duration: Duration,
    pub reclaim_duration: Option<Duration>,
    pub completed_cleanup_duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub struct QueueDiagnosticsSnapshot {
    pub queue_counts: BatchQueueCounts,
    pub active_evaluator_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct QueueMetricsState {
    fetch_completed_ms: WindowMetric,
    fetch_completed_batches: WindowMetric,
    fetch_completed_prefetch_fill_ratio: WindowMetric,
    insert_bundle_ms: WindowMetric,
    insert_bundle_batches: WindowMetric,
    insert_bundle_ms_per_batch: WindowMetric,
    insert_bundle_serialize_ms: WindowMetric,
    insert_bundle_payload_bytes: WindowMetric,
    insert_bundle_payload_bytes_per_batch: WindowMetric,
    insert_bundle_db_batches_ms: WindowMetric,
    insert_bundle_db_inputs_ms: WindowMetric,
    insert_bundle_commit_ms: WindowMetric,
    insert_bundle_local_pending_at_start: WindowMetric,
    insert_bundle_db_pending_at_start: WindowMetric,
}

impl<S> SamplerQueue<S>
where
    S: SamplerWorkerStore + Clone + Send + Sync + 'static,
{
    pub fn new(
        store: S,
        run_id: i32,
        task_id: i64,
        config: SamplerQueueConfig,
        mut checkpoint: SamplerQueueCheckpoint,
    ) -> Self {
        let now = Instant::now();
        let max_batch_size = config.max_batch_size.max(MIN_BATCH_SIZE);
        checkpoint.batch_size_current = config
            .fixed_batch_size
            .unwrap_or(checkpoint.batch_size_current)
            .clamp(MIN_BATCH_SIZE, max_batch_size);
        Self {
            run_id,
            task_id,
            store,
            config,
            checkpoint,
            pending_insert: VecDeque::new(),
            ready_processed: VecDeque::new(),
            pending_insert_tasks: Vec::new(),
            pending_processed_fetch: None,
            pending_completed_cleanup: None,
            cached_db_queue_counts: None,
            cached_tick_queue_counts: None,
            cached_active_evaluator_count: None,
            blocker: None,
            last_reclaim_at: now.checked_sub(RECLAIM_INTERVAL).unwrap_or(now),
            last_completed_cleanup_at: now.checked_sub(COMPLETED_CLEANUP_INTERVAL).unwrap_or(now),
            batch_size_tune_cooldown_remaining: 0,
            eval_ms_per_sample: RollingMetric::default(),
            training_batch_sizing: TrainingBatchSizing::default(),
            metrics: QueueMetricsState::default(),
            busy: BusyTime::default(),
        }
    }

    pub fn config(&self) -> &SamplerQueueConfig {
        &self.config
    }

    pub fn apply_config(&mut self, config: SamplerQueueConfig) {
        self.config = config;
        self.checkpoint.batch_size_current = self
            .config
            .fixed_batch_size
            .unwrap_or(self.checkpoint.batch_size_current)
            .clamp(MIN_BATCH_SIZE, self.effective_max_batch_size());
        self.batch_size_tune_cooldown_remaining = 0;
    }

    pub fn checkpoint(&self) -> SamplerQueueCheckpoint {
        self.checkpoint.clone()
    }

    pub fn current_batch_size(&self) -> usize {
        self.checkpoint.batch_size_current
    }

    pub fn observe_completed_eval_batch(&mut self, batch_size: usize, total_eval_time_ms: f64) {
        if batch_size == 0 || !total_eval_time_ms.is_finite() || total_eval_time_ms <= 0.0 {
            return;
        }
        self.eval_ms_per_sample
            .observe_batch(total_eval_time_ms, batch_size);
        self.tune_batch_size();
    }

    pub fn runtime_metrics(&self) -> SamplerQueueRuntimeMetrics {
        SamplerQueueRuntimeMetrics {
            blocker: self.blocker.clone(),
            db_pending_batches: self.cached_db_queue_counts.map(|counts| counts.pending),
            db_claimed_batches: self.cached_db_queue_counts.map(|counts| counts.claimed),
            db_completed_batches: self.cached_db_queue_counts.map(|counts| counts.completed),
            local_pending_batches: self.pending_insert.len(),
            local_inflight_insert_tasks: self.pending_insert_tasks.len(),
            local_inflight_insert_batches: self
                .pending_insert_tasks
                .iter()
                .map(|task| task.batch_count)
                .sum(),
            local_ready_processed_batches: self.ready_processed.len(),
            rolling: SamplerQueueRollingAverages::default(),
        }
    }

    pub fn take_metrics_snapshot(&mut self) -> SamplerQueueRollingAverages {
        SamplerQueueRollingAverages {
            fetch_completed_ms: self.metrics.fetch_completed_ms.snapshot_and_reset(),
            fetch_completed_batches: self.metrics.fetch_completed_batches.snapshot_and_reset(),
            fetch_completed_prefetch_fill_ratio: self
                .metrics
                .fetch_completed_prefetch_fill_ratio
                .snapshot_and_reset(),
            insert_bundle_ms: self.metrics.insert_bundle_ms.snapshot_and_reset(),
            insert_bundle_batches: self.metrics.insert_bundle_batches.snapshot_and_reset(),
            insert_bundle_ms_per_batch: self
                .metrics
                .insert_bundle_ms_per_batch
                .snapshot_and_reset(),
            insert_bundle_serialize_ms: self
                .metrics
                .insert_bundle_serialize_ms
                .snapshot_and_reset(),
            insert_bundle_payload_bytes: self
                .metrics
                .insert_bundle_payload_bytes
                .snapshot_and_reset(),
            insert_bundle_payload_bytes_per_batch: self
                .metrics
                .insert_bundle_payload_bytes_per_batch
                .snapshot_and_reset(),
            insert_bundle_db_batches_ms: self
                .metrics
                .insert_bundle_db_batches_ms
                .snapshot_and_reset(),
            insert_bundle_db_inputs_ms: self
                .metrics
                .insert_bundle_db_inputs_ms
                .snapshot_and_reset(),
            insert_bundle_commit_ms: self.metrics.insert_bundle_commit_ms.snapshot_and_reset(),
            insert_bundle_local_pending_at_start: self
                .metrics
                .insert_bundle_local_pending_at_start
                .snapshot_and_reset(),
            insert_bundle_db_pending_at_start: self
                .metrics
                .insert_bundle_db_pending_at_start
                .snapshot_and_reset(),
        }
    }

    pub fn last_completed_batch_id(&self) -> Option<i64> {
        self.checkpoint.last_completed_batch_id
    }

    pub async fn queue_counts(&mut self) -> Result<BatchQueueCounts, StoreError> {
        let counts = self.db_queue_counts().await?;
        Ok(self.queue_counts_with_local_buffer(counts))
    }

    async fn db_queue_counts(&mut self) -> Result<BatchQueueCounts, StoreError> {
        let _io = self.busy.io();
        let counts = self
            .store
            .get_batch_queue_counts(
                self.run_id,
                Some(self.task_id),
                self.last_completed_batch_id(),
            )
            .await?;
        self.cached_db_queue_counts = Some(counts);
        Ok(counts)
    }

    async fn reclaim_abandoned_batches(&self) -> Result<u64, StoreError> {
        let _io = self.busy.io();
        self.store.reclaim_abandoned_batches(self.run_id).await
    }

    async fn cleanup_consumed_completed_batches(&self) -> Result<u64, StoreError> {
        let _io = self.busy.io();
        let Some(up_to_batch_id) = self.last_completed_batch_id() else {
            return Ok(0);
        };
        self.store
            .cleanup_consumed_completed_batches(
                self.run_id,
                up_to_batch_id,
                COMPLETED_CLEANUP_BATCH_LIMIT,
            )
            .await
    }

    pub async fn tick(&mut self) -> Result<QueueTickResult, StoreError> {
        let completed = self.get_processed().await?;
        let completed_cleanup_duration = self.drain_finished_completed_cleanup().await?;
        self.ensure_completed_cleanup_if_due();
        let reclaim_duration = self.reclaim_abandoned_batches_if_due().await?;
        let queue_snapshot_started = Instant::now();
        let queue_counts = self.queue_counts().await?;
        let queue_snapshot_duration = queue_snapshot_started.elapsed();
        self.cached_tick_queue_counts = Some(queue_counts);
        Ok(QueueTickResult {
            completed,
            queue_counts,
            queue_snapshot_duration,
            reclaim_duration,
            completed_cleanup_duration,
        })
    }

    pub async fn cleanup_completed_batches(&mut self) -> Result<Option<Duration>, StoreError> {
        if let Some(task) = self.pending_completed_cleanup.take() {
            let _ = self.consume_completed_cleanup_task(task).await?;
        }
        let Some(_) = self.last_completed_batch_id() else {
            return Ok(None);
        };
        let cleanup_started = Instant::now();
        self.cleanup_consumed_completed_batches().await?;
        self.last_completed_cleanup_at = Instant::now();
        Ok(Some(cleanup_started.elapsed()))
    }

    pub(crate) fn production_batch_size(&mut self, training_remaining: Option<usize>) -> usize {
        self.training_batch_sizing.batch_size(
            self.checkpoint.batch_size_current,
            training_remaining,
            self.cached_active_evaluator_count.unwrap_or(0),
        )
    }

    pub(crate) async fn needs_generation(
        &mut self,
        counts: BatchQueueCounts,
    ) -> Result<bool, StoreError> {
        let _io = self.busy.io();
        let evaluators = self
            .store
            .count_active_evaluator_nodes(self.run_id)
            .await?
            .max(0) as usize;
        self.cached_active_evaluator_count = Some(evaluators);
        self.cached_tick_queue_counts = Some(counts);
        Ok(evaluators > 0 && (counts.pending.max(0) as usize) < evaluators)
    }

    pub fn diagnostics_snapshot(&self) -> Option<QueueDiagnosticsSnapshot> {
        self.cached_tick_queue_counts
            .map(|queue_counts| QueueDiagnosticsSnapshot {
                queue_counts,
                active_evaluator_count: self.cached_active_evaluator_count,
            })
    }

    pub fn target_pending_batches(&self, active_evaluator_count: usize) -> Option<usize> {
        Some(active_evaluator_count)
    }

    pub fn ingest(&mut self, batches: Vec<LatentBatch>, requires_feedback: bool) {
        if let Some(remaining) = &mut self.training_batch_sizing.previous_remaining {
            *remaining =
                remaining.saturating_sub(batches.iter().map(|batch| batch.nr_samples).sum());
        }
        self.pending_insert
            .extend(batches.into_iter().map(|batch| (batch, requires_feedback)));
        self.ensure_insert_pump();
    }

    fn local_unpersisted_batches(&self) -> usize {
        self.pending_insert.len()
            + self
                .pending_insert_tasks
                .iter()
                .map(|task| task.batch_count)
                .sum::<usize>()
    }

    fn snapshot_insert_bundle_start_state(&self) -> (usize, Option<i64>) {
        (
            self.pending_insert.len(),
            self.cached_db_queue_counts
                .map(|db_counts| db_counts.pending.max(0)),
        )
    }

    fn observe_insert_bundle_start_state(
        &mut self,
        local_pending_at_start: usize,
        db_pending_at_start: Option<i64>,
    ) {
        self.metrics
            .insert_bundle_local_pending_at_start
            .observe(local_pending_at_start as f64);
        if let Some(db_pending) = db_pending_at_start {
            self.metrics
                .insert_bundle_db_pending_at_start
                .observe(db_pending as f64);
        }
    }

    pub async fn get_processed(&mut self) -> Result<Vec<CompletedBatch>, StoreError> {
        self.drain_finished_insert().await?;
        self.ensure_insert_pump();
        self.drain_finished_processed_fetch().await?;
        self.ensure_processed_prefetch();

        Ok(self.take_ready_processed())
    }

    pub(crate) async fn get_processed_ready(&mut self) -> Result<Vec<CompletedBatch>, StoreError> {
        self.drain_finished_insert().await?;
        self.ensure_insert_pump();
        self.drain_finished_processed_fetch().await?;
        Ok(self.take_ready_processed())
    }

    pub(crate) fn cancel_nonessential_background_work(&mut self) {
        if let Some(task) = self.pending_processed_fetch.take() {
            task.abort();
        }
        if let Some(task) = self.pending_completed_cleanup.take() {
            task.abort();
        }
    }

    pub async fn flush(&mut self) -> Result<(), StoreError> {
        loop {
            self.drain_finished_insert().await?;
            if self.pending_insert_tasks.is_empty() && self.pending_insert.is_empty() {
                break;
            }
            self.ensure_insert_pump();
            if self.pending_insert_tasks.is_empty() {
                break;
            }
            let task = self.pending_insert_tasks.swap_remove(0);
            self.consume_insert_task(task).await?;
        }
        Ok(())
    }

    pub(crate) fn mark_processed(&mut self, processed: &[CompletedBatch]) {
        if let Some(last) = processed.last() {
            self.checkpoint.last_completed_batch_id = Some(last.batch_id);
        }
        self.ensure_completed_cleanup_if_due();
    }

    pub(crate) fn queue_counts_with_local_buffer(
        &self,
        queue_counts: BatchQueueCounts,
    ) -> BatchQueueCounts {
        BatchQueueCounts {
            pending: queue_counts
                .pending
                .saturating_add(self.local_unpersisted_batches() as i64),
            claimed: queue_counts.claimed,
            completed: queue_counts.completed,
            failed: queue_counts.failed,
        }
    }

    async fn reclaim_abandoned_batches_if_due(&mut self) -> Result<Option<Duration>, StoreError> {
        if self.last_reclaim_at.elapsed() < RECLAIM_INTERVAL {
            return Ok(None);
        }
        let reclaim_started = Instant::now();
        self.reclaim_abandoned_batches().await?;
        let _io = self.busy.io();
        self.blocker = self
            .store
            .get_queue_blocker(self.run_id, self.task_id, self.last_completed_batch_id())
            .await?;
        self.last_reclaim_at = Instant::now();
        Ok(Some(reclaim_started.elapsed()))
    }

    fn ensure_completed_cleanup_if_due(&mut self) {
        if self.pending_completed_cleanup.is_some() {
            return;
        }
        let Some(up_to_batch_id) = self.last_completed_batch_id() else {
            return;
        };
        if self.last_completed_cleanup_at.elapsed() < COMPLETED_CLEANUP_INTERVAL {
            return;
        }
        let busy = self.busy.clone();
        let store = self.store.clone();
        let run_id = self.run_id;
        self.pending_completed_cleanup = Some(tokio::spawn(async move {
            let _io = busy.io();
            let started = Instant::now();
            store
                .cleanup_consumed_completed_batches(
                    run_id,
                    up_to_batch_id,
                    COMPLETED_CLEANUP_BATCH_LIMIT,
                )
                .await?;
            Ok(started.elapsed())
        }));
    }

    async fn drain_finished_completed_cleanup(&mut self) -> Result<Option<Duration>, StoreError> {
        let Some(task) = self.pending_completed_cleanup.as_ref() else {
            return Ok(None);
        };
        if !task.is_finished() {
            return Ok(None);
        }
        let task = self
            .pending_completed_cleanup
            .take()
            .expect("checked pending completed cleanup");
        self.consume_completed_cleanup_task(task).await.map(Some)
    }

    async fn consume_completed_cleanup_task(
        &mut self,
        task: PendingCompletedCleanupTask,
    ) -> Result<Duration, StoreError> {
        match task.await {
            Ok(Ok(duration)) => {
                self.last_completed_cleanup_at = Instant::now();
                Ok(duration)
            }
            Ok(Err(err)) => Err(err),
            Err(err) => Err(StoreError::store(format!(
                "sampler queue completed cleanup task failed: {err}"
            ))),
        }
    }

    fn ensure_processed_prefetch(&mut self) {
        if !self.ready_processed.is_empty() || self.pending_processed_fetch.is_some() {
            return;
        }

        let busy = self.busy.clone();
        let store = self.store.clone();
        let run_id = self.run_id;
        let task_id = self.task_id;
        let fetch_limit = self.config.completed_batch_fetch_limit.max(1);
        let after_batch_id = self.checkpoint.last_completed_batch_id;
        // PostgreSQL cannot see an earlier bundle whose insert has not committed.
        // Keep the completion cursor below every outstanding insert. Reserving a
        // boundary also excludes inserts started after this async fetch is spawned.
        let before_batch_id = self
            .pending_insert_tasks
            .iter()
            .map(|task| task.first_batch_id)
            .min()
            .unwrap_or_else(|| next_batch_ids(1)[0]);
        self.pending_processed_fetch = Some(tokio::spawn(async move {
            let _io = busy.io();
            let started = Instant::now();
            let batches = store
                .fetch_completed_batches(run_id, task_id, fetch_limit, true, after_batch_id)
                .await?;
            let batches = batches
                .into_iter()
                .take_while(|batch| batch.batch_id < before_batch_id)
                .collect();
            Ok((batches, started.elapsed()))
        }));
    }

    fn ensure_insert_pump(&mut self) {
        let max_concurrent_insert_tasks = self.config.max_concurrent_insert_tasks.max(1);
        while self.pending_insert_tasks.len() < max_concurrent_insert_tasks
            && !self.pending_insert.is_empty()
        {
            let (local_pending_at_start, db_pending_at_start) =
                self.snapshot_insert_bundle_start_state();

            let bundle_size = self.config.max_insert_bundle_size.max(1);
            let requires_training_values = self.pending_insert.front().unwrap().1;
            let batch_count = self
                .pending_insert
                .iter()
                .take(bundle_size)
                .take_while(|(_, feedback)| *feedback == requires_training_values)
                .count();
            let batches = self
                .pending_insert
                .drain(..batch_count)
                .map(|(batch, _)| batch)
                .collect::<Vec<_>>();
            let batch_ids = next_batch_ids(batch_count);
            self.checkpoint.last_produced_batch_id = batch_ids.last().copied();
            let busy = self.busy.clone();
            let store = self.store.clone();
            let run_id = self.run_id;
            let task_id = self.task_id;
            self.pending_insert_tasks.push(PendingInsertTask {
                first_batch_id: batch_ids[0],
                batch_count,
                local_pending_at_start,
                db_pending_at_start,
                handle: tokio::spawn(async move {
                    let _io = busy.io();
                    let outcome = store
                        .insert_batches(
                            run_id,
                            task_id,
                            requires_training_values,
                            &batch_ids,
                            &batches,
                        )
                        .await?;
                    Ok(outcome.metrics)
                }),
            });
        }
    }

    async fn drain_finished_insert(&mut self) -> Result<(), StoreError> {
        let mut index = 0;
        while index < self.pending_insert_tasks.len() {
            if !self.pending_insert_tasks[index].handle.is_finished() {
                index += 1;
                continue;
            }

            let task = self.pending_insert_tasks.swap_remove(index);
            self.consume_insert_task(task).await?;
        }

        Ok(())
    }

    async fn consume_insert_task(&mut self, task: PendingInsertTask) -> Result<(), StoreError> {
        let metrics = task.handle.await.map_err(|err| {
            StoreError::store(format!("sampler queue insert task failed: {err}"))
        })??;
        self.observe_insert_bundle_start_state(
            task.local_pending_at_start,
            task.db_pending_at_start,
        );
        self.metrics.insert_bundle_ms.observe(metrics.end_to_end_ms);
        self.metrics
            .insert_bundle_batches
            .observe(task.batch_count as f64);
        if task.batch_count > 0 {
            self.metrics.insert_bundle_ms_per_batch.observe_weighted(
                metrics.end_to_end_ms / task.batch_count as f64,
                task.batch_count,
            );
            self.metrics
                .insert_bundle_payload_bytes_per_batch
                .observe_weighted(
                    metrics.payload_bytes as f64 / task.batch_count as f64,
                    task.batch_count,
                );
        }
        self.observe_insert_bundle_store_metrics(&metrics);
        Ok(())
    }

    fn observe_insert_bundle_store_metrics(&mut self, metrics: &InsertBatchesMetrics) {
        self.metrics
            .insert_bundle_serialize_ms
            .observe(metrics.serialize_ms);
        self.metrics
            .insert_bundle_payload_bytes
            .observe(metrics.payload_bytes as f64);
        self.metrics
            .insert_bundle_db_batches_ms
            .observe(metrics.insert_batches_exec_ms);
        self.metrics
            .insert_bundle_db_inputs_ms
            .observe(metrics.insert_inputs_exec_ms);
        self.metrics
            .insert_bundle_commit_ms
            .observe(metrics.commit_ms);
    }

    async fn drain_finished_processed_fetch(&mut self) -> Result<(), StoreError> {
        let Some(task) = self.pending_processed_fetch.as_ref() else {
            return Ok(());
        };
        if !task.is_finished() {
            return Ok(());
        }

        let task = self
            .pending_processed_fetch
            .take()
            .expect("checked pending processed fetch");
        self.consume_processed_fetch_task(task).await
    }

    async fn consume_processed_fetch_task(
        &mut self,
        task: PendingProcessedFetchTask,
    ) -> Result<(), StoreError> {
        let (completed, duration) = match task.await {
            Ok(Ok(completed)) => completed,
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(StoreError::store(format!(
                    "sampler queue completed-batch fetch task failed: {err}"
                )));
            }
        };
        self.metrics.fetch_completed_ms.observe_duration(duration);
        self.metrics
            .fetch_completed_batches
            .observe(completed.len() as f64);
        let fetch_limit = self.config.completed_batch_fetch_limit.max(1) as f64;
        self.metrics
            .fetch_completed_prefetch_fill_ratio
            .observe((completed.len() as f64 / fetch_limit).clamp(0.0, 1.0));
        self.ready_processed.extend(completed);
        Ok(())
    }

    fn tune_batch_size(&mut self) {
        if let Some(size) = self.config.fixed_batch_size {
            self.checkpoint.batch_size_current =
                size.clamp(MIN_BATCH_SIZE, self.effective_max_batch_size());
            return;
        }
        let Some(eval_ms_per_sample) = self.eval_ms_per_sample.value() else {
            return;
        };
        if self.batch_size_tune_cooldown_remaining > 0 {
            self.batch_size_tune_cooldown_remaining -= 1;
            return;
        }
        if self.config.target_batch_eval_ms <= 0.0 || !self.config.target_batch_eval_ms.is_finite()
        {
            return;
        }
        let current_eval_batch_ms = eval_ms_per_sample * self.checkpoint.batch_size_current as f64;
        if current_eval_batch_ms <= 0.0 || !current_eval_batch_ms.is_finite() {
            return;
        }
        let ratio = self.config.target_batch_eval_ms / current_eval_batch_ms;
        if !ratio.is_finite() || ratio <= 0.0 {
            return;
        }
        let deadband = DEFAULT_BATCH_SIZE_DEADBAND_RATIO;
        let lower = 1.0 - deadband;
        let upper = 1.0 + deadband;
        if ratio >= lower && ratio <= upper {
            return;
        }
        let next = ((self.checkpoint.batch_size_current as f64) * ratio).round() as usize;
        let next = next.clamp(MIN_BATCH_SIZE, self.effective_max_batch_size());
        if next == self.checkpoint.batch_size_current {
            return;
        }
        self.checkpoint.batch_size_current = next;
        self.batch_size_tune_cooldown_remaining = DEFAULT_BATCH_SIZE_COOLDOWN_TICKS;
    }

    fn take_ready_processed(&mut self) -> Vec<CompletedBatch> {
        self.ready_processed.drain(..).collect::<Vec<_>>()
    }

    fn effective_max_batch_size(&self) -> usize {
        self.config.max_batch_size.max(MIN_BATCH_SIZE)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::evaluation::{Batch, Point};
    use crate::runners::test_support::RecordingStore;
    use crate::sampling::LatentBatchSpec;

    fn latent_batch_with_weight(weight: f64) -> LatentBatch {
        let batch = Batch::from_points([Point::new(vec![weight], vec![], weight)]).expect("batch");
        LatentBatchSpec::from_batch(&batch).build()
    }

    fn recording_queue(store: RecordingStore) -> SamplerQueue<RecordingStore> {
        SamplerQueue::new(
            store,
            1,
            1,
            SamplerQueueConfig {
                target_batch_eval_ms: 500.0,

                max_batch_size: 4096,
                fixed_batch_size: None,

                max_batches_per_tick: 16,
                max_insert_bundle_size: 1,
                max_concurrent_insert_tasks: 2,
                completed_batch_fetch_limit: 16,
                max_batch_retries: 3,
            },
            SamplerQueueCheckpoint {
                batch_size_current: 128,
                ..SamplerQueueCheckpoint::default()
            },
        )
    }

    #[test]
    fn live_fixed_batch_override_preserves_other_defaults() {
        let base = recording_queue(RecordingStore::default()).config().clone();
        let mut config = base.clone();
        let tuning: SamplerQueueTuning = serde_json::from_value(serde_json::json!({
            "fixed_batch_size": 512, "max_batch_size": 1024
        }))
        .unwrap();
        tuning.validate().unwrap();
        config.apply_tuning(&tuning);
        assert_eq!(config.fixed_batch_size, Some(512));
        assert_eq!(config.max_batch_size, 1024);
        let mut restored = base.clone();
        restored.apply_tuning(&SamplerQueueTuning::default());
        assert_eq!(restored, base);
        let invalid: SamplerQueueTuning =
            serde_json::from_value(serde_json::json!({"fixed_batch_size": 0})).unwrap();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn fixed_batches_survive_adaptation_and_config_updates() {
        let mut queue = recording_queue(RecordingStore::default());
        let mut config = queue.config().clone();
        config.fixed_batch_size = Some(256);
        queue.apply_config(config.clone());
        assert_eq!(queue.current_batch_size(), 256);
        queue.eval_ms_per_sample.observe_batch(10000.0, 16);
        queue.tune_batch_size();
        assert_eq!(queue.current_batch_size(), 256);
        config.max_batch_size = 128;
        queue.apply_config(config);
        assert_eq!(queue.current_batch_size(), 128);
    }

    #[tokio::test]
    async fn io_metrics_use_operation_durations_not_collection_time() {
        let mut queue = recording_queue(RecordingStore::default());
        queue
            .consume_insert_task(PendingInsertTask {
                first_batch_id: 1,
                batch_count: 2,
                local_pending_at_start: 2,
                db_pending_at_start: Some(0),
                handle: tokio::spawn(async {
                    Ok(InsertBatchesMetrics {
                        end_to_end_ms: 24.0,
                        ..Default::default()
                    })
                }),
            })
            .await
            .expect("insert completed");
        queue
            .consume_processed_fetch_task(tokio::spawn(async {
                Ok((vec![], Duration::from_millis(7)))
            }))
            .await
            .expect("fetch completed");
        let cleanup = queue
            .consume_completed_cleanup_task(tokio::spawn(async { Ok(Duration::from_millis(11)) }))
            .await
            .expect("cleanup completed");
        assert_eq!(cleanup, Duration::from_millis(11));
        let metrics = queue.take_metrics_snapshot();
        assert_eq!(metrics.insert_bundle_ms.mean, Some(24.0));
        assert_eq!(metrics.insert_bundle_ms_per_batch.mean, Some(12.0));
        assert_eq!(metrics.fetch_completed_ms.mean, Some(7.0));
        assert_eq!(queue.take_metrics_snapshot().insert_bundle_ms.count, 0);
    }

    #[tokio::test]
    async fn enqueue_fills_free_slots_without_exceeding_the_bound() {
        let store = RecordingStore::default();
        let mut queue = recording_queue(store.clone());
        queue.ingest(vec![latent_batch_with_weight(1.0)], true);
        queue.ingest(vec![latent_batch_with_weight(2.0)], true);
        assert_eq!(queue.pending_insert_tasks.len(), 2);
        queue.ingest(vec![latent_batch_with_weight(3.0)], true);
        assert_eq!(queue.pending_insert_tasks.len(), 2);
        assert_eq!(queue.pending_insert.len(), 1);
        queue.flush().await.unwrap();
        assert_eq!(store.recorded_inserts().len(), 3);
    }

    #[tokio::test]
    async fn completed_fetch_handle_does_not_keep_io_busy() {
        let mut queue = recording_queue(RecordingStore::default());
        queue.ensure_processed_prefetch();
        while !queue
            .pending_processed_fetch
            .as_ref()
            .unwrap()
            .is_finished()
        {
            tokio::task::yield_now().await;
        }
        let occupied = queue.busy.snapshot().io_seconds;
        tokio::task::yield_now().await;
        assert_eq!(queue.busy.snapshot().io_seconds, occupied);
        queue.drain_finished_processed_fetch().await.unwrap();
        assert_eq!(queue.busy.snapshot().io_seconds, occupied);
    }

    #[tokio::test]
    async fn concurrent_insert_tasks_keep_batch_ids_in_production_order() {
        let mut store = RecordingStore::default();
        let commit_first = store.block_first_insert();
        let mut queue = recording_queue(store.clone());

        queue.ingest(
            vec![latent_batch_with_weight(1.0), latent_batch_with_weight(2.0)],
            true,
        );
        store.inserted.notified().await;
        assert_eq!(
            store.recorded_inserts()[0].0,
            2.0,
            "second insert commits first"
        );
        commit_first.notify_one();
        queue.flush().await.expect("queue flush");

        let recorded = store.recorded_inserts();
        assert_eq!(recorded.len(), 2);

        let first_ids = recorded
            .iter()
            .find(|(weight, _)| *weight == 1.0)
            .expect("first logical batch")
            .1
            .clone();
        let second_ids = recorded
            .iter()
            .find(|(weight, _)| *weight == 2.0)
            .expect("second logical batch")
            .1
            .clone();

        assert_eq!(first_ids.len(), 1);
        assert_eq!(second_ids.len(), 1);
        assert!(
            first_ids[0] < second_ids[0],
            "production-order batch ids must stay monotonic: first={:?} second={:?}",
            first_ids,
            second_ids
        );
    }

    #[tokio::test]
    async fn completed_cursor_cannot_skip_uncommitted_or_future_inserts() {
        let store = RecordingStore::default();
        let mut queue = recording_queue(store.clone());
        let ids = next_batch_ids(3);
        *store.completed_ids.lock().unwrap() = vec![ids[0], ids[2]];
        let (commit, committed) = tokio::sync::oneshot::channel();
        queue.pending_insert_tasks.push(PendingInsertTask {
            first_batch_id: ids[1],
            batch_count: 1,
            local_pending_at_start: 1,
            db_pending_at_start: None,
            handle: tokio::spawn(async move {
                committed.await.unwrap();
                Ok(InsertBatchesMetrics::default())
            }),
        });
        queue.ensure_processed_prefetch();
        let fetch = queue.pending_processed_fetch.take().unwrap();
        queue.consume_processed_fetch_task(fetch).await.unwrap();
        let completed = queue.take_ready_processed();
        assert_eq!(
            completed.iter().map(|b| b.batch_id).collect::<Vec<_>>(),
            vec![ids[0]]
        );
        queue.mark_processed(&completed);

        // The delayed bundle commits; it and the previously deferred result are
        // now both visible. A newer insert racing the fetch must still wait.
        commit.send(()).unwrap();
        let insert = queue.pending_insert_tasks.pop().unwrap();
        queue.consume_insert_task(insert).await.unwrap();
        queue.ensure_processed_prefetch();
        let future_id = next_batch_ids(1)[0];
        *store.completed_ids.lock().unwrap() = vec![ids[0], ids[1], ids[2], future_id];
        let fetch = queue.pending_processed_fetch.take().unwrap();
        queue.consume_processed_fetch_task(fetch).await.unwrap();
        let completed = queue.take_ready_processed();
        assert_eq!(
            completed.iter().map(|b| b.batch_id).collect::<Vec<_>>(),
            vec![ids[1], ids[2]]
        );
        queue.mark_processed(&completed);
        queue.ensure_processed_prefetch();
        let fetch = queue.pending_processed_fetch.take().unwrap();
        queue.consume_processed_fetch_task(fetch).await.unwrap();
        assert_eq!(queue.take_ready_processed()[0].batch_id, future_id);
    }

    #[tokio::test]
    async fn get_processed_ready_does_not_start_completed_fetch() {
        let store = RecordingStore::default();
        let mut queue = recording_queue(store.clone());

        let processed = queue
            .get_processed_ready()
            .await
            .expect("non-blocking processed drain");

        assert!(processed.is_empty());
        assert_eq!(store.fetch_completed_calls(), 0);
    }
    #[tokio::test]
    async fn refill_threshold_counts_local_work_and_allows_overshoot() {
        let mut queue = recording_queue(RecordingStore::default());
        queue
            .pending_insert
            .push_back((latent_batch_with_weight(1.0), false));
        let counts = queue.queue_counts_with_local_buffer(BatchQueueCounts {
            pending: 19,
            ..Default::default()
        });
        assert_eq!(counts.pending, 20);
        assert!(!queue.needs_generation(counts).await.unwrap());
        assert!(
            queue
                .needs_generation(BatchQueueCounts {
                    pending: 19,
                    ..Default::default()
                })
                .await
                .unwrap()
        );
        assert!(
            !queue
                .needs_generation(BatchQueueCounts {
                    pending: 80,
                    ..Default::default()
                })
                .await
                .unwrap()
        );
    }

    #[test]
    fn training_chunks_cover_workers_without_shrinking_the_tail() {
        let mut sizing = TrainingBatchSizing::default();
        assert_eq!(sizing.batch_size(5000, Some(10000), 2), 1250);
        assert_eq!(sizing.batch_size(5000, Some(7500), 2), 1250);
        assert_eq!(sizing.batch_size(5000, Some(100), 2), 1250);
        assert_eq!(sizing.batch_size(500, Some(100), 2), 500);
        assert_eq!(sizing.batch_size(5000, Some(20000), 2), 2500);
        assert_eq!(sizing.batch_size(5000, None, 2), 5000);
        assert_eq!(sizing.batch_size(5000, Some(10000), 4), 625);
        sizing.batch_size(5000, Some(0), 4);
        assert_eq!(sizing.batch_size(5000, Some(8), 4), MIN_BATCH_SIZE);
        assert_eq!(sizing.batch_size(5000, Some(8), 0), 5000);
    }
    #[test]
    fn timing_smoothing_preserves_history_and_is_batch_partition_invariant() {
        let mut whole = recording_queue(RecordingStore::default());
        let mut split = recording_queue(RecordingStore::default());
        whole.observe_completed_eval_batch(5000, 5000.0);
        split.observe_completed_eval_batch(5000, 5000.0);
        whole.observe_completed_eval_batch(5000, 10000.0);
        for _ in 0..10 {
            split.observe_completed_eval_batch(500, 1000.0);
        }
        let mean = whole.eval_ms_per_sample.value().unwrap();
        assert!(
            mean > 1.0 && mean < 2.0,
            "large batches must retain timing history"
        );
        assert!((mean - split.eval_ms_per_sample.value().unwrap()).abs() < 1e-12);
    }
}
