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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SamplerQueueConfig {
    pub queue_buffer: f64,
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
    pending_insert_drain_active: bool,
    pending_processed_fetch: Option<PendingProcessedFetchTask>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct QueueMetricsState {
    get_processed_ms: RollingMetric,
    fetch_completed_ms: RollingMetric,
    insert_batches_ms: RollingMetric,
    insert_batches_ms_per_batch: RollingMetric,
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
            pending_insert_drain_active: false,
            pending_processed_fetch: None,
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
                insert_batches_ms: RollingMetricSnapshot::from(&self.metrics.insert_batches_ms),
                insert_batches_ms_per_batch: RollingMetricSnapshot::from(
                    &self.metrics.insert_batches_ms_per_batch,
                ),
                flush_ms: RollingMetricSnapshot::from(&self.metrics.flush_ms),
            },
        }
    }

    pub fn last_completed_batch_id(&self) -> Option<i64> {
        self.checkpoint.last_completed_batch_id
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

    pub fn ingest(&mut self, batches: Vec<LatentBatch>) {
        self.pending_insert.extend(batches);
        self.start_insert_if_idle();
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
    }

    pub async fn get_processed(&mut self) -> Result<Vec<CompletedBatch>, StoreError> {
        let started = Instant::now();
        self.drain_finished_insert().await?;
        self.drain_finished_processed_fetch().await?;
        self.start_insert_if_idle();
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

        let batch_limit =
            hard_limit.min(target_pending_after_enqueue.saturating_sub(pending_before));
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
            self.start_insert_if_idle();
            if self.pending_insert_task.is_none() && self.pending_insert.is_empty() {
                break;
            }
            if let Some(task) = self.pending_insert_task.take() {
                self.consume_insert_task(task).await?;
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

    fn start_insert_if_idle(&mut self) {
        if self.pending_insert_task.is_some()
            || self.pending_insert.is_empty()
            || self.pending_insert_drain_active
        {
            return;
        }

        self.pending_insert_drain_active = true;
        self.spawn_next_insert_bundle();
    }

    fn spawn_next_insert_bundle(&mut self) {
        if self.pending_insert_task.is_some() || self.pending_insert.is_empty() {
            self.pending_insert_drain_active = false;
            return;
        }

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
            observe_duration_ms(&mut self.metrics.insert_batches_ms, duration);
            if task.batch_count > 0 {
                observe_duration_ms(
                    &mut self.metrics.insert_batches_ms_per_batch,
                    duration / task.batch_count as u32,
                );
            }
            self.spawn_next_insert_bundle();
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
