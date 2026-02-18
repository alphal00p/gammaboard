//! Control-plane orchestration for node-local role runners.
//!
//! The node worker reconciles desired assignments from the DB control plane
//! into running local role tasks.

use crate::engines::test_only::{TestOnlySinEvaluator, TestOnlyTrainingSamplerAggregatorEngine};
use crate::{
    AssignmentLeaseStore, Worker, WorkerRegistryStore, WorkerRole,
    ControlPlaneStore, WorkerStatus, PgStore, RunSpecStore, RunnerConfig,
    SamplerAggregatorEngine, SamplerAggregatorRunner, WorkerRunner, WorkerRunnerConfig,
};
use serde_json::{Value as JsonValue, json};
use std::time::Duration;
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{sleep, timeout},
};

#[derive(Debug, Clone)]
pub struct NodeWorkerConfig {
    pub poll_interval: Duration,
}

impl Default for NodeWorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(1_000),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalRole {
    Evaluator,
    SamplerAggregator,
}

impl LocalRole {
    fn worker_role(self) -> WorkerRole {
        match self {
            LocalRole::Evaluator => WorkerRole::Evaluator,
            LocalRole::SamplerAggregator => WorkerRole::SamplerAggregator,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            LocalRole::Evaluator => "evaluator",
            LocalRole::SamplerAggregator => "sampler_aggregator",
        }
    }

    fn worker_id(self, node_id: &str) -> String {
        format!("{node_id}-{}", self.as_str())
    }
}

struct ManagedRoleTask {
    run_id: i32,
    stop_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

#[derive(Default)]
struct WorkerState {
    evaluator: Option<ManagedRoleTask>,
    sampler_aggregator: Option<ManagedRoleTask>,
}

pub async fn run_node_worker(
    store: PgStore,
    node_id: String,
    config: NodeWorkerConfig,
) -> Result<(), crate::StoreError> {
    let mut state = WorkerState::default();
    let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());

    loop {
        reconcile_role(&store, &node_id, LocalRole::Evaluator, &mut state.evaluator).await?;
        reconcile_role(
            &store,
            &node_id,
            LocalRole::SamplerAggregator,
            &mut state.sampler_aggregator,
        )
        .await?;

        tokio::select! {
            _ = &mut shutdown => {
                println!("🛑 stopping node-worker for node {}", node_id);
                break;
            }
            _ = sleep(config.poll_interval) => {}
        }
    }

    if let Some(task) = state.evaluator.take() {
        stop_task(task).await;
    }
    if let Some(task) = state.sampler_aggregator.take() {
        stop_task(task).await;
    }

    Ok(())
}

async fn reconcile_role(
    store: &PgStore,
    node_id: &str,
    role: LocalRole,
    slot: &mut Option<ManagedRoleTask>,
) -> Result<(), crate::StoreError> {
    let desired_run_id = store
        .get_desired_assignment(node_id, role.worker_role())
        .await?
        .map(|assignment| assignment.run_id);

    let current_run_id = slot.as_ref().map(|task| task.run_id);
    if current_run_id == desired_run_id {
        return Ok(());
    }

    if let Some(task) = slot.take() {
        println!(
            "↩️ node={} role={} stopping run {}",
            node_id,
            role.as_str(),
            task.run_id
        );
        stop_task(task).await;
    }

    if let Some(run_id) = desired_run_id {
        println!(
            "▶️ node={} role={} starting run {}",
            node_id,
            role.as_str(),
            run_id
        );
        *slot = Some(spawn_role_task(
            store.clone(),
            node_id.to_string(),
            role,
            run_id,
        ));
    }

    Ok(())
}

async fn stop_task(task: ManagedRoleTask) {
    let _ = task.stop_tx.send(true);
    match timeout(Duration::from_secs(15), task.join_handle).await {
        Ok(_) => {}
        Err(_) => {
            eprintln!("⚠️ timed out waiting for role task shutdown");
        }
    }
}

fn spawn_role_task(
    store: PgStore,
    node_id: String,
    role: LocalRole,
    run_id: i32,
) -> ManagedRoleTask {
    let (stop_tx, stop_rx) = watch::channel(false);

    let join_handle = tokio::spawn(async move {
        let result = match role {
            LocalRole::Evaluator => run_evaluator_task(store, node_id, run_id, stop_rx).await,
            LocalRole::SamplerAggregator => {
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
    store: PgStore,
    node_id: String,
    run_id: i32,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), crate::StoreError> {
    let worker_id = LocalRole::Evaluator.worker_id(&node_id);
    let Some(spec) = store.load_run_spec(run_id).await? else {
        eprintln!("⚠️ run {} has no RunSpec; evaluator not started", run_id);
        return Ok(());
    };

    let loop_sleep_ms = required_u64(&spec.worker_runner_params, "loop_sleep_ms")?;
    let min_eval_time_per_sample_ms =
        required_u64(&spec.worker_runner_params, "min_eval_time_per_sample_ms")?;

    store
        .register_worker(&Worker {
            worker_id: worker_id.clone(),
            node_id: Some(node_id.clone()),
            role: WorkerRole::Evaluator,
            implementation: spec.worker_implementation.clone(),
            version: spec.worker_version.clone(),
            node_specs: json!({ "node_id": node_id }),
            status: WorkerStatus::Active,
            last_seen: None,
        })
        .await?;
    store.assign_evaluator(run_id, &worker_id).await?;

    let mut runner = WorkerRunner::new(
        run_id,
        worker_id.clone(),
        TestOnlySinEvaluator,
        store.clone(),
        WorkerRunnerConfig {
            min_eval_time_per_sample: Duration::from_millis(min_eval_time_per_sample_ms),
        },
    );

    loop {
        if *stop_rx.borrow() {
            break;
        }

        if let Err(err) = store.heartbeat_worker(&worker_id).await {
            eprintln!("⚠️ heartbeat failed for {}: {}", worker_id, err);
        }

        if let Err(err) = runner.tick().await {
            eprintln!("⚠️ evaluator tick failed for {}: {}", worker_id, err);
        }

        tokio::select! {
            _ = stop_rx.changed() => {}
            _ = sleep(Duration::from_millis(loop_sleep_ms)) => {}
        }
    }

    if let Err(err) = store.unassign_evaluator(run_id, &worker_id).await {
        eprintln!(
            "⚠️ failed to unassign evaluator {} from run {}: {}",
            worker_id, run_id, err
        );
    }
    if let Err(err) = store
        .update_worker_status(&worker_id, WorkerStatus::Inactive)
        .await
    {
        eprintln!(
            "⚠️ failed to mark evaluator {} inactive: {}",
            worker_id, err
        );
    }

    Ok(())
}

async fn run_sampler_aggregator_task(
    store: PgStore,
    node_id: String,
    run_id: i32,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), crate::StoreError> {
    let worker_id = LocalRole::SamplerAggregator.worker_id(&node_id);
    let Some(spec) = store.load_run_spec(run_id).await? else {
        eprintln!(
            "⚠️ run {} has no RunSpec; sampler-aggregator not started",
            run_id
        );
        return Ok(());
    };

    let interval_ms = required_u64(&spec.sampler_aggregator_runner_params, "interval_ms")?;
    let lease_ttl_ms = required_u64(&spec.sampler_aggregator_runner_params, "lease_ttl_ms")?;
    let max_pending_batches = required_usize(
        &spec.sampler_aggregator_runner_params,
        "max_pending_batches",
    )?;
    let max_batches_per_tick = required_usize(
        &spec.sampler_aggregator_runner_params,
        "max_batches_per_tick",
    )?;
    let completed_batch_fetch_limit = required_usize(
        &spec.sampler_aggregator_runner_params,
        "completed_batch_fetch_limit",
    )?;
    let batch_size = optional_usize(&spec.sampler_aggregator_params, "batch_size")?.unwrap_or(64);
    let training_target_samples =
        optional_usize(&spec.sampler_aggregator_params, "training_target_samples")?.unwrap_or(0);
    let training_delay_per_sample_ms = optional_u64(
        &spec.sampler_aggregator_params,
        "training_delay_per_sample_ms",
    )?
    .unwrap_or(0);

    let engine = TestOnlyTrainingSamplerAggregatorEngine::new(
        batch_size,
        training_target_samples,
        training_delay_per_sample_ms,
    );
    let implementation = engine.implementation().to_string();
    let version = engine.version().to_string();

    store
        .register_worker(&Worker {
            worker_id: worker_id.clone(),
            node_id: Some(node_id.clone()),
            role: WorkerRole::SamplerAggregator,
            implementation,
            version,
            node_specs: json!({ "node_id": node_id }),
            status: WorkerStatus::Active,
            last_seen: None,
        })
        .await?;

    let mut runner = SamplerAggregatorRunner::new(
        run_id,
        engine,
        store.clone(),
        store.clone(),
        store.clone(),
        RunnerConfig {
            max_batches_per_tick,
            max_pending_batches,
            completed_batch_fetch_limit,
        },
    )
    .await
    .map_err(|err| crate::StoreError::new(err.to_string()))?;

    let lease_ttl = Duration::from_millis(lease_ttl_ms);
    let mut owns_lease = false;

    loop {
        if *stop_rx.borrow() {
            break;
        }

        if let Err(err) = store.heartbeat_worker(&worker_id).await {
            eprintln!("⚠️ heartbeat failed for {}: {}", worker_id, err);
        }

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

        if owns_lease {
            if let Err(err) = runner.tick().await {
                eprintln!(
                    "⚠️ sampler-aggregator tick failed for {}: {}",
                    worker_id, err
                );
            }
        }

        tokio::select! {
            _ = stop_rx.changed() => {}
            _ = sleep(Duration::from_millis(interval_ms)) => {}
        }
    }

    if owns_lease {
        if let Err(err) = store
            .release_sampler_aggregator_lease(run_id, &worker_id)
            .await
        {
            eprintln!(
                "⚠️ failed to release sampler-aggregator lease for run {}: {}",
                run_id, err
            );
        }
    }
    if let Err(err) = store
        .update_worker_status(&worker_id, WorkerStatus::Inactive)
        .await
    {
        eprintln!(
            "⚠️ failed to mark sampler-aggregator {} inactive: {}",
            worker_id, err
        );
    }

    Ok(())
}

fn required_u64(params: &JsonValue, key: &str) -> Result<u64, crate::StoreError> {
    params.get(key).and_then(|v| v.as_u64()).ok_or_else(|| {
        crate::StoreError::new(format!(
            "missing or invalid configuration parameter: {}",
            key
        ))
    })
}

fn required_usize(params: &JsonValue, key: &str) -> Result<usize, crate::StoreError> {
    let raw = required_u64(params, key)?;
    usize::try_from(raw).map_err(|_| {
        crate::StoreError::new(format!(
            "configuration parameter {} is too large for usize",
            key
        ))
    })
}

fn optional_u64(params: &JsonValue, key: &str) -> Result<Option<u64>, crate::StoreError> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        crate::StoreError::new(format!(
            "optional configuration parameter {} is not a u64",
            key
        ))
    })
}

fn optional_usize(params: &JsonValue, key: &str) -> Result<Option<usize>, crate::StoreError> {
    let Some(raw) = optional_u64(params, key)? else {
        return Ok(None);
    };
    usize::try_from(raw).map(Some).map_err(|_| {
        crate::StoreError::new(format!(
            "optional configuration parameter {} is too large for usize",
            key
        ))
    })
}
