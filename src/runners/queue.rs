use crate::core::{
    BatchQueueCounts, CompletedBatch, RollingMetricSnapshot, SamplerQueueRuntimeMetrics,
    SamplerWorkerStore, StoreError,
};
use crate::runners::rolling_metric::RollingMetric;
use crate::sampling::LatentBatch;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

const RECLAIM_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETED_CLEANUP_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETED_CLEANUP_BATCH_LIMIT: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerQueueConfig {
    pub queue_buffer: f64,
    pub local_pending_buffer_multiplier: f64,
    pub max_queue_size: usize,
    pub max_batches_per_tick: usize,
    pub max_insert_bundle_size: usize,
    pub completed_batch_fetch_limit: usize,
    pub strict_batch_ordering: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SamplerQueueCheckpoint {
    #[serde(default)]
    pub last_completed_batch_id: Option<i64>,
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
    pending_insert_task: Option<PendingInsertTask>,
    insert_pump_running: bool,
    pending_processed_fetch: Option<PendingProcessedFetchTask>,
    cached_tick_queue_counts: Option<BatchQueueCounts>,
    cached_active_evaluator_count: Option<usize>,
    last_reclaim_at: Instant,
    last_completed_cleanup_at: Instant,
    metrics: QueueMetricsState,
}

struct PendingInsertTask {
    batch_count: usize,
    started_at: Instant,
    handle: JoinHandle<Result<(), StoreError>>,
}

struct PendingProcessedFetchTask {
    started_at: Instant,
    handle: JoinHandle<Result<Vec<CompletedBatch>, StoreError>>,
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
    get_processed_ms: RollingMetric,
    fetch_completed_ms: RollingMetric,
    insert_bundle_ms: RollingMetric,
    insert_bundle_ms_per_batch: RollingMetric,
    insert_bundle_local_pending_at_start: RollingMetric,
    flush_ms: RollingMetric,
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
        checkpoint: SamplerQueueCheckpoint,
    ) -> Self {
        let now = Instant::now();
        Self {
            run_id,
            task_id,
            requires_training_values,
            store,
            config,
            checkpoint,
            pending_insert: VecDeque::new(),
            ready_processed: VecDeque::new(),
            pending_insert_task: None,
            insert_pump_running: false,
            pending_processed_fetch: None,
            cached_tick_queue_counts: None,
            cached_active_evaluator_count: None,
            last_reclaim_at: now.checked_sub(RECLAIM_INTERVAL).unwrap_or(now),
            last_completed_cleanup_at: now
                .checked_sub(COMPLETED_CLEANUP_INTERVAL)
                .unwrap_or(now),
            metrics: QueueMetricsState::default(),
        }
    }

    pub fn config(&self) -> &SamplerQueueConfig {
        &self.config
    }

    pub fn checkpoint(&self) -> SamplerQueueCheckpoint {
        self.checkpoint.clone()
    }

    pub fn runtime_metrics(&self) -> SamplerQueueRuntimeMetrics {
        SamplerQueueRuntimeMetrics {
            local_pending_batches: self.pending_insert.len(),
            local_inflight_insert_batches: self
                .pending_insert_task
                .as_ref()
                .map(|task| task.batch_count)
                .unwrap_or(0),
            local_ready_processed_batches: self.ready_processed.len(),
            rolling: crate::core::SamplerQueueRollingAverages {
                get_processed_ms: RollingMetricSnapshot::from(&self.metrics.get_processed_ms),
                fetch_completed_ms: RollingMetricSnapshot::from(&self.metrics.fetch_completed_ms),
                insert_bundle_ms: RollingMetricSnapshot::from(&self.metrics.insert_bundle_ms),
                insert_bundle_ms_per_batch: RollingMetricSnapshot::from(
                    &self.metrics.insert_bundle_ms_per_batch,
                ),
                insert_bundle_local_pending_at_start: RollingMetricSnapshot::from(
                    &self.metrics.insert_bundle_local_pending_at_start,
                ),
                flush_ms: RollingMetricSnapshot::from(&self.metrics.flush_ms),
            },
        }
    }

    pub fn last_completed_batch_id(&self) -> Option<i64> {
        self.checkpoint.last_completed_batch_id
    }

    pub async fn queue_counts(&self) -> Result<BatchQueueCounts, StoreError> {
        let counts = self
            .store
            .get_batch_queue_counts(self.run_id, self.last_completed_batch_id())
            .await?;
        Ok(self.queue_counts_with_local_buffer(counts))
    }

    pub async fn open_batch_count(&self) -> Result<i64, StoreError> {
        self.store.get_open_batch_count(self.run_id).await
    }

    async fn reclaim_abandoned_batches(&self) -> Result<u64, StoreError> {
        self.store.reclaim_abandoned_batches(self.run_id).await
    }

    async fn cleanup_consumed_completed_batches(
        &self,
        limit: usize,
    ) -> Result<u64, StoreError> {
        let Some(up_to_batch_id) = self.last_completed_batch_id() else {
            return Ok(0);
        };
        self.store
            .cleanup_consumed_completed_batches(self.run_id, up_to_batch_id, limit)
            .await
    }

    pub async fn tick(&mut self) -> Result<QueueTickResult, StoreError> {
        let completed = self.get_processed().await?;
        let completed_cleanup_duration = self.cleanup_consumed_completed_batches_if_due().await?;
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

    pub async fn force_cleanup_consumed_completed_batches(&mut self) -> Result<Option<Duration>, StoreError> {
        let Some(_) = self.last_completed_batch_id() else {
            return Ok(None);
        };
        let cleanup_started = Instant::now();
        loop {
            let deleted = self
                .cleanup_consumed_completed_batches(COMPLETED_CLEANUP_BATCH_LIMIT)
                .await?;
            if deleted < COMPLETED_CLEANUP_BATCH_LIMIT as u64 {
                break;
            }
        }
        self.last_completed_cleanup_at = Instant::now();
        Ok(Some(cleanup_started.elapsed()))
    }

    pub async fn plan_production(
        &mut self,
        max_producable: Option<usize>,
        queue_counts: BatchQueueCounts,
        batch_size_current: usize,
    ) -> Result<Vec<usize>, StoreError> {
        let active_evaluator_count = self
            .store
            .count_active_evaluator_nodes(self.run_id)
            .await?
            .max(0) as usize;
        self.cached_active_evaluator_count = Some(active_evaluator_count);
        self.cached_tick_queue_counts = Some(queue_counts);
        Ok(self.get_sample(
            max_producable,
            queue_counts,
            active_evaluator_count,
            batch_size_current,
        ))
    }

    pub fn validate_batch_plan(
        &self,
        batch_plan: &[usize],
        max_batch_size: usize,
    ) -> Result<(), StoreError> {
        if batch_plan.len() > self.config.max_batches_per_tick {
            return Err(StoreError::store(format!(
                "batch plan exceeded max_batches_per_tick: planned={} max_batches_per_tick={}",
                batch_plan.len(),
                self.config.max_batches_per_tick
            )));
        }
        if let Some(max_planned_batch_size) = batch_plan.iter().copied().max()
            && max_planned_batch_size > max_batch_size
        {
            return Err(StoreError::store(format!(
                "batch plan exceeded max_batch_size: planned={} max_batch_size={}",
                max_planned_batch_size, max_batch_size
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
        Some(
            ((active_evaluator_count as f64) * self.config.queue_buffer)
                .ceil()
                .max(0.0) as usize,
        )
    }

    pub fn target_local_pending_batches(&self, active_evaluator_count: usize) -> Option<usize> {
        if !self.config.local_pending_buffer_multiplier.is_finite()
            || self.config.local_pending_buffer_multiplier < 0.0
        {
            return None;
        }
        let target_pending_batches = self.target_pending_batches(active_evaluator_count)?;
        Some(
            ((target_pending_batches as f64) * self.config.local_pending_buffer_multiplier)
                .ceil()
                .max(0.0) as usize,
        )
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
                .pending_insert_task
                .as_ref()
                .map(|task| task.batch_count)
                .unwrap_or(0)
    }

    pub(crate) fn local_work_drained(&self) -> bool {
        self.pending_insert.is_empty()
            && self.ready_processed.is_empty()
            && self.pending_insert_task.is_none()
            && self.pending_processed_fetch.is_none()
            && !self.insert_pump_running
    }

    fn observe_insert_bundle_local_pending_at_start(&mut self) {
        self.metrics
            .insert_bundle_local_pending_at_start
            .observe(self.pending_insert.len() as f64);
    }

    pub async fn get_processed(&mut self) -> Result<Vec<CompletedBatch>, StoreError> {
        let started = Instant::now();
        self.drain_finished_insert().await?;
        self.drain_finished_processed_fetch().await?;
        self.ensure_processed_prefetch();

        let ready = self.ready_processed.drain(..).collect::<Vec<_>>();
        observe_duration_ms(&mut self.metrics.get_processed_ms, started.elapsed());
        Ok(ready)
    }

    pub(crate) async fn get_processed_blocking(
        &mut self,
    ) -> Result<Vec<CompletedBatch>, StoreError> {
        let ready = self.get_processed().await?;
        if !ready.is_empty() {
            return Ok(ready);
        }

        if self.pending_processed_fetch.is_none() {
            self.ensure_processed_prefetch();
        }
        let Some(task) = self.pending_processed_fetch.take() else {
            return Ok(Vec::new());
        };
        self.consume_processed_fetch_task(task).await?;
        Ok(self.ready_processed.drain(..).collect::<Vec<_>>())
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

        let Some(target_pending_after_enqueue) =
            self.target_pending_batches(active_evaluator_count)
        else {
            return Vec::new();
        };
        let local_target_pending_after_enqueue = self
            .target_local_pending_batches(active_evaluator_count)
            .unwrap_or(target_pending_after_enqueue);

        let batch_limit = hard_limit
            .min(target_pending_after_enqueue.saturating_sub(pending_before))
            .min(
                local_target_pending_after_enqueue.saturating_sub(self.local_unpersisted_batches()),
            );
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
        let started = Instant::now();
        loop {
            self.drain_finished_insert().await?;
            if self.pending_insert_task.is_none() && self.pending_insert.is_empty() {
                break;
            }
            self.ensure_insert_pump();
            if let Some(task) = self.pending_insert_task.take() {
                self.consume_insert_task(task).await?;
            } else {
                break;
            }
        }
        observe_duration_ms(&mut self.metrics.flush_ms, started.elapsed());
        Ok(())
    }

    pub(crate) fn mark_processed(&mut self, processed: &[CompletedBatch]) {
        if let Some(last) = processed.last() {
            self.checkpoint.last_completed_batch_id = Some(last.batch_id);
        }
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

    async fn cleanup_consumed_completed_batches_if_due(
        &mut self,
    ) -> Result<Option<Duration>, StoreError> {
        let Some(_) = self.last_completed_batch_id() else {
            return Ok(None);
        };
        if self.last_completed_cleanup_at.elapsed() < COMPLETED_CLEANUP_INTERVAL {
            return Ok(None);
        }
        let cleanup_started = Instant::now();
        let _ = self
            .cleanup_consumed_completed_batches(COMPLETED_CLEANUP_BATCH_LIMIT)
            .await?;
        self.last_completed_cleanup_at = Instant::now();
        Ok(Some(cleanup_started.elapsed()))
    }

    fn ensure_processed_prefetch(&mut self) {
        if !self.ready_processed.is_empty() || self.pending_processed_fetch.is_some() {
            return;
        }

        let store = self.store.clone();
        let run_id = self.run_id;
        let fetch_limit = self.config.completed_batch_fetch_limit;
        let strict_batch_ordering = self.config.strict_batch_ordering;
        let after_batch_id = self.checkpoint.last_completed_batch_id;
        self.pending_processed_fetch = Some(PendingProcessedFetchTask {
            started_at: Instant::now(),
            handle: tokio::spawn(async move {
                store
                    .fetch_completed_batches(
                        run_id,
                        fetch_limit,
                        strict_batch_ordering,
                        after_batch_id,
                    )
                    .await
            }),
        });
    }

    fn start_insert_pump_if_idle(&mut self) {
        if self.insert_pump_running || self.pending_insert.is_empty() {
            return;
        }

        self.insert_pump_running = true;
        self.ensure_insert_pump();
    }

    fn ensure_insert_pump(&mut self) {
        if self.pending_insert_task.is_some() || self.pending_insert.is_empty() {
            if self.pending_insert.is_empty() {
                self.insert_pump_running = false;
            }
            return;
        }

        self.observe_insert_bundle_local_pending_at_start();

        let bundle_size = self.config.max_insert_bundle_size.max(1);
        let batch_count = self.pending_insert.len().min(bundle_size);
        let batches = self.pending_insert.drain(..batch_count).collect::<Vec<_>>();
        let store = self.store.clone();
        let run_id = self.run_id;
        let task_id = self.task_id;
        let requires_training_values = self.requires_training_values;
        self.pending_insert_task = Some(PendingInsertTask {
            batch_count,
            started_at: Instant::now(),
            handle: tokio::spawn(async move {
                store
                    .insert_batches(run_id, task_id, requires_training_values, &batches)
                    .await?;
                Ok(())
            }),
        });
    }

    async fn drain_finished_insert(&mut self) -> Result<(), StoreError> {
        let Some(task) = self.pending_insert_task.as_ref() else {
            self.insert_pump_running = false;
            return Ok(());
        };
        if !task.handle.is_finished() {
            return Ok(());
        }

        let task = self
            .pending_insert_task
            .take()
            .expect("checked pending insert task");
        self.consume_insert_task(task).await
    }

    async fn consume_insert_task(&mut self, task: PendingInsertTask) -> Result<(), StoreError> {
        let duration = task.started_at.elapsed();
        let result = match task.handle.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err),
            Err(err) => Err(StoreError::store(format!(
                "sampler queue insert task failed: {err}"
            ))),
        };
        if result.is_ok() {
            observe_duration_ms(&mut self.metrics.insert_bundle_ms, duration);
            if task.batch_count > 0 {
                observe_duration_ms(
                    &mut self.metrics.insert_bundle_ms_per_batch,
                    duration / task.batch_count as u32,
                );
            }
            self.ensure_insert_pump();
        } else {
            self.insert_pump_running = false;
        }
        result
    }

    async fn drain_finished_processed_fetch(&mut self) -> Result<(), StoreError> {
        let Some(task) = self.pending_processed_fetch.as_ref() else {
            return Ok(());
        };
        if !task.handle.is_finished() {
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
        let duration = task.started_at.elapsed();
        let completed = match task.handle.await {
            Ok(Ok(completed)) => completed,
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                return Err(StoreError::store(format!(
                    "sampler queue completed-batch fetch task failed: {err}"
                )));
            }
        };
        observe_duration_ms(&mut self.metrics.fetch_completed_ms, duration);
        self.ready_processed.extend(completed);
        Ok(())
    }
}

fn observe_duration_ms(metric: &mut RollingMetric, duration: Duration) {
    let ms = duration.as_secs_f64() * 1000.0;
    if ms.is_finite() && ms >= 0.0 {
        metric.observe(ms);
    }
}
