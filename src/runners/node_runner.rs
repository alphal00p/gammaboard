//! Node-local worker orchestration and role reconciliation.

use super::{
    evaluator::EvaluatorRunner, sampler_aggregator::RunnerConfig,
    sampler_aggregator::SamplerAggregatorRunner,
};
use crate::core::{
    AggregationStore, AssignmentLeaseStore, ControlPlaneStore, RunSpecStore, StoreError,
    WorkQueueStore, Worker, WorkerRegistryStore, WorkerRole, WorkerStatus,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value as JsonValue, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{sleep, timeout},
};

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

struct ManagedRoleTask {
    run_id: i32,
    // Cooperative shutdown signal for the background role loop.
    stop_tx: watch::Sender<bool>,
    // Join handle so we can await termination when reconciling/stopping.
    join_handle: JoinHandle<()>,
}

#[derive(Default)]
struct WorkerState {
    evaluator: Option<ManagedRoleTask>,
    sampler_aggregator: Option<ManagedRoleTask>,
}

const ROLE_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

pub struct NodeRunner<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    config: NodeRunnerConfig,
    state: WorkerState,
}

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub fn new(store: S, node_id: impl Into<String>, config: NodeRunnerConfig) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            config,
            state: WorkerState::default(),
        }
    }

    pub async fn run(mut self) -> Result<(), StoreError> {
        let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());

        loop {
            // Reconciliation is idempotent; only starts/stops tasks on assignment changes.
            self.reconcile_role(WorkerRole::Evaluator).await?;
            self.reconcile_role(WorkerRole::SamplerAggregator).await?;

            tokio::select! {
                // Wake immediately on Ctrl+C instead of waiting for next poll tick.
                _ = &mut shutdown => {
                    println!("🛑 stopping node-runner for node {}", self.node_id);
                    break;
                }
                // Periodic control-plane poll.
                _ = sleep(self.config.poll_interval) => {}
            }
        }

        self.stop_all().await;
        Ok(())
    }

    async fn reconcile_role(&mut self, role: WorkerRole) -> Result<(), StoreError> {
        let desired_run_id = self
            .store
            .get_desired_assignment(&self.node_id, role)
            .await?
            .map(|assignment| assignment.run_id);
        let current_run_id = self.role_slot_mut(role).as_ref().map(|task| task.run_id);

        if current_run_id == desired_run_id {
            return Ok(());
        }

        self.stop_slot(role).await;

        if let Some(run_id) = desired_run_id {
            println!(
                "▶️ node={} role={} starting run {}",
                self.node_id, role, run_id
            );
            *self.role_slot_mut(role) = Some(spawn_role_task(
                self.store.clone(),
                self.node_id.clone(),
                role,
                run_id,
            ));
        }

        Ok(())
    }

    fn role_slot_mut(&mut self, role: WorkerRole) -> &mut Option<ManagedRoleTask> {
        match role {
            WorkerRole::Evaluator => &mut self.state.evaluator,
            WorkerRole::SamplerAggregator => &mut self.state.sampler_aggregator,
        }
    }

    async fn stop_slot(&mut self, role: WorkerRole) {
        if let Some(task) = self.role_slot_mut(role).take() {
            println!(
                "↩️ node={} role={} stopping run {}",
                self.node_id, role, task.run_id
            );
            stop_task(task).await;
        }
    }

    async fn stop_all(&mut self) {
        self.stop_slot(WorkerRole::Evaluator).await;
        self.stop_slot(WorkerRole::SamplerAggregator).await;
    }
}

async fn stop_task(task: ManagedRoleTask) {
    // Ask task to stop cooperatively.
    let _ = task.stop_tx.send(true);
    // Bound shutdown latency so reconciliation cannot hang forever.
    match timeout(ROLE_TASK_SHUTDOWN_TIMEOUT, task.join_handle).await {
        Ok(_) => {}
        Err(_) => {
            eprintln!("⚠️ timed out waiting for role task shutdown");
        }
    }
}

async fn register_active_worker(
    store: &impl NodeRunnerStore,
    worker_id: &str,
    node_id: &str,
    role: WorkerRole,
    implementation: &str,
    version: &str,
) -> Result<(), StoreError> {
    store
        .register_worker(&Worker {
            worker_id: worker_id.to_string(),
            node_id: Some(node_id.to_string()),
            role,
            implementation: implementation.to_string(),
            version: version.to_string(),
            node_specs: json!({ "node_id": node_id }),
            status: WorkerStatus::Active,
            last_seen: None,
        })
        .await
}

async fn heartbeat_with_log(store: &impl NodeRunnerStore, worker_id: &str) {
    if let Err(err) = store.heartbeat_worker(worker_id).await {
        eprintln!("⚠️ heartbeat failed for {}: {}", worker_id, err);
    }
}

async fn mark_inactive_with_log(store: &impl NodeRunnerStore, worker_id: &str, role: WorkerRole) {
    if let Err(err) = store
        .update_worker_status(worker_id, WorkerStatus::Inactive)
        .await
    {
        eprintln!("⚠️ failed to mark {} {} inactive: {}", role, worker_id, err);
    }
}

fn spawn_role_task(
    store: impl NodeRunnerStore,
    node_id: String,
    role: WorkerRole,
    run_id: i32,
) -> ManagedRoleTask {
    // `watch` is used so each loop can cheaply check current stop state and await changes.
    let (stop_tx, stop_rx) = watch::channel(false);

    // Each role gets its own long-lived background task.
    let join_handle = tokio::spawn(async move {
        let result = match role {
            WorkerRole::Evaluator => run_evaluator_task(store, node_id, run_id, stop_rx).await,
            WorkerRole::SamplerAggregator => {
                run_sampler_aggregator_task(store, node_id, run_id, stop_rx).await
            }
        };

        if let Err(err) = result {
            eprintln!("❌ role task failed: {}", err);
        }
    });

    ManagedRoleTask {
        run_id,
        stop_tx,
        join_handle,
    }
}

async fn run_evaluator_task(
    store: impl NodeRunnerStore,
    node_id: String,
    run_id: i32,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let worker_id = role_worker_id(&node_id, WorkerRole::Evaluator);
    let Some(spec) = store.load_run_spec(run_id).await? else {
        eprintln!("⚠️ run {} has no RunSpec; evaluator not started", run_id);
        return Ok(());
    };

    let evaluator_params: EvaluatorRunnerParams =
        parse_params(&spec.evaluator_runner_params, "evaluator_runner_params")?;
    let evaluator = spec
        .evaluator_implementation
        .build(&spec.evaluator_params)
        .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?;
    evaluator
        .validate_point_spec(&spec.point_spec)
        .map_err(|err| {
            StoreError::store(format!(
                "incompatible evaluator for point_spec on run {}: {}",
                run_id, err
            ))
        })?;
    let evaluator_implementation = evaluator.implementation();
    let evaluator_version = evaluator.version();

    register_active_worker(
        &store,
        &worker_id,
        &node_id,
        WorkerRole::Evaluator,
        evaluator_implementation,
        evaluator_version,
    )
    .await?;
    store.assign_evaluator(run_id, &worker_id).await?;

    let mut runner = EvaluatorRunner::new(
        run_id,
        worker_id.clone(),
        evaluator,
        Arc::new({
            let implementation = spec.evaluator_implementation;
            move |params: &JsonValue| implementation.build_observable(params)
        }),
        spec.observable_params.clone(),
        spec.point_spec.clone(),
        store.clone(),
    );

    loop {
        // Fast path stop check before doing work this tick.
        if *stop_rx.borrow() {
            break;
        }
        let tick_started = Instant::now();

        heartbeat_with_log(&store, &worker_id).await;

        if let Err(err) = runner.tick().await {
            eprintln!("⚠️ evaluator tick failed for {}: {}", worker_id, err);
        }

        // Keep a minimum tick period while still allowing immediate stop.
        let elapsed = tick_started.elapsed();
        let min_loop_time = Duration::from_millis(evaluator_params.min_loop_time_ms);
        if elapsed < min_loop_time {
            tokio::select! {
                _ = stop_rx.changed() => {}
                _ = sleep(min_loop_time - elapsed) => {}
            }
        }
    }

    if let Err(err) = store.unassign_evaluator(run_id, &worker_id).await {
        eprintln!(
            "⚠️ failed to unassign evaluator {} from run {}: {}",
            worker_id, run_id, err
        );
    }
    mark_inactive_with_log(&store, &worker_id, WorkerRole::Evaluator).await;

    Ok(())
}

async fn run_sampler_aggregator_task(
    store: impl NodeRunnerStore,
    node_id: String,
    run_id: i32,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let worker_id = role_worker_id(&node_id, WorkerRole::SamplerAggregator);
    let Some(spec) = store.load_run_spec(run_id).await? else {
        eprintln!(
            "⚠️ run {} has no RunSpec; sampler-aggregator not started",
            run_id
        );
        return Ok(());
    };

    let runner_params: SamplerAggregatorRunnerParams = parse_params(
        &spec.sampler_aggregator_runner_params,
        "sampler_aggregator_runner_params",
    )?;
    let engine = spec
        .sampler_aggregator_implementation
        .build(&spec.sampler_aggregator_params)
        .map_err(|err| StoreError::store(format!("failed to build sampler-aggregator: {err}")))?;
    engine
        .validate_point_spec(&spec.point_spec)
        .map_err(|err| {
            StoreError::store(format!(
                "incompatible sampler-aggregator for point_spec on run {}: {}",
                run_id, err
            ))
        })?;
    let sampler_aggregator_implementation = engine.implementation();
    let sampler_aggregator_version = engine.version();
    let aggregated_observable = spec
        .evaluator_implementation
        .build_observable(&spec.observable_params)
        .map_err(|err| {
            StoreError::store(format!("failed to build aggregated observable: {err}"))
        })?;

    register_active_worker(
        &store,
        &worker_id,
        &node_id,
        WorkerRole::SamplerAggregator,
        sampler_aggregator_implementation,
        sampler_aggregator_version,
    )
    .await?;

    let mut runner = SamplerAggregatorRunner::new(
        run_id,
        engine,
        aggregated_observable,
        store.clone(),
        store.clone(),
        RunnerConfig {
            max_batches_per_tick: runner_params.max_batches_per_tick,
            max_pending_batches: runner_params.max_pending_batches,
            completed_batch_fetch_limit: runner_params.completed_batch_fetch_limit,
        },
        spec.point_spec.clone(),
    )
    .await
    .map_err(|err| StoreError::store(err.to_string()))?;

    let lease_ttl = Duration::from_millis(runner_params.lease_ttl_ms);
    let mut owns_lease = false;

    loop {
        // Fast path stop check before lease/tick work.
        if *stop_rx.borrow() {
            break;
        }

        heartbeat_with_log(&store, &worker_id).await;

        // Exactly one sampler-aggregator should actively tick per run.
        // Acquire on first iteration, then renew while ownership is kept.
        let lease_result = if owns_lease {
            store
                .renew_sampler_aggregator_lease(run_id, &worker_id, lease_ttl)
                .await
        } else {
            store
                .acquire_sampler_aggregator_lease(run_id, &worker_id, lease_ttl)
                .await
        };

        match lease_result {
            Ok(has_lease) => owns_lease = has_lease,
            Err(err) => {
                eprintln!("⚠️ lease operation failed for {}: {}", worker_id, err);
                owns_lease = false;
            }
        }

        // Only lease owner is allowed to enqueue/process batches for this run.
        if owns_lease && let Err(err) = runner.tick().await {
            eprintln!(
                "⚠️ sampler-aggregator tick failed for {}: {}",
                worker_id, err
            );
        }

        // Sleep between attempts, but stop immediately when requested.
        tokio::select! {
            _ = stop_rx.changed() => {}
            _ = sleep(Duration::from_millis(runner_params.interval_ms)) => {}
        }
    }

    if owns_lease
        && let Err(err) = store
            .release_sampler_aggregator_lease(run_id, &worker_id)
            .await
    {
        eprintln!(
            "⚠️ failed to release sampler-aggregator lease for run {}: {}",
            run_id, err
        );
    }
    mark_inactive_with_log(&store, &worker_id, WorkerRole::SamplerAggregator).await;

    Ok(())
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct EvaluatorRunnerParams {
    min_loop_time_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct SamplerAggregatorRunnerParams {
    interval_ms: u64,
    lease_ttl_ms: u64,
    max_pending_batches: usize,
    max_batches_per_tick: usize,
    completed_batch_fetch_limit: usize,
}

impl Default for SamplerAggregatorRunnerParams {
    fn default() -> Self {
        Self {
            interval_ms: 500,
            lease_ttl_ms: 5_000,
            max_pending_batches: 128,
            max_batches_per_tick: 1,
            completed_batch_fetch_limit: 512,
        }
    }
}

fn parse_params<T: DeserializeOwned>(params: &JsonValue, section: &str) -> Result<T, StoreError> {
    serde_json::from_value(params.clone())
        .map_err(|err| StoreError::store(format!("invalid {}: {}", section, err)))
}
