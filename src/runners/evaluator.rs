//! Evaluator worker runner orchestration.
#![allow(clippy::too_many_arguments)]

use crate::core::{
    BatchClaim, BatchFailOutcome, BatchTransformConfig, EngineError, EvalError, EvaluatorConfig,
    EvaluatorPerformanceMetrics, EvaluatorPerformanceSnapshot, EvaluatorWorkerStore, StoreError,
};
use crate::evaluation::{BatchResult, EvalBatchOptions, Evaluator, Materializer};
use crate::runners::busy_time::BusyTime;
use crate::runners::process_memory::current_rss_bytes;
use crate::runners::rolling_metric::RollingMetric;
use crate::runners::stage_context::resolve_stage_context;
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    panic::AssertUnwindSafe,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::task::JoinHandle;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluatorRunnerParams {
    pub performance_snapshot_interval_ms: u64,
    pub min_tick_time_ms: u64,
    #[serde(default = "default_evaluator_db_pool_size")]
    pub db_pool_size: u32,
}

fn default_evaluator_db_pool_size() -> u32 {
    2
}

#[derive(Debug, Error)]
pub enum EvaluatorRunnerError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Eval(EvalError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub struct EvaluatorRunner<S> {
    run_id: i32,
    node_name: String,
    evaluator: Box<dyn Evaluator>,
    evaluator_config: EvaluatorConfig,
    domain: Domain,
    params: EvaluatorRunnerParams,
    current_task_id: Option<i64>,
    materializer: Option<Box<dyn Materializer>>,
    prefetch_buffer: LatentPrefetchBuffer<S>,
    submit_buffer: ResultSubmitBuffer<S>,
    active_batch: Option<ActiveBatch>,
    last_claim_reconciliation: Instant,
    draining: bool,
    performance_snapshot_interval: Duration,
    last_snapshot_at: Instant,
    epoch: String,
    cumulative: crate::core::models::EvaluatorCumulativeMetrics,
    batches_completed_total: i64,
    samples_evaluated_total: i64,
    rolling: EvaluatorRollingAverages,
    busy: BusyTime,
    counters: EvaluatorPipelineCounters,
    store: S,
    current_batch_transforms: Vec<Box<dyn crate::evaluation::BatchTransform>>,
    max_batch_retries: i32,
}

struct TaskRuntimeContext {
    evaluator_config: EvaluatorConfig,
    materializer: Box<dyn Materializer>,
    batch_transforms: Vec<Box<dyn crate::evaluation::BatchTransform>>,
}

#[derive(Debug, Clone, Default)]
struct EvaluatorRollingAverages {
    total_ms_per_sample: RollingMetric,
    fetch_ms_per_sample: RollingMetric,
    fetch_stall_ms_per_sample: RollingMetric,
    evaluate_ms_per_sample: RollingMetric,
    materialization_ms_per_sample: RollingMetric,
    submit_ms_per_sample: RollingMetric,
    submit_stall_ms_per_sample: RollingMetric,
}

#[derive(Debug, Clone, Default)]
struct EvaluatorPipelineCounters {
    fetch_attempts: i64,
    fetch_hits: i64,
    fetch_stalls: i64,
    queue_starved_attempts: i64,
    submit_attempts: i64,
    submit_slot_hits: i64,
    submit_stalls: i64,
}

struct PopOutcome {
    claimed: Option<BatchClaim>,
    hit: bool,
    stalled: bool,
    wait_time_ms: f64,
}

struct LatentPrefetchBuffer<S> {
    busy: BusyTime,
    run_id: i32,
    node_uuid: String,
    ready_batch: Option<BatchClaim>,
    // Retained across a failed/ambiguous claim request; retries recover the same row.
    claim_token: Option<String>,
    pending_prefetch: Option<JoinHandle<Result<Option<BatchClaim>, StoreError>>>,
    empty_poll_delay: Duration,
    next_poll: Instant,
    _marker: std::marker::PhantomData<S>,
}

impl<S> Drop for LatentPrefetchBuffer<S> {
    fn drop(&mut self) {
        if let Some(handle) = &self.pending_prefetch {
            handle.abort();
        }
    }
}

impl<S> LatentPrefetchBuffer<S>
where
    S: EvaluatorWorkerStore + Clone + Send + Sync + 'static,
{
    fn new(run_id: i32, node_uuid: String, busy: BusyTime) -> Self {
        Self {
            run_id,
            node_uuid,
            busy,
            ready_batch: None,
            claim_token: None,
            pending_prefetch: None,
            empty_poll_delay: Duration::ZERO,
            next_poll: Instant::now(),
            _marker: std::marker::PhantomData,
        }
    }

    fn has_pending_work(&self) -> bool {
        self.ready_batch.is_some() || self.claim_token.is_some()
    }

    fn waiting_to_poll(&self) -> bool {
        !self.has_pending_work()
            && self.pending_prefetch.is_none()
            && Instant::now() < self.next_poll
    }

    async fn pop(&mut self, store: &S, draining: bool) -> Result<PopOutcome, EvaluatorRunnerError> {
        if self
            .pending_prefetch
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            self.finish_prefetch().await?;
        }
        let hit = self.ready_batch.is_some();
        let started = Instant::now();
        if self.ready_batch.is_none() {
            self.maybe_start_prefetch(store, draining);
            if self.pending_prefetch.is_some() {
                self.finish_prefetch().await?;
            }
        }
        let claimed = self.ready_batch.take();
        if claimed.is_some() {
            self.maybe_start_prefetch(store, draining);
        }
        Ok(PopOutcome {
            claimed,
            hit,
            stalled: !hit,
            wait_time_ms: if hit {
                0.0
            } else {
                started.elapsed().as_secs_f64() * 1000.0
            },
        })
    }

    fn maybe_start_prefetch(&mut self, store: &S, draining: bool) {
        if self.ready_batch.is_some()
            || self.pending_prefetch.is_some()
            || (draining && self.claim_token.is_none())
            || self.waiting_to_poll()
        {
            return;
        }
        let token = self
            .claim_token
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let store = store.clone();
        let run_id = self.run_id;
        let node_uuid = self.node_uuid.clone();
        let busy = self.busy.clone();
        self.pending_prefetch = Some(tokio::spawn(async move {
            let _io = busy.io();
            store.claim_batch(run_id, &node_uuid, &token).await
        }));
    }

    async fn finish_prefetch(&mut self) -> Result<(), EvaluatorRunnerError> {
        let Some(handle) = self.pending_prefetch.as_mut() else {
            return Ok(());
        };
        // Await by reference: cancellation must not detach the only request handle.
        let outcome = handle.await;
        self.pending_prefetch = None;
        match outcome {
            Ok(Ok(claimed)) => {
                if claimed.is_some() {
                    self.empty_poll_delay = Duration::ZERO;
                    self.next_poll = Instant::now();
                } else {
                    // Idle fleets must not hammer the queue. Successful claims
                    // reset the delay; retries of ambiguous claims bypass it.
                    self.empty_poll_delay = (self.empty_poll_delay * 2)
                        .clamp(Duration::from_millis(2), Duration::from_millis(100));
                    self.next_poll = Instant::now()
                        + self.empty_poll_delay.mul_f64(rand::random_range(0.5..=1.0));
                }
                self.ready_batch = claimed;
                self.claim_token = None;
                Ok(())
            }
            Ok(Err(err)) => {
                warn!(run_id=self.run_id, node_uuid=%self.node_uuid,
                    claim_token=?self.claim_token, operation="claim_batch", error=%err,
                    "claim request failed; retaining token for retry");
                Err(err.into())
            }
            Err(err) => {
                Err(StoreError::store(format!("evaluator prefetch task failed: {err}")).into())
            }
        }
    }
}

struct ActiveBatch {
    claim: BatchClaim,
    fetch_time_ms: f64,
    fetch_stall_time_ms: f64,
    evaluation: Option<Result<EvaluatedBatch, FailedEvaluation>>,
}

struct EvaluatedBatch {
    result: BatchResult,
    total_time_ms: f64,
    materialization_time_ms: f64,
    eval_time_ms: f64,
    processed_samples: usize,
}

struct FailedEvaluation {
    message: String,
    compute_time_ms: f64,
    outcome: Option<BatchFailOutcome>,
}

struct Submission {
    batch_id: i64,
    task_id: i64,
    claim_token: String,
    result: BatchResult,
    outcome: SubmitOutcome,
}

#[derive(Clone)]
struct SubmitOutcome {
    processed_samples: usize,
    total_time_ms: f64,
    fetch_time_ms: f64,
    fetch_stall_time_ms: f64,
    materialization_time_ms: f64,
    eval_time_ms: f64,
    submit_time_ms: f64,
    submit_stall_time_ms: f64,
}

struct ResultSubmitBuffer<S> {
    busy: BusyTime,
    node_uuid: String,
    store: S,
    submission: Option<Arc<Submission>>,
    pending_submit: Option<JoinHandle<Result<SubmitOutcome, StoreError>>>,
}

impl<S> Drop for ResultSubmitBuffer<S> {
    fn drop(&mut self) {
        if let Some(handle) = &self.pending_submit {
            handle.abort();
        }
    }
}

impl<S> ResultSubmitBuffer<S>
where
    S: EvaluatorWorkerStore + Clone + Send + Sync + 'static,
{
    fn new(store: S, node_uuid: String, busy: BusyTime) -> Self {
        Self {
            node_uuid,
            store,
            busy,
            submission: None,
            pending_submit: None,
        }
    }

    fn is_idle(&self) -> bool {
        self.submission.is_none()
    }

    fn start_submit(&mut self, submission: Submission) {
        assert!(self.is_idle());
        self.submission = Some(Arc::new(submission));
        self.ensure_submit();
    }

    fn ensure_submit(&mut self) {
        if self.pending_submit.is_some() {
            return;
        }
        let Some(submission) = self.submission.clone() else {
            return;
        };
        let store = self.store.clone();
        let node_uuid = self.node_uuid.clone();
        let busy = self.busy.clone();
        self.pending_submit = Some(tokio::spawn(async move {
            let _io = busy.io();
            let started = Instant::now();
            store
                .submit_batch_results(
                    submission.batch_id,
                    &node_uuid,
                    &submission.claim_token,
                    &submission.result,
                    submission.outcome.total_time_ms,
                )
                .await?;
            let mut outcome = submission.outcome.clone();
            outcome.submit_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            Ok(outcome)
        }));
    }

    async fn drain_finished(
        &mut self,
        run_id: i32,
        node_name: &str,
    ) -> Result<Option<SubmitOutcome>, EvaluatorRunnerError> {
        self.ensure_submit();
        if !self
            .pending_submit
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            return Ok(None);
        }
        self.wait_for_slot(run_id, node_name).await
    }

    async fn wait_for_slot(
        &mut self,
        run_id: i32,
        node_name: &str,
    ) -> Result<Option<SubmitOutcome>, EvaluatorRunnerError> {
        self.ensure_submit();
        let Some(handle) = self.pending_submit.as_mut() else {
            return Ok(None);
        };
        let result = handle.await;
        self.pending_submit = None;
        match result {
            Ok(Ok(outcome)) => {
                self.submission = None;
                Ok(Some(outcome))
            }
            Ok(Err(err)) if err.is_batch_ownership_lost() => {
                info!(run_id, node_name, node_uuid=%self.node_uuid, error=%err,
                    "dropping stale evaluator result after batch ownership was lost");
                self.submission = None;
                Ok(None)
            }
            Ok(Err(err)) => {
                let submission = self.submission.as_ref().expect("pending submission");
                warn!(run_id, node_name, node_uuid=%self.node_uuid,
                    batch_id=submission.batch_id, task_id=submission.task_id,
                    claim_token=%submission.claim_token, operation="submit_batch_results", error=%err,
                    "result submission failed; retaining result for retry");
                Err(err.into())
            }
            Err(err) => {
                Err(StoreError::store(format!("evaluator submit task failed: {err}")).into())
            }
        }
    }
}

impl<S> EvaluatorRunner<S>
where
    S: EvaluatorWorkerStore + Clone + Send + Sync + 'static,
{
    fn panic_message(payload: Box<dyn Any + Send>) -> String {
        if let Some(message) = payload.downcast_ref::<&str>() {
            return (*message).to_string();
        }
        if let Some(message) = payload.downcast_ref::<String>() {
            return message.clone();
        }
        "unknown panic payload".to_string()
    }

    fn call_with_panic_guard<T, E>(
        label: &str,
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, EvaluatorRunnerError>
    where
        E: Into<EvaluatorRunnerError>,
    {
        match std::panic::catch_unwind(AssertUnwindSafe(action)) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err.into()),
            Err(payload) => Err(EvaluatorRunnerError::Engine(EngineError::engine(format!(
                "{label} panicked: {}",
                Self::panic_message(payload)
            )))),
        }
    }

    pub fn new(
        store: S,
        run_id: i32,
        node_name: impl Into<String>,
        node_uuid: impl Into<String>,
        evaluator_config: EvaluatorConfig,
        evaluator: Box<dyn Evaluator>,
        domain: Domain,
        params: EvaluatorRunnerParams,
        max_batch_retries: i32,
    ) -> Self {
        let node_name = node_name.into();
        let node_uuid = node_uuid.into();
        let busy = BusyTime::default();
        let performance_snapshot_interval =
            Duration::from_millis(params.performance_snapshot_interval_ms);
        Self {
            run_id,
            node_name,
            evaluator,
            evaluator_config,
            domain,
            params,
            current_task_id: None,
            materializer: None,
            prefetch_buffer: LatentPrefetchBuffer::new(run_id, node_uuid.clone(), busy.clone()),
            submit_buffer: ResultSubmitBuffer::new(store.clone(), node_uuid, busy.clone()),
            active_batch: None,
            last_claim_reconciliation: Instant::now() - Duration::from_secs(1),
            draining: false,
            performance_snapshot_interval,
            last_snapshot_at: Instant::now(),
            epoch: uuid::Uuid::new_v4().to_string(),
            cumulative: Default::default(),
            batches_completed_total: 0,
            samples_evaluated_total: 0,
            rolling: EvaluatorRollingAverages::default(),
            busy,
            counters: EvaluatorPipelineCounters::default(),
            store,
            current_batch_transforms: Vec::new(),
            max_batch_retries,
        }
    }

    pub fn params(&self) -> &EvaluatorRunnerParams {
        &self.params
    }

    fn build_batch_transforms(
        configs: &[BatchTransformConfig],
        domain: &Domain,
    ) -> Result<Vec<Box<dyn crate::evaluation::BatchTransform>>, EvaluatorRunnerError> {
        configs
            .iter()
            .map(|config| {
                let transform = config.build().map_err(|err| {
                    EvaluatorRunnerError::Store(StoreError::store(format!(
                        "failed to build batch transform: {err}"
                    )))
                })?;
                transform.validate_domain(domain).map_err(|err| {
                    EvaluatorRunnerError::Store(StoreError::store(format!(
                        "failed to validate batch transform domain: {err}"
                    )))
                })?;
                Ok(transform)
            })
            .collect()
    }

    async fn ensure_task_context(&mut self, task_id: i64) -> Result<(), EvaluatorRunnerError> {
        if self.current_task_id == Some(task_id) {
            return Ok(());
        }

        let TaskRuntimeContext {
            evaluator_config,
            materializer,
            batch_transforms,
        } = self.load_task_context(task_id).await?;

        if evaluator_config != self.evaluator_config {
            let evaluator = evaluator_config.build().map_err(|err| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "failed to build evaluator for task {}: {err}",
                    task_id
                )))
            })?;
            self.evaluator = evaluator;
            self.evaluator_config = evaluator_config;
        }

        self.current_task_id = Some(task_id);
        self.materializer = Some(materializer);
        self.current_batch_transforms = batch_transforms;
        Ok(())
    }

    async fn load_task_context(
        &self,
        task_id: i64,
    ) -> Result<TaskRuntimeContext, EvaluatorRunnerError> {
        let io = self.busy.io();
        let task = self
            .store
            .load_run_task(task_id)
            .await
            .map_err(EvaluatorRunnerError::Store)?
            .ok_or_else(|| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "claimed batch references missing task {}",
                    task_id
                )))
            })?;
        let resolved =
            resolve_stage_context(&self.store, self.run_id, &task, task.sequence_nr, None)
                .await
                .map_err(EvaluatorRunnerError::Store)?;
        drop(io);
        let batch_transforms =
            Self::build_batch_transforms(&resolved.batch_transforms, &self.domain)?;
        let evaluator_domain = resolved.evaluator_config.resolve_domain().map_err(|err| {
            EvaluatorRunnerError::Store(StoreError::store(format!(
                "failed to resolve evaluator domain for task {}: {err}",
                task_id
            )))
        })?;
        if evaluator_domain != self.domain {
            return Err(EvaluatorRunnerError::Store(StoreError::store(format!(
                "task {} evaluator domain {:?} does not match run domain {:?}",
                task_id, evaluator_domain, self.domain
            ))));
        }
        let materializer = resolved
            .sampler_config
            .build_materializer(
                &self.domain,
                resolved.handoff.as_ref().map(|handoff| handoff.as_ref()),
            )
            .map_err(|err| {
                EvaluatorRunnerError::Store(StoreError::store(format!(
                    "failed to build materializer for task {}: {err}",
                    task_id
                )))
            })?;
        materializer.validate_domain(&self.domain).map_err(|err| {
            EvaluatorRunnerError::Store(StoreError::store(format!(
                "failed to validate materializer domain for task {}: {err}",
                task_id
            )))
        })?;
        Ok(TaskRuntimeContext {
            evaluator_config: resolved.evaluator_config,
            materializer,
            batch_transforms,
        })
    }

    async fn reconcile_claims(&mut self) -> Result<(), EvaluatorRunnerError> {
        if self.last_claim_reconciliation.elapsed() < Duration::from_secs(1) {
            return Ok(());
        }
        let mut tokens = Vec::new();
        if let Some(token) = &self.prefetch_buffer.claim_token {
            tokens.push(token.clone());
        }
        if let Some(batch) = &self.prefetch_buffer.ready_batch {
            tokens.push(batch.claim_token.clone());
        }
        if let Some(batch) = &self.active_batch {
            tokens.push(batch.claim.claim_token.clone());
        }
        if let Some(submission) = &self.submit_buffer.submission {
            tokens.push(submission.claim_token.clone());
        }
        let _io = self.busy.io();
        let reclaimed = self
            .store
            .release_untracked_claims(self.run_id, &self.prefetch_buffer.node_uuid, &tokens)
            .await?;
        self.last_claim_reconciliation = Instant::now();
        if reclaimed > 0 {
            warn!(run_id=self.run_id, node_name=%self.node_name,
                node_uuid=%self.prefetch_buffer.node_uuid, reclaimed,
                "released claims no longer tracked by this evaluator");
        }
        Ok(())
    }

    async fn persist_batch_failure(&mut self) -> Result<(), EvaluatorRunnerError> {
        let io = self.busy.io();
        let batch = self.active_batch.as_ref().expect("active failed batch");
        let Some(Err(failure)) = &batch.evaluation else {
            unreachable!()
        };
        let batch_id = batch.claim.batch_id;
        let task_id = batch.claim.task_id;
        let token = batch.claim.claim_token.clone();
        let message = failure.message.clone();
        let compute_time_ms = failure.compute_time_ms;
        let outcome = if let Some(outcome) = failure.outcome {
            outcome
        } else {
            let outcome = match self
                .store
                .fail_batch(
                    batch_id,
                    &self.prefetch_buffer.node_uuid,
                    &token,
                    &message,
                    self.max_batch_retries,
                )
                .await
            {
                Ok(outcome) => outcome,
                Err(err) if err.is_batch_ownership_lost() => {
                    info!(run_id=self.run_id, node_name=%self.node_name, batch_id, task_id,
                        "discarding failure for a revoked batch claim");
                    self.active_batch = None;
                    return Ok(());
                }
                Err(err) => {
                    warn!(run_id=self.run_id, node_name=%self.node_name, batch_id, task_id,
                        claim_token=%token, operation="fail_batch", error=%err,
                        "batch failure notification failed; retaining it for retry");
                    return Err(err.into());
                }
            };
            if let Some(Err(failure)) = &mut self.active_batch.as_mut().unwrap().evaluation {
                failure.outcome = Some(outcome);
            }
            outcome
        };
        if let BatchFailOutcome::PermanentlyFailed {
            task_id,
            retry_count,
        } = outcome
        {
            let reason = format!(
                "batch {batch_id} failed after {retry_count}/{} retries: {message}",
                self.max_batch_retries
            );
            self.store.fail_run_task(task_id, &reason).await?;
            self.store
                .clear_desired_assignments_for_run(self.run_id)
                .await?;
        }
        warn!(run_id=self.run_id, node_name=%self.node_name, batch_id, task_id,
            compute_time_ms, error=%message, ?outcome, "evaluator batch failed");
        self.active_batch = None;
        drop(io);
        self.flush_performance_snapshot_if_due(false).await
    }

    pub async fn tick(&mut self) -> Result<(), EvaluatorRunnerError> {
        crate::runners::activity::set("waiting");
        self.reconcile_claims().await?;
        self.consume_finished_submit().await?;

        if self.active_batch.is_none() {
            if self.prefetch_buffer.waiting_to_poll() {
                self.flush_performance_snapshot_if_due(false).await?;
                return Ok(());
            }
            self.counters.fetch_attempts += 1;
            let started = Instant::now();
            let pop = self.prefetch_buffer.pop(&self.store, self.draining).await?;
            let Some(claim) = pop.claimed else {
                self.counters.queue_starved_attempts += 1;
                self.flush_performance_snapshot_if_due(false).await?;
                return Ok(());
            };
            if pop.hit {
                self.counters.fetch_hits += 1;
            }
            if pop.stalled {
                self.counters.fetch_stalls += 1;
            }
            self.active_batch = Some(ActiveBatch {
                claim,
                fetch_time_ms: started.elapsed().as_secs_f64() * 1000.0,
                fetch_stall_time_ms: pop.wait_time_ms,
                evaluation: None,
            });
        }
        let active = self.active_batch.as_ref().unwrap();
        if active.evaluation.is_none() {
            let task_id = active.claim.task_id;
            let batch_id = active.claim.batch_id;
            crate::runners::activity::context(self.run_id, task_id);
            crate::runners::activity::set("initializing runtime");
            if let Err(err) = self.ensure_task_context(task_id).await {
                warn!(run_id=self.run_id, node_name=%self.node_name, batch_id, task_id,
                    operation="ensure_task_context", error=%err,
                    "task context loading failed; retaining claimed batch");
                return Err(err);
            }
            // Synchronous computation has no cancellation point. Put the outcome
            // back into runner state before any fallible persistence awaits.
            let mut active = self.active_batch.take().unwrap();
            let outcome = self.evaluate_claim(&active.claim);
            active.evaluation = Some(outcome);
            self.active_batch = Some(active);
        }
        if self
            .active_batch
            .as_ref()
            .unwrap()
            .evaluation
            .as_ref()
            .unwrap()
            .is_err()
        {
            return self.persist_batch_failure().await;
        }

        // A failure submitting the previous batch must not discard the result
        // just evaluated. Both payloads stay owned until their acknowledgement.
        let mut submit_stall_time_ms = 0.0;
        if !self.submit_buffer.is_idle() {
            self.counters.submit_stalls += 1;
            let started = Instant::now();
            let outcome = self
                .submit_buffer
                .wait_for_slot(self.run_id, &self.node_name)
                .await?;
            submit_stall_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            self.consume_submitted_result(outcome).await?;
        } else {
            self.counters.submit_slot_hits += 1;
        }
        let active = self.active_batch.take().unwrap();
        let Some(Ok(evaluated)) = active.evaluation else {
            unreachable!()
        };
        self.counters.submit_attempts += 1;
        self.submit_buffer.start_submit(Submission {
            batch_id: active.claim.batch_id,
            task_id: active.claim.task_id,
            claim_token: active.claim.claim_token,
            result: evaluated.result,
            outcome: SubmitOutcome {
                processed_samples: evaluated.processed_samples,
                total_time_ms: evaluated.total_time_ms,
                fetch_time_ms: active.fetch_time_ms,
                fetch_stall_time_ms: active.fetch_stall_time_ms,
                materialization_time_ms: evaluated.materialization_time_ms,
                eval_time_ms: evaluated.eval_time_ms,
                submit_time_ms: 0.0,
                submit_stall_time_ms,
            },
        });
        Ok(())
    }

    fn evaluate_claim(&mut self, claimed: &BatchClaim) -> Result<EvaluatedBatch, FailedEvaluation> {
        let _compute = self.busy.compute();
        let compute_started = Instant::now();
        let result = (|| -> Result<EvaluatedBatch, EvaluatorRunnerError> {
            crate::runners::activity::set("materializing");
            let materializer = self.materializer.as_mut().ok_or_else(|| {
                StoreError::store(format!(
                    "evaluator task {} has no materializer",
                    claimed.task_id
                ))
            })?;
            let mut batch = Self::call_with_panic_guard("materializer.materialize_batch", || {
                materializer
                    .materialize_batch(&claimed.latent_batch)
                    .map_err(EvaluatorRunnerError::Engine)
            })?;
            for transform in &self.current_batch_transforms {
                batch = Self::call_with_panic_guard("batch_transform.apply", || {
                    transform.apply(batch).map_err(EvaluatorRunnerError::Engine)
                })?;
            }
            self.domain.validate_batch(&batch).map_err(|err| {
                EngineError::engine(format!(
                    "materialized batch does not match run domain: {err}"
                ))
            })?;
            let materialization_time_ms = compute_started.elapsed().as_secs_f64() * 1000.0;
            crate::runners::activity::set("evaluating");
            let started = Instant::now();
            let result = Self::call_with_panic_guard("evaluator.eval_batch", || {
                self.evaluator
                    .eval_batch(
                        &batch,
                        &claimed.latent_batch.accumulator,
                        EvalBatchOptions {
                            require_training_values: claimed.requires_training_values,
                        },
                    )
                    .map_err(EvaluatorRunnerError::Eval)
            })?;
            let eval_time_ms = started.elapsed().as_secs_f64() * 1000.0;
            if claimed.requires_training_values && result.values.is_none() {
                return Err(EngineError::engine(format!(
                    "result is missing training values for training batch {}",
                    claimed.batch_id
                ))
                .into());
            }
            if !result.matches_batch(&batch) {
                return Err(EngineError::engine(format!(
                    "result length mismatch for batch {}: expected {}, got {}",
                    claimed.batch_id,
                    batch.size(),
                    result.len()
                ))
                .into());
            }
            Ok(EvaluatedBatch {
                result,
                processed_samples: batch.size(),
                eval_time_ms,
                materialization_time_ms,
                total_time_ms: materialization_time_ms + eval_time_ms,
            })
        })();
        result.map_err(|err| FailedEvaluation {
            message: err.to_string(),
            compute_time_ms: compute_started.elapsed().as_secs_f64() * 1000.0,
            outcome: None,
        })
    }

    async fn consume_finished_submit(&mut self) -> Result<(), EvaluatorRunnerError> {
        let outcome = self
            .submit_buffer
            .drain_finished(self.run_id, &self.node_name)
            .await?;
        self.consume_submitted_result(outcome).await
    }

    async fn consume_submitted_result(
        &mut self,
        outcome: Option<SubmitOutcome>,
    ) -> Result<(), EvaluatorRunnerError> {
        let Some(outcome) = outcome else {
            return Ok(());
        };

        self.observe_eval_batch(
            outcome.processed_samples,
            outcome.total_time_ms,
            outcome.fetch_time_ms,
            outcome.fetch_stall_time_ms,
            outcome.materialization_time_ms,
            outcome.eval_time_ms,
            outcome.submit_time_ms,
            outcome.submit_stall_time_ms,
        );
        self.flush_performance_snapshot_if_due(false).await?;
        Ok(())
    }

    fn observe_eval_batch(
        &mut self,
        samples: usize,
        total_time_ms: f64,
        fetch_time_ms: f64,
        fetch_stall_time_ms: f64,
        materialization_time_ms: f64,
        eval_time_ms: f64,
        submit_time_ms: f64,
        submit_stall_time_ms: f64,
    ) {
        self.cumulative.evaluate_seconds += eval_time_ms / 1000.0;
        self.cumulative.materialize_seconds += materialization_time_ms / 1000.0;
        self.cumulative.fetch_wait_seconds += fetch_stall_time_ms / 1000.0;
        self.cumulative.submit_seconds += submit_time_ms / 1000.0;
        self.cumulative.submit_wait_seconds += submit_stall_time_ms / 1000.0;
        self.batches_completed_total += 1;
        crate::runners::activity::completed_batch();
        crate::runners::activity::set("waiting");
        self.samples_evaluated_total += samples as i64;
        self.rolling
            .total_ms_per_sample
            .observe_batch(total_time_ms, samples);
        self.rolling
            .fetch_ms_per_sample
            .observe_batch(fetch_time_ms, samples);
        self.rolling
            .fetch_stall_ms_per_sample
            .observe_batch(fetch_stall_time_ms, samples);
        self.rolling
            .materialization_ms_per_sample
            .observe_batch(materialization_time_ms, samples);
        self.rolling
            .evaluate_ms_per_sample
            .observe_batch(eval_time_ms, samples);
        self.rolling
            .submit_ms_per_sample
            .observe_batch(submit_time_ms, samples);
        self.rolling
            .submit_stall_ms_per_sample
            .observe_batch(submit_stall_time_ms, samples);
    }

    async fn flush_performance_snapshot_if_due(
        &mut self,
        force: bool,
    ) -> Result<(), EvaluatorRunnerError> {
        let due = if self.performance_snapshot_interval.is_zero() {
            true
        } else {
            self.last_snapshot_at.elapsed() >= self.performance_snapshot_interval
        };
        if !force && !due {
            return Ok(());
        }

        let progress = self
            .store
            .load_run_sample_progress(self.run_id)
            .await
            .map_err(EvaluatorRunnerError::Store)?;
        let completed_samples_total = progress
            .as_ref()
            .map(|progress| progress.nr_completed_samples)
            .unwrap_or(self.samples_evaluated_total);
        // Idle workers are observable before winning a batch, including after
        // a task transition. An in-flight batch retains its own task identity.
        let task_id = self
            .active_batch
            .as_ref()
            .map(|batch| batch.claim.task_id)
            .or_else(|| {
                progress
                    .as_ref()
                    .and_then(|progress| progress.active_task_id)
            });

        let snapshot = EvaluatorPerformanceSnapshot {
            run_id: self.run_id,
            node_name: self.node_name.clone(),
            metrics: EvaluatorPerformanceMetrics {
                epoch: Some(self.epoch.clone()),
                node_uuid: Some(self.prefetch_buffer.node_uuid.clone()),
                task_id: task_id.map(|id| id.to_string()),
                cumulative: Some(self.cumulative.clone()),
                busy: Some(self.busy.snapshot()),
                engine_diagnostics: self.evaluator.diagnostics(),
                batches_completed: self.batches_completed_total,
                samples_evaluated: self.samples_evaluated_total,
                avg_time_per_sample_ms: self.rolling.total_ms_per_sample.value().unwrap_or(0.0),
                std_time_per_sample_ms: self.rolling.total_ms_per_sample.std_dev(),
                avg_fetch_time_per_sample_ms: self
                    .rolling
                    .fetch_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_fetch_time_per_sample_ms: self.rolling.fetch_ms_per_sample.std_dev(),
                avg_fetch_stall_time_per_sample_ms: self
                    .rolling
                    .fetch_stall_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_fetch_stall_time_per_sample_ms: self
                    .rolling
                    .fetch_stall_ms_per_sample
                    .std_dev(),
                prefetch_hit_ratio: ratio(self.counters.fetch_hits, self.counters.fetch_attempts),
                fetch_stall_ratio: ratio(self.counters.fetch_stalls, self.counters.fetch_attempts),
                queue_starvation_ratio: ratio(
                    self.counters.queue_starved_attempts,
                    self.counters.fetch_attempts,
                ),
                avg_evaluate_time_per_sample_ms: self
                    .rolling
                    .evaluate_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_evaluate_time_per_sample_ms: self.rolling.evaluate_ms_per_sample.std_dev(),
                avg_materialization_time_per_sample_ms: self
                    .rolling
                    .materialization_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_materialization_time_per_sample_ms: self
                    .rolling
                    .materialization_ms_per_sample
                    .std_dev(),
                avg_submit_time_per_sample_ms: self
                    .rolling
                    .submit_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_submit_time_per_sample_ms: self.rolling.submit_ms_per_sample.std_dev(),
                avg_submit_stall_time_per_sample_ms: self
                    .rolling
                    .submit_stall_ms_per_sample
                    .value()
                    .unwrap_or(0.0),
                std_submit_stall_time_per_sample_ms: self
                    .rolling
                    .submit_stall_ms_per_sample
                    .std_dev(),
                submit_slot_hit_ratio: ratio(
                    self.counters.submit_slot_hits,
                    self.counters.submit_attempts,
                ),
                submit_stall_ratio: ratio(
                    self.counters.submit_stalls,
                    self.counters.submit_attempts,
                ),
                completed_samples_total,
            },
            rss_bytes: current_rss_bytes(),
        };

        self.store
            .record_evaluator_performance_snapshot(&snapshot)
            .await
            .map_err(EvaluatorRunnerError::Store)?;

        self.last_snapshot_at = Instant::now();
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), EvaluatorRunnerError> {
        self.draining = true;
        while self.active_batch.is_some() || self.prefetch_buffer.has_pending_work() {
            self.tick().await?;
        }
        let outcome = self
            .submit_buffer
            .wait_for_slot(self.run_id, &self.node_name)
            .await?;
        self.consume_submitted_result(outcome).await?;
        self.flush_performance_snapshot_if_due(true).await?;
        Ok(())
    }
}

fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
