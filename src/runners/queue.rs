use crate::core::SamplerQueueCheckpoint;
use crate::core::{
    BatchQueueCounts, CompletedBatch, InsertBatchesMetrics, SamplerQueueRollingAverages,
    SamplerQueueRuntimeMetrics, SamplerQueueTuning, SamplerWorkerStore, StoreError, next_batch_ids,
};
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
    pub queue_buffer: f64,
    pub target_batch_eval_ms: f64,
    #[serde(default = "default_batch_size_deadband_ratio")]
    pub batch_size_deadband_ratio: f64,
    #[serde(default = "default_batch_size_cooldown_ticks")]
    pub batch_size_cooldown_ticks: u32,
    pub max_batch_size: usize,
    /// Disable adaptation; training boundaries and remaining budgets still cap batches.
    #[serde(default)]
    pub fixed_batch_size: Option<usize>,
    pub max_queue_size: usize,
    pub max_batches_per_tick: usize,
    pub max_insert_bundle_size: usize,
    pub max_concurrent_insert_tasks: usize,
    pub completed_batch_fetch_limit: usize,
    #[serde(default = "default_max_batch_retries")]
    pub max_batch_retries: i32,
}

impl SamplerQueueConfig {
    pub fn apply_tuning(&mut self, tuning: &SamplerQueueTuning) {
        apply_option(&mut self.queue_buffer, tuning.queue_buffer);
        apply_option(&mut self.target_batch_eval_ms, tuning.target_batch_eval_ms);
        apply_option(
            &mut self.batch_size_deadband_ratio,
            tuning.batch_size_deadband_ratio,
        );
        apply_option(
            &mut self.batch_size_cooldown_ticks,
            tuning.batch_size_cooldown_ticks,
        );
        apply_option(&mut self.max_batch_size, tuning.max_batch_size);
        apply_option(&mut self.max_queue_size, tuning.max_queue_size);
        apply_option(&mut self.max_batches_per_tick, tuning.max_batches_per_tick);
        apply_option(
            &mut self.max_insert_bundle_size,
            tuning.max_insert_bundle_size,
        );
        apply_option(
            &mut self.max_concurrent_insert_tasks,
            tuning.max_concurrent_insert_tasks,
        );
        apply_option(
            &mut self.completed_batch_fetch_limit,
            tuning.completed_batch_fetch_limit,
        );
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
    requires_training_values: bool,
    store: S,
    config: SamplerQueueConfig,
    checkpoint: SamplerQueueCheckpoint,
    pending_insert: VecDeque<LatentBatch>,
    ready_processed: VecDeque<CompletedBatch>,
    pending_insert_tasks: Vec<PendingInsertTask>,
    pending_processed_fetch: Option<PendingProcessedFetchTask>,
    pending_completed_cleanup: Option<PendingCompletedCleanupTask>,
    cached_db_queue_counts: Option<BatchQueueCounts>,
    cached_tick_queue_counts: Option<BatchQueueCounts>,
    cached_active_evaluator_count: Option<usize>,
    last_reclaim_at: Instant,
    last_completed_cleanup_at: Instant,
    batch_size_tune_cooldown_remaining: u32,
    eval_ms_per_sample: RollingMetric,
    training_batch_sizing: TrainingBatchSizing,
    metrics: QueueMetricsState,
    utilization: QueueUtilizationState,
}

/// Keep a finite training window divisible across workers, without recursively
/// shrinking chunks as its remaining sample count decreases.
#[derive(Default)]
struct TrainingBatchSizing {
    previous_remaining: Option<usize>,
    cap: Option<usize>,
}

impl TrainingBatchSizing {
    fn batch_size(&mut self, target: usize, remaining: Option<usize>, evaluators: usize) -> usize {
        match remaining {
            Some(remaining) if remaining > 0 && evaluators > 0 => {
                if self
                    .previous_remaining
                    .is_none_or(|previous| remaining > previous)
                {
                    self.cap = Some(
                        remaining
                            .div_ceil(evaluators.saturating_mul(4))
                            .max(MIN_BATCH_SIZE),
                    );
                }
                self.previous_remaining = Some(remaining);
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

const fn default_batch_size_deadband_ratio() -> f64 {
    DEFAULT_BATCH_SIZE_DEADBAND_RATIO
}

const fn default_batch_size_cooldown_ticks() -> u32 {
    DEFAULT_BATCH_SIZE_COOLDOWN_TICKS
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

#[derive(Debug, Clone, Copy, Default)]
pub struct QueueUtilizationSnapshot {
    pub insert_task_utilization: Option<f64>,
    pub completed_fetch_utilization: Option<f64>,
}

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

#[derive(Debug, Clone)]
struct QueueUtilizationState {
    window_started_at: Instant,
    last_accounted_at: Instant,
    insert_busy_slot_secs: f64,
    completed_fetch_busy_secs: f64,
}

impl QueueUtilizationState {
    fn new(now: Instant) -> Self {
        Self {
            window_started_at: now,
            last_accounted_at: now,
            insert_busy_slot_secs: 0.0,
            completed_fetch_busy_secs: 0.0,
        }
    }
}

impl<S> SamplerQueue<S>
where
    S: SamplerWorkerStore + Clone + Send + Sync + 'static,
{
    pub fn new(
        store: S,
        run_id: i32,
        task_id: i64,
        requires_training_values: bool,
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
            requires_training_values,
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
            last_reclaim_at: now.checked_sub(RECLAIM_INTERVAL).unwrap_or(now),
            last_completed_cleanup_at: now.checked_sub(COMPLETED_CLEANUP_INTERVAL).unwrap_or(now),
            batch_size_tune_cooldown_remaining: 0,
            eval_ms_per_sample: RollingMetric::default(),
            training_batch_sizing: TrainingBatchSizing::default(),
            metrics: QueueMetricsState::default(),
            utilization: QueueUtilizationState::new(now),
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
            insert_task_utilization: None,
            completed_fetch_utilization: None,
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

    pub fn take_utilization_snapshot(&mut self) -> QueueUtilizationSnapshot {
        let now = Instant::now();
        self.account_utilization(now);
        let elapsed_secs = now
            .saturating_duration_since(self.utilization.window_started_at)
            .as_secs_f64();
        let insert_capacity = self.config.max_concurrent_insert_tasks.max(1) as f64;
        let snapshot = if elapsed_secs <= 0.0 {
            QueueUtilizationSnapshot::default()
        } else {
            QueueUtilizationSnapshot {
                insert_task_utilization: Some(
                    (self.utilization.insert_busy_slot_secs / (elapsed_secs * insert_capacity))
                        .clamp(0.0, 1.0),
                ),
                completed_fetch_utilization: Some(
                    (self.utilization.completed_fetch_busy_secs / elapsed_secs).clamp(0.0, 1.0),
                ),
            }
        };
        self.utilization.window_started_at = now;
        self.utilization.last_accounted_at = now;
        self.utilization.insert_busy_slot_secs = 0.0;
        self.utilization.completed_fetch_busy_secs = 0.0;
        snapshot
    }

    pub fn last_completed_batch_id(&self) -> Option<i64> {
        self.checkpoint.last_completed_batch_id
    }

    pub async fn queue_counts(&mut self) -> Result<BatchQueueCounts, StoreError> {
        let counts = self.db_queue_counts().await?;
        Ok(self.queue_counts_with_local_buffer(counts))
    }

    async fn db_queue_counts(&mut self) -> Result<BatchQueueCounts, StoreError> {
        let counts = self
            .store
            .get_batch_queue_counts(self.run_id, self.last_completed_batch_id())
            .await?;
        self.cached_db_queue_counts = Some(counts);
        Ok(counts)
    }

    async fn reclaim_abandoned_batches(&self) -> Result<u64, StoreError> {
        self.store.reclaim_abandoned_batches(self.run_id).await
    }

    async fn cleanup_consumed_completed_batches(&self) -> Result<u64, StoreError> {
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

    pub async fn plan_production(
        &mut self,
        max_producable: Option<usize>,
        training_remaining: Option<usize>,
        queue_counts: BatchQueueCounts,
    ) -> Result<Vec<usize>, StoreError> {
        let active_evaluator_count = self
            .store
            .count_active_evaluator_nodes(self.run_id)
            .await?
            .max(0) as usize;
        self.cached_active_evaluator_count = Some(active_evaluator_count);
        self.cached_tick_queue_counts = Some(queue_counts);
        let batch_size = self.training_batch_sizing.batch_size(
            self.checkpoint.batch_size_current,
            training_remaining,
            active_evaluator_count,
        );
        Ok(self.get_sample(
            max_producable,
            queue_counts,
            active_evaluator_count,
            batch_size,
        ))
    }

    pub fn validate_batch_plan(&self, batch_plan: &[usize]) -> Result<(), StoreError> {
        if batch_plan.len() > self.config.max_batches_per_tick {
            return Err(StoreError::store(format!(
                "batch plan exceeded max_batches_per_tick: planned={} max_batches_per_tick={}",
                batch_plan.len(),
                self.config.max_batches_per_tick
            )));
        }
        if let Some(max_planned_batch_size) = batch_plan.iter().copied().max()
            && max_planned_batch_size > self.effective_max_batch_size()
        {
            return Err(StoreError::store(format!(
                "batch plan exceeded max_batch_size: planned={} max_batch_size={}",
                max_planned_batch_size,
                self.effective_max_batch_size()
            )));
        }
        Ok(())
    }

    pub fn diagnostics_snapshot(&self) -> Option<QueueDiagnosticsSnapshot> {
        self.cached_tick_queue_counts
            .map(|queue_counts| QueueDiagnosticsSnapshot {
                queue_counts,
                active_evaluator_count: self.cached_active_evaluator_count,
            })
    }

    pub fn target_pending_batches(&self, active_evaluator_count: usize) -> Option<usize> {
        if !self.config.queue_buffer.is_finite() || self.config.queue_buffer < 0.0 {
            return None;
        }
        Some(((active_evaluator_count as f64) * self.config.queue_buffer).ceil() as usize)
    }

    pub fn ingest(&mut self, batches: Vec<LatentBatch>) {
        if batches.is_empty() {
            return;
        }

        self.pending_insert.extend(batches);
        self.start_insert_pump_if_idle();
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
        self.drain_finished_processed_fetch().await?;
        self.ensure_processed_prefetch();

        Ok(self.take_ready_processed())
    }

    pub(crate) async fn get_processed_ready(&mut self) -> Result<Vec<CompletedBatch>, StoreError> {
        self.drain_finished_insert().await?;
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

    pub fn get_sample(
        &self,
        max_producable: Option<usize>,
        queue_counts: BatchQueueCounts,
        active_evaluator_count: usize,
        batch_size_current: usize,
    ) -> Vec<usize> {
        let pending_before = queue_counts.pending.max(0) as usize;
        let open_before = queue_counts.open().max(0) as usize;
        let remaining_capacity = self.config.max_queue_size.saturating_sub(open_before);
        let hard_limit = remaining_capacity.min(self.config.max_batches_per_tick);
        if hard_limit == 0 {
            return Vec::new();
        }

        let Some(target_pending) = self.target_pending_batches(active_evaluator_count) else {
            return Vec::new();
        };
        // queue_counts includes local and in-flight inserts. One pending target
        // therefore bounds both the database queue and unpersisted production.
        let batch_limit = hard_limit.min(target_pending.saturating_sub(pending_before));
        if batch_limit == 0 {
            return Vec::new();
        }

        match max_producable {
            None => vec![batch_size_current; batch_limit],
            Some(max_samples) => {
                let base_total_samples = batch_limit.saturating_mul(batch_size_current);
                if base_total_samples <= max_samples {
                    vec![batch_size_current; batch_limit]
                } else if max_samples == 0 || batch_size_current == 0 {
                    Vec::new()
                } else {
                    let nr_batches = max_samples.div_ceil(batch_size_current);
                    let base_size = max_samples / nr_batches;
                    let remainder = max_samples % nr_batches;
                    let mut plan = Vec::with_capacity(nr_batches);
                    for i in 0..nr_batches {
                        plan.push(if i < remainder {
                            base_size + 1
                        } else {
                            base_size
                        });
                    }
                    plan
                }
            }
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
        let store = self.store.clone();
        let run_id = self.run_id;
        self.pending_completed_cleanup = Some(tokio::spawn(async move {
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

        self.account_utilization(Instant::now());
        let store = self.store.clone();
        let run_id = self.run_id;
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
            let started = Instant::now();
            let batches = store
                .fetch_completed_batches(run_id, fetch_limit, true, after_batch_id)
                .await?;
            let batches = batches
                .into_iter()
                .take_while(|batch| batch.batch_id < before_batch_id)
                .collect();
            Ok((batches, started.elapsed()))
        }));
    }

    fn start_insert_pump_if_idle(&mut self) {
        if !self.pending_insert_tasks.is_empty() || self.pending_insert.is_empty() {
            return;
        }

        self.ensure_insert_pump();
    }

    fn ensure_insert_pump(&mut self) {
        let max_concurrent_insert_tasks = self.config.max_concurrent_insert_tasks.max(1);
        while self.pending_insert_tasks.len() < max_concurrent_insert_tasks
            && !self.pending_insert.is_empty()
        {
            self.account_utilization(Instant::now());
            let (local_pending_at_start, db_pending_at_start) =
                self.snapshot_insert_bundle_start_state();

            let bundle_size = self.config.max_insert_bundle_size.max(1);
            let batch_count = self.pending_insert.len().min(bundle_size);
            let batches = self.pending_insert.drain(..batch_count).collect::<Vec<_>>();
            let batch_ids = next_batch_ids(batch_count);
            self.checkpoint.last_produced_batch_id = batch_ids.last().copied();
            let store = self.store.clone();
            let run_id = self.run_id;
            let task_id = self.task_id;
            let requires_training_values = self.requires_training_values;
            self.pending_insert_tasks.push(PendingInsertTask {
                first_batch_id: batch_ids[0],
                batch_count,
                local_pending_at_start,
                db_pending_at_start,
                handle: tokio::spawn(async move {
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

            self.account_utilization(Instant::now());
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
            self.metrics
                .insert_bundle_ms_per_batch
                .observe(metrics.end_to_end_ms / task.batch_count as f64);
            self.metrics
                .insert_bundle_payload_bytes_per_batch
                .observe(metrics.payload_bytes as f64 / task.batch_count as f64);
        }
        self.observe_insert_bundle_store_metrics(&metrics);
        self.ensure_insert_pump();
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
        self.account_utilization(Instant::now());
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

    fn account_utilization(&mut self, now: Instant) {
        let elapsed_secs = now
            .saturating_duration_since(self.utilization.last_accounted_at)
            .as_secs_f64();
        if elapsed_secs > 0.0 {
            self.utilization.insert_busy_slot_secs +=
                elapsed_secs * self.pending_insert_tasks.len() as f64;
            if self.pending_processed_fetch.is_some() {
                self.utilization.completed_fetch_busy_secs += elapsed_secs;
            }
        }
        self.utilization.last_accounted_at = now;
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
        let deadband = self.sanitized_batch_size_deadband_ratio();
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
        self.batch_size_tune_cooldown_remaining = self.config.batch_size_cooldown_ticks;
    }

    fn take_ready_processed(&mut self) -> Vec<CompletedBatch> {
        self.ready_processed.drain(..).collect::<Vec<_>>()
    }

    fn effective_max_batch_size(&self) -> usize {
        self.config.max_batch_size.max(MIN_BATCH_SIZE)
    }

    fn sanitized_batch_size_deadband_ratio(&self) -> f64 {
        let value = self.config.batch_size_deadband_ratio;
        if !value.is_finite() || value < 0.0 {
            DEFAULT_BATCH_SIZE_DEADBAND_RATIO
        } else {
            value.min(0.95)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        AggregationStore, BatchClaim, ControlPlaneStore, DesiredAssignment,
        EvaluatorPerformanceSnapshot, InsertBatchesOutcome, RegisteredNode, RunReadStore,
        RunSampleProgress, RunStageSnapshot, RunTask, RunTaskInput, RunTaskStore,
        SamplerAggregatorPerformanceSnapshot, WorkQueueStore,
    };
    use crate::evaluation::{Batch, Point};
    use crate::sampling::{LatentBatchPayload, LatentBatchSpec};
    use crate::stores::{
        EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress, RuntimeLogPage,
        SamplerPerformanceHistoryEntry, TaskOutputSnapshot, TaskStageSnapshot,
    };
    use crate::utils::domain::Domain;
    use async_trait::async_trait;
    use serde_json::Value as JsonValue;
    use std::sync::{Arc, Mutex};

    type RecordedInserts = Arc<Mutex<Vec<(f64, Vec<i64>)>>>;

    #[derive(Clone, Default)]
    struct RecordingStore {
        inserts: RecordedInserts,
        fetch_completed_calls: Arc<Mutex<usize>>,
        completed_ids: Arc<Mutex<Vec<i64>>>,
    }

    impl RecordingStore {
        fn recorded_inserts(&self) -> Vec<(f64, Vec<i64>)> {
            self.inserts.lock().expect("recording lock").clone()
        }

        fn fetch_completed_calls(&self) -> usize {
            *self.fetch_completed_calls.lock().expect("recording lock")
        }
    }

    #[async_trait]
    impl WorkQueueStore for RecordingStore {
        async fn insert_batches(
            &self,
            _run_id: i32,
            _task_id: i64,
            _requires_training_values: bool,
            batch_ids: &[i64],
            batches: &[LatentBatch],
        ) -> Result<InsertBatchesOutcome, StoreError> {
            let logical_weight = match &batches[0].payload {
                LatentBatchPayload::IndexedBatch { weights, .. } => weights[0],
                LatentBatchPayload::HavanaInference { .. } => 0.0,
            };
            if logical_weight == 1.0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            self.inserts
                .lock()
                .expect("recording lock")
                .push((logical_weight, batch_ids.to_vec()));
            Ok(InsertBatchesOutcome {
                batch_ids: batch_ids.to_vec(),
                metrics: InsertBatchesMetrics::default(),
            })
        }

        async fn get_batch_queue_counts(
            &self,
            _run_id: i32,
            _completed_after_batch_id: Option<i64>,
        ) -> Result<BatchQueueCounts, StoreError> {
            unreachable!("unused in test")
        }

        async fn claim_batch(
            &self,
            _run_id: i32,
            _node_uuid: &str,
        ) -> Result<Option<BatchClaim>, StoreError> {
            unreachable!("unused in test")
        }

        async fn release_claimed_batches_for_worker(
            &self,
            _run_id: i32,
            _node_uuid: &str,
        ) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn submit_batch_results(
            &self,
            _batch_id: i64,
            _node_uuid: &str,
            _result: &crate::evaluation::BatchResult,
            _eval_time_ms: f64,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn record_evaluator_performance_snapshot(
            &self,
            _snapshot: &EvaluatorPerformanceSnapshot,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn record_sampler_performance_snapshot(
            &self,
            _snapshot: &SamplerAggregatorPerformanceSnapshot,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn fail_batch(
            &self,
            _batch_id: i64,
            _last_error: &str,
            _max_batch_retries: i32,
        ) -> Result<crate::core::BatchFailOutcome, StoreError> {
            unreachable!("unused in test")
        }

        async fn fetch_completed_batches(
            &self,
            _run_id: i32,
            _limit: usize,
            _strict_ordering: bool,
            _after_batch_id: Option<i64>,
        ) -> Result<Vec<crate::core::CompletedBatch>, StoreError> {
            *self.fetch_completed_calls.lock().expect("recording lock") += 1;
            Ok(self
                .completed_ids
                .lock()
                .unwrap()
                .iter()
                .copied()
                .filter(|id| *id > _after_batch_id.unwrap_or(0))
                .map(|batch_id| CompletedBatch {
                    batch_id,
                    task_id: 1,
                    requires_training_values: true,
                    batch_size: 1,
                    result: crate::evaluation::BatchResult::new(
                        None,
                        crate::evaluation::AccumulatorState::Empty(Default::default()),
                    ),
                    completed_at: None,
                    total_eval_time_ms: None,
                })
                .collect())
        }

        async fn cleanup_consumed_completed_batches(
            &self,
            _run_id: i32,
            _up_to_batch_id: i64,
            _limit: usize,
        ) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn reclaim_abandoned_batches(&self, _run_id: i32) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }
    }

    #[async_trait]
    impl AggregationStore for RecordingStore {
        async fn load_current_accumulator(
            &self,
            _run_id: i32,
        ) -> Result<Option<JsonValue>, StoreError> {
            unreachable!("unused in test")
        }

        async fn persist_task_result_snapshot(
            &self,
            _run_id: i32,
            _task_id: i64,
            _result: &JsonValue,
        ) -> Result<i64, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_sampler_checkpoint(
            &self,
            _run_id: i32,
        ) -> Result<Option<crate::core::SamplerAggregatorCheckpoint>, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_stage_snapshot(
            &self,
            _snapshot_id: i64,
        ) -> Result<Option<RunStageSnapshot>, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_latest_stage_snapshot_before_sequence(
            &self,
            _run_id: i32,
            _sequence_nr: i32,
        ) -> Result<Option<RunStageSnapshot>, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_task_activation_snapshot(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<Option<RunStageSnapshot>, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_run_sample_progress(
            &self,
            _run_id: i32,
        ) -> Result<Option<RunSampleProgress>, StoreError> {
            unreachable!("unused in test")
        }

        async fn save_aggregation(
            &self,
            _run_id: i32,
            _task_id: i64,
            _current_accumulator: &JsonValue,
            _persisted_observable: Option<&JsonValue>,
            _delta_batches_completed: i32,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn save_sampler_checkpoint(
            &self,
            _run_id: i32,
            _checkpoint: &crate::core::SamplerAggregatorCheckpoint,
            _stage: Option<&crate::core::RunStageSnapshot>,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn save_run_sample_progress(
            &self,
            _run_id: i32,
            _nr_produced_samples: i64,
            _nr_completed_samples: i64,
            _sampler_runner_uptime_ms: f64,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn save_run_stage_snapshot(
            &self,
            _snapshot: &RunStageSnapshot,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }
    }

    #[async_trait]
    impl RunTaskStore for RecordingStore {
        async fn append_run_tasks(
            &self,
            _run_id: i32,
            _tasks: &[RunTaskInput],
        ) -> Result<Vec<RunTask>, StoreError> {
            unreachable!("unused in test")
        }

        async fn list_run_tasks(&self, _run_id: i32) -> Result<Vec<RunTask>, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_run_task(&self, _task_id: i64) -> Result<Option<RunTask>, StoreError> {
            unreachable!("unused in test")
        }

        async fn remove_pending_run_task(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<bool, StoreError> {
            unreachable!("unused in test")
        }

        async fn update_run_task_queue_tuning(
            &self,
            _run_id: i32,
            _task_id: i64,
            _queue_tuning: Option<crate::core::SamplerQueueTuning>,
        ) -> Result<RunTask, StoreError> {
            unreachable!("unused in test")
        }

        async fn load_active_run_task(&self, _run_id: i32) -> Result<Option<RunTask>, StoreError> {
            unreachable!("unused in test")
        }

        async fn activate_next_run_task(
            &self,
            _run_id: i32,
        ) -> Result<Option<RunTask>, StoreError> {
            unreachable!("unused in test")
        }

        async fn update_run_task_progress(
            &self,
            _task_id: i64,
            _nr_produced_samples: i64,
            _nr_completed_samples: i64,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn set_run_task_spawn_origin(
            &self,
            _task_id: i64,
            _spawned_from_snapshot_id: Option<i64>,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn complete_run_task(&self, _task_id: i64) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn persist_task_measurement_output(
            &self,
            _task_id: i64,
            _output: &crate::core::TaskMeasurementOutput,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn persist_task_controller_output(
            &self,
            _task_id: i64,
            _output: &crate::core::ControllerTaskOutput,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn fail_run_task(&self, _task_id: i64, _reason: &str) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }
    }

    #[async_trait]
    impl ControlPlaneStore for RecordingStore {
        async fn update_desired_assignments(
            &self,
            _updates: &[crate::core::NodeAssignmentUpdate],
        ) -> Result<bool, crate::core::StoreError> {
            unreachable!("unused in test")
        }

        async fn upsert_desired_assignment(
            &self,
            _node_name: &str,
            _role: crate::core::WorkerRole,
            _run_id: i32,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn announce_node(
            &self,
            _node_name: &str,
            _node_uuid: &str,
            _capabilities: &crate::core::NodeCapabilities,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn set_current_assignment(
            &self,
            _node_uuid: &str,
            _role: crate::core::WorkerRole,
            _run_id: i32,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn clear_current_assignment(&self, _node_uuid: &str) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn clear_desired_assignment(&self, _node_name: &str) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn clear_desired_assignments_for_run(&self, _run_id: i32) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn clear_desired_assignments_for_run_except_node(
            &self,
            _run_id: i32,
            _keep_node_name: &str,
        ) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn clear_all_desired_assignments(&self) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_desired_assignment(
            &self,
            _node_name: &str,
        ) -> Result<Option<DesiredAssignment>, StoreError> {
            unreachable!("unused in test")
        }

        async fn list_desired_assignments(
            &self,
            _node_name: Option<&str>,
        ) -> Result<Vec<DesiredAssignment>, StoreError> {
            unreachable!("unused in test")
        }

        async fn list_nodes(
            &self,
            _node_name: Option<&str>,
        ) -> Result<Vec<RegisteredNode>, StoreError> {
            unreachable!("unused in test")
        }

        async fn create_node_launch_request(
            &self,
            _backend: &str,
            _requested_count: i32,
            _name_prefix: Option<&str>,
            _args: &JsonValue,
        ) -> Result<crate::core::NodeLaunchRequest, StoreError> {
            unreachable!("unused in test")
        }

        async fn list_node_launch_requests(
            &self,
        ) -> Result<Vec<crate::core::NodeLaunchRequest>, StoreError> {
            unreachable!("unused in test")
        }

        async fn claim_external_node_launch_request(
            &self,
        ) -> Result<Option<crate::core::NodeLaunchRequest>, StoreError> {
            unreachable!("unused in test")
        }

        async fn update_node_launch_request_state(
            &self,
            _id: i64,
            _state: &str,
            _started_count: i32,
            _result: &JsonValue,
            _error: Option<&str>,
        ) -> Result<crate::core::NodeLaunchRequest, StoreError> {
            unreachable!("unused in test")
        }

        async fn count_active_evaluator_nodes(&self, _run_id: i32) -> Result<i64, StoreError> {
            unreachable!("unused in test")
        }

        async fn request_node_shutdown(&self, _node_name: &str) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn request_all_nodes_shutdown(&self) -> Result<u64, StoreError> {
            unreachable!("unused in test")
        }

        async fn consume_node_shutdown_request(
            &self,
            _node_uuid: &str,
        ) -> Result<bool, StoreError> {
            unreachable!("unused in test")
        }

        async fn expire_node_lease(&self, _node_uuid: &str) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn create_run(
            &self,
            _name: &str,
            _run_toml: &str,
            _provenance: &JsonValue,
            _integration_params: &JsonValue,
            _target: Option<&JsonValue>,
            _domain: &Domain,
            _initial_stage_snapshot: &RunStageSnapshot,
            _initial_tasks: &[RunTaskInput],
        ) -> Result<i32, StoreError> {
            unreachable!("unused in test")
        }

        async fn set_run_parent_metadata(
            &self,
            _run_id: i32,
            _parent_run_id: i32,
            _parent_task_id: Option<i64>,
            _spawn_kind: &str,
            _spawn_label: Option<&str>,
        ) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn remove_run(&self, _run_id: i32) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }
    }

    #[async_trait]
    impl RunReadStore for RecordingStore {
        async fn health_check(&self) -> Result<(), StoreError> {
            unreachable!("unused in test")
        }

        async fn get_all_runs(&self) -> Result<Vec<RunProgress>, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_run_progress(&self, _run_id: i32) -> Result<Option<RunProgress>, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_task_output_snapshots(
            &self,
            _run_id: i32,
            _task_id: i64,
            _after_snapshot_id: Option<i64>,
            _limit: i64,
        ) -> Result<Vec<TaskOutputSnapshot>, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_latest_task_stage_snapshot(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<Option<TaskStageSnapshot>, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_latest_task_stage_snapshot_id(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<Option<String>, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_runtime_logs(
            &self,
            _limit: i64,
            _source: Option<&str>,
            _run_id: Option<i32>,
            _include_child_runs: bool,
            _node_name: Option<&str>,
            _node_uuid: Option<&str>,
            _level: Option<&str>,
            _query: Option<&str>,
            _before_id: Option<i64>,
        ) -> Result<RuntimeLogPage, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_registered_workers(
            &self,
            _run_id: Option<i32>,
        ) -> Result<Vec<RegisteredWorkerEntry>, StoreError> {
            Ok(Vec::new())
        }

        async fn get_evaluator_performance_history(
            &self,
            _run_id: i32,
            _limit: i64,
            _worker_id: Option<&str>,
        ) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, StoreError> {
            unreachable!("unused in test")
        }

        async fn get_sampler_performance_history(
            &self,
            _run_id: i32,
            _limit: i64,
            _worker_id: Option<&str>,
        ) -> Result<Vec<SamplerPerformanceHistoryEntry>, StoreError> {
            unreachable!("unused in test")
        }
    }

    fn latent_batch_with_weight(weight: f64) -> LatentBatch {
        let batch = Batch::from_points([Point::new(vec![weight], vec![], weight)]).expect("batch");
        LatentBatchSpec::from_batch(&batch).build()
    }

    fn recording_queue(store: RecordingStore) -> SamplerQueue<RecordingStore> {
        SamplerQueue::new(
            store,
            1,
            1,
            true,
            SamplerQueueConfig {
                queue_buffer: 1.0,
                target_batch_eval_ms: 500.0,
                batch_size_deadband_ratio: 0.15,
                batch_size_cooldown_ticks: 3,
                max_batch_size: 4096,
                fixed_batch_size: None,
                max_queue_size: 16,
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
    async fn concurrent_insert_tasks_keep_batch_ids_in_production_order() {
        let store = RecordingStore::default();
        let mut queue = recording_queue(store.clone());

        queue.ingest(vec![
            latent_batch_with_weight(1.0),
            latent_batch_with_weight(2.0),
        ]);
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
    #[test]
    fn pending_target_counts_unpersisted_work_and_respects_capacity() {
        let mut queue = recording_queue(RecordingStore::default());
        queue
            .pending_insert
            .push_back(latent_batch_with_weight(1.0));
        let counts = queue.queue_counts_with_local_buffer(BatchQueueCounts {
            pending: 1,
            claimed: 2,
            ..Default::default()
        });
        assert_eq!(counts.pending, 2);
        assert_eq!(queue.get_sample(None, counts, 4, 128), vec![128; 2]);
        queue.config.max_queue_size = 5;
        assert_eq!(queue.get_sample(None, counts, 4, 128), vec![128]);
        queue.config.max_queue_size = 16;
        queue.config.max_batches_per_tick = 1;
        assert_eq!(queue.get_sample(None, counts, 4, 128), vec![128]);
        queue.config.max_batches_per_tick = 16;
        assert_eq!(queue.get_sample(Some(150), counts, 4, 128), vec![75; 2]);
        assert!(queue.get_sample(Some(0), counts, 4, 128).is_empty());
        assert!(queue.get_sample(None, counts, 0, 128).is_empty());
        queue.config.queue_buffer = 0.0;
        assert!(queue.get_sample(None, counts, 4, 128).is_empty());
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
