//! Node-local worker orchestration and role reconciliation.
//!
//! A `run_node` process is role-agnostic. Desired role/run comes from DB.
//! The supervisor loop polls desired assignment and starts/stops one worker task.

use super::{
    evaluator::EvaluatorRunner, sampler_aggregator::RunnerConfig,
    sampler_aggregator::SamplerAggregatorRunner,
};
use crate::core::{
    AggregationStore, AssignmentLeaseStore, ControlPlaneStore, RunSpecStore, StoreError,
    WorkQueueStore, Worker as WorkerRecord, WorkerRegistryStore, WorkerRole, WorkerStatus,
};
use crate::engines::observable::ObservableFactory;
use crate::engines::{
    Evaluator, EvaluatorEngine, ObservableEngine, SamplerAggregator, SamplerAggregatorEngine,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use std::time::Duration;
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct NodeRunnerConfig {
    pub poll_interval: Duration,
}

pub trait NodeRunnerStore:
    RunSpecStore
    + ControlPlaneStore
    + WorkerRegistryStore
    + AssignmentLeaseStore
    + WorkQueueStore
    + AggregationStore
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> NodeRunnerStore for T where
    T: RunSpecStore
        + ControlPlaneStore
        + WorkerRegistryStore
        + AssignmentLeaseStore
        + WorkQueueStore
        + AggregationStore
        + Clone
        + Send
        + Sync
        + 'static
{
}

impl Default for NodeRunnerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(1_000),
        }
    }
}

fn role_worker_id(node_id: &str, role: WorkerRole) -> String {
    format!("{node_id}-{role}")
}

const ROLE_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoleTarget {
    role: WorkerRole,
    run_id: i32,
}

struct ActiveWorker<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    worker_id: String,
    role: WorkerRole,
    run_id: i32,
}

impl<S: NodeRunnerStore> ActiveWorker<S> {
    fn new(
        store: S,
        node_id: impl Into<String>,
        worker_id: impl Into<String>,
        role: WorkerRole,
        run_id: i32,
    ) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            worker_id: worker_id.into(),
            role,
            run_id,
        }
    }

    async fn run(self, stop_rx: watch::Receiver<bool>) -> Result<(), StoreError> {
        match self.role {
            WorkerRole::Evaluator => self.run_evaluator(stop_rx).await,
            WorkerRole::SamplerAggregator => self.run_sampler_aggregator(stop_rx).await,
        }
    }

    async fn run_evaluator(self, mut stop_rx: watch::Receiver<bool>) -> Result<(), StoreError> {
        let Some(spec) = self.store.load_run_spec(self.run_id).await? else {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::Evaluator,
                event_type = "run_spec_missing",
                "run has no RunSpec; evaluator not started"
            );
            return Ok(());
        };

        let evaluator =
            EvaluatorEngine::build(spec.evaluator_implementation, &spec.evaluator_params)
                .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?;
        evaluator
            .validate_point_spec(&spec.point_spec)
            .map_err(|err| {
                StoreError::store(format!(
                    "incompatible evaluator for point_spec on run {}: {}",
                    self.run_id, err
                ))
            })?;

        let observable =
            ObservableEngine::build(spec.observable_implementation, &spec.observable_params)
                .map_err(|err| StoreError::store(format!("failed to build observable: {err}")))?;

        if !evaluator.supports_observable(&observable) {
            return Err(StoreError::store(format!(
                "incompatible evaluator/observable pair for run {}: evaluator={} observable={}",
                self.run_id, spec.evaluator_implementation, spec.observable_implementation
            )));
        }

        self.register_active_worker(spec.evaluator_implementation.as_ref())
            .await?;
        self.store
            .assign_evaluator(self.run_id, &self.worker_id)
            .await?;

        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "worker_started",
            "evaluator worker started"
        );

        let observable_factory = ObservableFactory::new(
            spec.observable_implementation,
            spec.observable_params.clone(),
        );
        let mut runner = EvaluatorRunner::new(
            self.run_id,
            self.worker_id.clone(),
            Box::new(evaluator),
            observable_factory,
            spec.point_spec.clone(),
            Duration::from_millis(
                spec.evaluator_runner_params
                    .performance_snapshot_interval_ms,
            ),
            self.store.clone(),
        );

        let idle_backoff = Duration::from_millis(spec.evaluator_runner_params.min_loop_time_ms);

        loop {
            if *stop_rx.borrow() {
                break;
            }

            self.heartbeat_with_log().await;

            let sleep_after = match runner.tick().await {
                Ok(tick) => {
                    if tick.processed_samples > 0 {
                        Duration::ZERO
                    } else {
                        idle_backoff
                    }
                }
                Err(err) => {
                    warn!(
                        target: "worker_log",
                        run_id = self.run_id,
                        node_id = %self.node_id,
                        worker_id = %self.worker_id,
                        role = %WorkerRole::Evaluator,
                        event_type = "tick_failed",
                        error = %err,
                        "evaluator tick failed"
                    );
                    idle_backoff
                }
            };

            if sleep_after > Duration::ZERO {
                tokio::select! {
                    _ = stop_rx.changed() => {}
                    _ = sleep(sleep_after) => {}
                }
            }
        }

        if let Err(err) = self
            .store
            .unassign_evaluator(self.run_id, &self.worker_id)
            .await
        {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::Evaluator,
                event_type = "unassign_failed",
                error = %err,
                "failed to unassign evaluator"
            );
        }

        self.mark_inactive_with_log().await;
        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "worker_stopped",
            "evaluator worker stopped"
        );

        Ok(())
    }

    async fn run_sampler_aggregator(
        self,
        mut stop_rx: watch::Receiver<bool>,
    ) -> Result<(), StoreError> {
        let Some(spec) = self.store.load_run_spec(self.run_id).await? else {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::SamplerAggregator,
                event_type = "run_spec_missing",
                "run has no RunSpec; sampler-aggregator not started"
            );
            return Ok(());
        };

        let engine = SamplerAggregatorEngine::build(
            spec.sampler_aggregator_implementation,
            &spec.sampler_aggregator_params,
        )
        .map_err(|err| StoreError::store(format!("failed to build sampler-aggregator: {err}")))?;
        engine
            .validate_point_spec(&spec.point_spec)
            .map_err(|err| {
                StoreError::store(format!(
                    "incompatible sampler-aggregator for point_spec on run {}: {}",
                    self.run_id, err
                ))
            })?;

        let observable_factory =
            ObservableFactory::new(spec.observable_implementation, spec.observable_params);

        self.register_active_worker(spec.sampler_aggregator_implementation.as_ref())
            .await?;

        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "worker_started",
            "sampler-aggregator worker started"
        );

        let mut runner = SamplerAggregatorRunner::new(
            self.run_id,
            self.worker_id.clone(),
            Box::new(engine),
            observable_factory,
            self.store.clone(),
            self.store.clone(),
            RunnerConfig {
                nr_samples: spec.sampler_aggregator_runner_params.nr_samples,
                performance_snapshot_interval_ms: spec
                    .sampler_aggregator_runner_params
                    .performance_snapshot_interval_ms,
                max_batches_per_tick: spec.sampler_aggregator_runner_params.max_batches_per_tick,
                max_pending_batches: spec.sampler_aggregator_runner_params.max_pending_batches,
                completed_batch_fetch_limit: spec
                    .sampler_aggregator_runner_params
                    .completed_batch_fetch_limit,
            },
            spec.point_spec.clone(),
        )
        .await
        .map_err(|err| StoreError::store(err.to_string()))?;

        let lease_ttl = Duration::from_millis(spec.sampler_aggregator_runner_params.lease_ttl_ms);
        let interval = Duration::from_millis(spec.sampler_aggregator_runner_params.interval_ms);
        let mut owns_lease = false;

        loop {
            if *stop_rx.borrow() {
                break;
            }

            self.heartbeat_with_log().await;

            let lease_result = if owns_lease {
                self.store
                    .renew_sampler_aggregator_lease(self.run_id, &self.worker_id, lease_ttl)
                    .await
            } else {
                self.store
                    .acquire_sampler_aggregator_lease(self.run_id, &self.worker_id, lease_ttl)
                    .await
            };

            match lease_result {
                Ok(has_lease) => owns_lease = has_lease,
                Err(err) => {
                    warn!(
                        target: "worker_log",
                        run_id = self.run_id,
                        node_id = %self.node_id,
                        worker_id = %self.worker_id,
                        role = %WorkerRole::SamplerAggregator,
                        event_type = "lease_operation_failed",
                        error = %err,
                        "lease operation failed"
                    );
                    owns_lease = false;
                }
            }

            if owns_lease && let Err(err) = runner.tick().await {
                warn!(
                    target: "worker_log",
                    run_id = self.run_id,
                    node_id = %self.node_id,
                    worker_id = %self.worker_id,
                    role = %WorkerRole::SamplerAggregator,
                    event_type = "tick_failed",
                    error = %err,
                    "sampler-aggregator tick failed"
                );
            }

            tokio::select! {
                _ = stop_rx.changed() => {}
                _ = sleep(interval) => {}
            }
        }

        if owns_lease
            && let Err(err) = self
                .store
                .release_sampler_aggregator_lease(self.run_id, &self.worker_id)
                .await
        {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::SamplerAggregator,
                event_type = "lease_release_failed",
                error = %err,
                "failed to release sampler-aggregator lease"
            );
        }

        self.mark_inactive_with_log().await;
        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "worker_stopped",
            "sampler-aggregator worker stopped"
        );

        Ok(())
    }

    async fn register_active_worker(&self, implementation: &str) -> Result<(), StoreError> {
        self.store
            .register_worker(&WorkerRecord {
                worker_id: self.worker_id.clone(),
                node_id: Some(self.node_id.clone()),
                role: self.role,
                implementation: implementation.to_string(),
                version: binary_version().to_string(),
                node_specs: json!({ "node_id": self.node_id }),
                status: WorkerStatus::Active,
                last_seen: None,
            })
            .await
    }

    async fn heartbeat_with_log(&self) {
        if let Err(err) = self.store.heartbeat_worker(&self.worker_id).await {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                worker_id = %self.worker_id,
                role = %self.role,
                event_type = "heartbeat_failed",
                error = %err,
                "worker heartbeat failed"
            );
        }
    }

    async fn mark_inactive_with_log(&self) {
        if let Err(err) = self
            .store
            .update_worker_status(&self.worker_id, WorkerStatus::Inactive)
            .await
        {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                worker_id = %self.worker_id,
                role = %self.role,
                event_type = "worker_inactive_failed",
                error = %err,
                "failed to mark worker inactive"
            );
        }
    }
}

pub struct NodeRunner<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    config: NodeRunnerConfig,
    role: Option<WorkerRole>,
    run_id: Option<i32>,
    worker_id: Option<String>,
    stop_tx: Option<watch::Sender<bool>>,
    handle: Option<JoinHandle<()>>,
}

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub fn new(store: S, node_id: impl Into<String>, config: NodeRunnerConfig) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            config,
            role: None,
            run_id: None,
            worker_id: None,
            stop_tx: None,
            handle: None,
        }
    }

    fn current_target(&self) -> Option<RoleTarget> {
        match (self.role, self.run_id) {
            (Some(role), Some(run_id)) => Some(RoleTarget { role, run_id }),
            _ => None,
        }
    }

    async fn resolve_desired_target(&self) -> Result<Option<RoleTarget>, StoreError> {
        let assignments = self
            .store
            .list_desired_assignments(Some(&self.node_id))
            .await?;

        if assignments.is_empty() {
            return Ok(None);
        }

        if assignments.len() == 1 {
            let assignment = &assignments[0];
            return Ok(Some(RoleTarget {
                role: assignment.role,
                run_id: assignment.run_id,
            }));
        }

        if let Some(current) = self.current_target()
            && let Some(matching) = assignments
                .iter()
                .find(|assignment| assignment.role == current.role)
        {
            warn!(
                node_id = %self.node_id,
                current_role = %current.role,
                conflict_count = assignments.len(),
                "multiple desired role assignments for one node; keeping current role assignment"
            );
            return Ok(Some(RoleTarget {
                role: matching.role,
                run_id: matching.run_id,
            }));
        }

        warn!(
            node_id = %self.node_id,
            conflict_count = assignments.len(),
            "multiple desired role assignments for one node; no active role selected"
        );
        Ok(None)
    }

    async fn reconcile(&mut self, desired_target: Option<RoleTarget>) -> Result<(), StoreError> {
        self.reap_finished_task().await;

        if self.current_target() == desired_target {
            return Ok(());
        }

        self.stop_current().await;

        if let Some(target) = desired_target {
            self.start(target);
        }

        Ok(())
    }

    fn start(&mut self, target: RoleTarget) {
        let worker_id = role_worker_id(&self.node_id, target.role);

        info!(
            target: "worker_log",
            run_id = target.run_id,
            node_id = %self.node_id,
            worker_id = %worker_id,
            role = %target.role,
            event_type = "role_start",
            "starting role task"
        );

        self.role = Some(target.role);
        self.run_id = Some(target.run_id);
        self.worker_id = Some(worker_id.clone());

        let runtime = ActiveWorker::new(
            self.store.clone(),
            self.node_id.clone(),
            worker_id.clone(),
            target.role,
            target.run_id,
        );

        let (stop_tx, stop_rx) = watch::channel(false);
        self.stop_tx = Some(stop_tx);

        self.handle = Some(tokio::spawn(async move {
            let result = runtime.run(stop_rx).await;
            if let Err(err) = result {
                error!(
                    target: "worker_log",
                    run_id = target.run_id,
                    worker_id = %worker_id,
                    role = %target.role,
                    event_type = "role_task_failed",
                    error = %err,
                    "role task failed"
                );
            }
        }));
    }

    async fn reap_finished_task(&mut self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        if !handle.is_finished() {
            return;
        }

        let run_id = self.run_id;
        let role = self.role;
        let worker_id = self.worker_id.clone();

        if let Some(handle) = self.handle.take()
            && let Err(err) = handle.await
        {
            warn!(
                target: "worker_log",
                run_id,
                node_id = %self.node_id,
                worker_id = %worker_id.unwrap_or_else(|| "unknown".to_string()),
                role = ?role,
                event_type = "role_task_join_failed",
                error = %err,
                "role task join failed"
            );
        }

        self.role = None;
        self.run_id = None;
        self.worker_id = None;
        self.stop_tx = None;

        if let (Some(run_id), Some(role)) = (run_id, role) {
            warn!(
                target: "worker_log",
                run_id,
                node_id = %self.node_id,
                role = %role,
                event_type = "role_task_exited",
                "role task exited; waiting for supervisor reconcile"
            );
        }
    }

    async fn stop_current(&mut self) {
        let (Some(run_id), Some(role), Some(worker_id)) =
            (self.run_id, self.role, self.worker_id.clone())
        else {
            self.role = None;
            self.run_id = None;
            self.worker_id = None;
            self.stop_tx = None;
            self.handle = None;
            return;
        };

        info!(
            target: "worker_log",
            run_id,
            node_id = %self.node_id,
            worker_id = %worker_id,
            role = %role,
            event_type = "role_stop",
            "stopping role task"
        );

        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(true);
        }

        if let Some(handle) = self.handle.take() {
            let mut handle = handle;
            match timeout(ROLE_TASK_SHUTDOWN_TIMEOUT, &mut handle).await {
                Ok(join_result) => {
                    if let Err(err) = join_result {
                        warn!(
                            target: "worker_log",
                            run_id,
                            node_id = %self.node_id,
                            worker_id = %worker_id,
                            role = %role,
                            event_type = "role_task_join_failed",
                            error = %err,
                            "role task join failed"
                        );
                    }
                }
                Err(_) => {
                    warn!(
                        target: "worker_log",
                        run_id,
                        node_id = %self.node_id,
                        worker_id = %worker_id,
                        role = %role,
                        event_type = "role_task_shutdown_timeout",
                        "timed out waiting for role task shutdown; aborting task"
                    );
                    handle.abort();
                }
            }
        }

        self.role = None;
        self.run_id = None;
        self.worker_id = None;
    }

    pub async fn run(mut self) -> Result<(), StoreError> {
        let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());

        loop {
            let desired_target = self.resolve_desired_target().await?;
            self.reconcile(desired_target).await?;

            tokio::select! {
                _ = &mut shutdown => {
                    info!(node_id = %self.node_id, "stopping node-runner");
                    break;
                }
                _ = sleep(self.config.poll_interval) => {}
            }
        }

        self.stop_current().await;
        Ok(())
    }
}

fn binary_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EvaluatorRunnerParams {
    min_loop_time_ms: u64,
    performance_snapshot_interval_ms: u64,
}

impl Default for EvaluatorRunnerParams {
    fn default() -> Self {
        Self {
            min_loop_time_ms: 0,
            performance_snapshot_interval_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SamplerAggregatorRunnerParams {
    interval_ms: u64,
    lease_ttl_ms: u64,
    nr_samples: usize,
    performance_snapshot_interval_ms: u64,
    max_pending_batches: usize,
    max_batches_per_tick: usize,
    completed_batch_fetch_limit: usize,
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
