use crate::api::ApiError;
use crate::core::{
    CapabilityRequirements, ControlPlaneStore, NodeCapabilities, NodeLaunchRequest, RegisteredNode,
    RunReadStore, RunSpecStore, WorkerRole,
};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct AssignedNode {
    pub node_name: String,
    pub run_id: i32,
    pub run_name: String,
    pub role: WorkerRole,
}

#[derive(Debug, Clone)]
pub struct AutoAssignResult {
    pub run_id: i32,
    pub run_name: String,
    pub sampler_already_assigned: bool,
    pub assigned_sampler: Option<String>,
    pub assigned_evaluators: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StoppedNode {
    pub node_name: String,
    pub rows_updated: u64,
}

#[derive(Debug, Clone)]
pub struct StoppedAllNodes {
    pub rows_updated: u64,
}

#[derive(Debug, Clone)]
pub struct GracefulNodeShutdownParams {
    pub sampler_drain_timeout: Duration,
    pub node_stop_timeout: Duration,
    pub poll_interval: Duration,
}

#[derive(Debug, Clone)]
pub struct GracefulNodeShutdownResult {
    pub assignments_cleared: u64,
    pub rows_updated: u64,
    pub sampler_drain_timed_out: bool,
    pub node_stop_timed_out: bool,
    pub active_samplers_remaining: usize,
    pub live_nodes_remaining: usize,
}

#[derive(Debug, Clone)]
pub struct AutoRunNodesPlan {
    pub requested_count: usize,
    pub node_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreatedNodeLaunchRequest {
    pub request: NodeLaunchRequest,
    pub should_resolve_locally: bool,
}

pub async fn create_node_launch_request(
    store: &impl ControlPlaneStore,
    count: usize,
    backend: &str,
    name_prefix: Option<&str>,
    args: &JsonValue,
    resolve_locally: bool,
) -> Result<CreatedNodeLaunchRequest, ApiError> {
    if count == 0 {
        return Err(ApiError::BadRequest(
            "requested node count must be greater than zero".to_string(),
        ));
    }
    let requested_count = i32::try_from(count)
        .map_err(|_| ApiError::BadRequest("requested node count is too large".to_string()))?;
    let request = store
        .create_node_launch_request(backend, requested_count, name_prefix, args)
        .await?;
    Ok(CreatedNodeLaunchRequest {
        request,
        should_resolve_locally: resolve_locally,
    })
}

pub async fn list_node_launch_requests(
    store: &impl ControlPlaneStore,
) -> Result<Vec<NodeLaunchRequest>, ApiError> {
    Ok(store.list_node_launch_requests().await?)
}

pub async fn claim_external_node_launch_request(
    store: &impl ControlPlaneStore,
) -> Result<Option<NodeLaunchRequest>, ApiError> {
    Ok(store.claim_external_node_launch_request().await?)
}

pub async fn reconcile_running_node_launch_requests(
    store: &impl ControlPlaneStore,
) -> Result<u64, ApiError> {
    Ok(store.reconcile_running_node_launch_requests().await?)
}

pub async fn mark_node_launch_request_starting(
    store: &impl ControlPlaneStore,
    id: i64,
    started_count: usize,
    result: &JsonValue,
) -> Result<NodeLaunchRequest, ApiError> {
    let started_count = i32::try_from(started_count)
        .map_err(|_| ApiError::Internal("started node count is too large".to_string()))?;
    Ok(store
        .update_node_launch_request_state(id, "starting", started_count, result, None)
        .await?)
}

pub async fn mark_node_launch_request_running(
    store: &impl ControlPlaneStore,
    id: i64,
    started_count: usize,
    result: &JsonValue,
) -> Result<NodeLaunchRequest, ApiError> {
    let started_count = i32::try_from(started_count)
        .map_err(|_| ApiError::Internal("started node count is too large".to_string()))?;
    Ok(store
        .update_node_launch_request_state(id, "running", started_count, result, None)
        .await?)
}

pub async fn mark_node_launch_request_failed(
    store: &impl ControlPlaneStore,
    id: i64,
    started_count: usize,
    result: &JsonValue,
    error: &str,
) -> Result<NodeLaunchRequest, ApiError> {
    let started_count = i32::try_from(started_count)
        .map_err(|_| ApiError::Internal("started node count is too large".to_string()))?;
    Ok(store
        .update_node_launch_request_state(id, "failed", started_count, result, Some(error))
        .await?)
}

pub async fn mark_node_launch_request_canceled(
    store: &impl ControlPlaneStore,
    id: i64,
    started_count: usize,
    result: &JsonValue,
) -> Result<NodeLaunchRequest, ApiError> {
    let started_count = i32::try_from(started_count)
        .map_err(|_| ApiError::Internal("started node count is too large".to_string()))?;
    Ok(store
        .update_node_launch_request_state(id, "canceled", started_count, result, None)
        .await?)
}

/// Assigns a node to a run/role in desired control-plane state.
pub async fn assign_node(
    store: &(impl ControlPlaneStore + RunReadStore),
    node_name: &str,
    run_id: i32,
    role: WorkerRole,
) -> Result<AssignedNode, ApiError> {
    let run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    store
        .upsert_desired_assignment(node_name, role, run_id)
        .await?;
    Ok(AssignedNode {
        node_name: node_name.to_string(),
        run_id,
        run_name: run.run_name,
        role,
    })
}

/// Clears a node's desired assignment.
pub async fn unassign_node(
    store: &impl ControlPlaneStore,
    node_name: &str,
) -> Result<(), ApiError> {
    store.clear_desired_assignment(node_name).await?;
    Ok(())
}

/// Requests shutdown for a specific node name.
pub async fn stop_node(
    store: &impl ControlPlaneStore,
    node_name: &str,
) -> Result<StoppedNode, ApiError> {
    let rows_updated = store.request_node_shutdown(node_name).await?;
    Ok(StoppedNode {
        node_name: node_name.to_string(),
        rows_updated,
    })
}

/// Requests shutdown for all currently registered nodes.
pub async fn stop_all_nodes(store: &impl ControlPlaneStore) -> Result<StoppedAllNodes, ApiError> {
    let rows_updated = store.request_all_nodes_shutdown().await?;
    Ok(StoppedAllNodes { rows_updated })
}

pub async fn stop_all_nodes_gracefully(
    store: &impl ControlPlaneStore,
    params: GracefulNodeShutdownParams,
) -> Result<GracefulNodeShutdownResult, ApiError> {
    let assignments_cleared = store.clear_all_desired_assignments().await?;
    let rows_updated = store.request_all_nodes_shutdown().await?;
    let wait_result = wait_for_graceful_node_shutdown(store, params).await?;

    Ok(GracefulNodeShutdownResult {
        assignments_cleared,
        rows_updated,
        sampler_drain_timed_out: wait_result.sampler_drain_timed_out,
        node_stop_timed_out: wait_result.node_stop_timed_out,
        active_samplers_remaining: wait_result.active_samplers_remaining,
        live_nodes_remaining: wait_result.live_nodes_remaining,
    })
}

struct GracefulNodeShutdownWaitResult {
    sampler_drain_timed_out: bool,
    node_stop_timed_out: bool,
    active_samplers_remaining: usize,
    live_nodes_remaining: usize,
}

async fn wait_for_graceful_node_shutdown(
    store: &impl ControlPlaneStore,
    params: GracefulNodeShutdownParams,
) -> Result<GracefulNodeShutdownWaitResult, ApiError> {
    let sampler_deadline = Instant::now() + params.sampler_drain_timeout;
    let mut node_deadline = None;
    let mut sampler_drain_timed_out = false;
    loop {
        let nodes = store.list_nodes(None).await?;
        let live_nodes_remaining = nodes.len();
        let active_samplers_remaining = nodes
            .iter()
            .filter(|node| {
                node.current_assignment
                    .as_ref()
                    .is_some_and(|assignment| assignment.role == WorkerRole::SamplerAggregator)
            })
            .count();
        let now = Instant::now();

        if active_samplers_remaining == 0 || now >= sampler_deadline {
            sampler_drain_timed_out = active_samplers_remaining > 0;
            node_deadline.get_or_insert(now + params.node_stop_timeout);
        }

        if live_nodes_remaining == 0 {
            return Ok(GracefulNodeShutdownWaitResult {
                sampler_drain_timed_out,
                node_stop_timed_out: false,
                active_samplers_remaining,
                live_nodes_remaining,
            });
        }

        if node_deadline.is_some_and(|deadline| now >= deadline) {
            return Ok(GracefulNodeShutdownWaitResult {
                sampler_drain_timed_out,
                node_stop_timed_out: true,
                active_samplers_remaining,
                live_nodes_remaining,
            });
        }

        tokio::time::sleep(params.poll_interval).await;
    }
}

/// Auto-assigns currently free nodes to sampler/evaluator roles for a run.
pub async fn auto_assign_run(
    store: &(impl ControlPlaneStore + RunReadStore + RunSpecStore),
    run_id: i32,
    max_evaluators: Option<usize>,
) -> Result<AutoAssignResult, ApiError> {
    let run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let run_spec = store
        .load_run_spec(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} spec not found")))?;
    let nodes = store.list_nodes(None).await?;
    let sampler_already_assigned = nodes.iter().any(|node| {
        node.desired_assignment.as_ref().is_some_and(|assignment| {
            assignment.run_id == run_id && assignment.role == WorkerRole::SamplerAggregator
        })
    });
    let mut free_nodes = nodes
        .into_iter()
        .filter(|node| node.desired_assignment.is_none())
        .collect::<Vec<_>>();

    let evaluator_limit = max_evaluators.unwrap_or(usize::MAX);
    let mut assigned_sampler = None;
    let mut assigned_evaluators = Vec::new();

    if !sampler_already_assigned {
        if let Some(node) = take_best_node(&mut free_nodes, &run_spec.sampler_requirements) {
            store
                .upsert_desired_assignment(&node.name, WorkerRole::SamplerAggregator, run_id)
                .await?;
            assigned_sampler = Some(node.name);
        }
    }

    for node in take_best_nodes(
        &mut free_nodes,
        &run_spec.evaluator_requirements,
        evaluator_limit,
    ) {
        store
            .upsert_desired_assignment(&node.name, WorkerRole::Evaluator, run_id)
            .await?;
        assigned_evaluators.push(node.name);
    }

    Ok(AutoAssignResult {
        run_id,
        run_name: run.run_name,
        sampler_already_assigned,
        assigned_sampler,
        assigned_evaluators,
    })
}

pub fn capabilities_satisfy(
    capabilities: &NodeCapabilities,
    requirements: &CapabilityRequirements,
) -> bool {
    requirements.iter().all(|(key, required)| {
        capabilities
            .get(key)
            .is_some_and(|available| available >= required)
    })
}

fn capability_score(
    capabilities: &NodeCapabilities,
    requirements: &CapabilityRequirements,
) -> (u64, u64, usize, String) {
    let extra_required = requirements
        .iter()
        .map(|(key, required)| capabilities.get(key).copied().unwrap_or(0) - required)
        .sum::<u64>();
    let extra_unrequired = capabilities
        .iter()
        .filter(|(key, _)| !requirements.contains_key(*key))
        .map(|(key, value)| capability_weight(key, *value))
        .sum::<u64>();
    (
        extra_required,
        extra_unrequired,
        capabilities.len(),
        String::new(),
    )
}

fn capability_weight(key: &str, value: u64) -> u64 {
    if key.contains("gpu") || key.contains("cuda") || key.contains("madnis") {
        value.saturating_mul(1_000)
    } else {
        value
    }
}

fn best_node_index(
    nodes: &[RegisteredNode],
    requirements: &CapabilityRequirements,
) -> Option<usize> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| capabilities_satisfy(&node.capabilities, requirements))
        .min_by_key(|(_, node)| {
            let mut score = capability_score(&node.capabilities, requirements);
            score.3 = node.name.clone();
            score
        })
        .map(|(index, _)| index)
}

fn take_best_node(
    nodes: &mut Vec<RegisteredNode>,
    requirements: &CapabilityRequirements,
) -> Option<RegisteredNode> {
    let index = best_node_index(nodes, requirements)?;
    Some(nodes.remove(index))
}

fn take_best_nodes(
    nodes: &mut Vec<RegisteredNode>,
    requirements: &CapabilityRequirements,
    limit: usize,
) -> Vec<RegisteredNode> {
    let mut selected = Vec::new();
    while selected.len() < limit {
        let Some(node) = take_best_node(nodes, requirements) else {
            break;
        };
        selected.push(node);
    }
    selected
}

/// Plans `w-N` node names for launching local node processes, skipping existing names.
pub async fn plan_auto_run_nodes(
    store: &impl ControlPlaneStore,
    requested_count: usize,
) -> Result<AutoRunNodesPlan, ApiError> {
    if requested_count == 0 {
        return Err(ApiError::BadRequest(
            "requested node count must be greater than zero".to_string(),
        ));
    }

    let existing = store
        .list_nodes(None)
        .await?
        .into_iter()
        .map(|node| node.name)
        .collect::<HashSet<_>>();

    let mut planned = Vec::with_capacity(requested_count);
    let mut index = 1usize;
    while planned.len() < requested_count {
        let candidate = format!("w-{index}");
        if !existing.contains(&candidate) && !planned.iter().any(|name| name == &candidate) {
            planned.push(candidate);
        }
        index = index.saturating_add(1);
    }

    Ok(AutoRunNodesPlan {
        requested_count,
        node_names: planned,
    })
}

/// Builds CLI arguments for launching one node process.
pub fn node_run_cli_args(
    node_name: &str,
    max_start_failures: u32,
    capabilities: &NodeCapabilities,
) -> Vec<String> {
    let mut args = vec![
        "node".to_string(),
        "run".to_string(),
        "--name".to_string(),
        node_name.to_string(),
        "--max-start-failures".to_string(),
        max_start_failures.to_string(),
    ];
    for (key, value) in capabilities {
        args.push("--capability".to_string());
        args.push(format!("{key}={value}"));
    }
    args
}
