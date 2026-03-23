use crate::api::ApiError;
use crate::core::{ControlPlaneStore, RunReadStore, WorkerRole};

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

pub async fn unassign_node(
    store: &impl ControlPlaneStore,
    node_name: &str,
) -> Result<(), ApiError> {
    store.clear_desired_assignment(node_name).await?;
    Ok(())
}

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

pub async fn auto_assign_run(
    store: &(impl ControlPlaneStore + RunReadStore),
    run_id: i32,
    max_evaluators: Option<usize>,
) -> Result<AutoAssignResult, ApiError> {
    let run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))?;
    let nodes = store.list_nodes(None).await?;
    let free_nodes = nodes
        .iter()
        .filter(|node| node.desired_assignment.is_none())
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();
    let sampler_already_assigned = nodes.iter().any(|node| {
        node.desired_assignment.as_ref().is_some_and(|assignment| {
            assignment.run_id == run_id && assignment.role == WorkerRole::SamplerAggregator
        })
    });

    let evaluator_limit = max_evaluators.unwrap_or(usize::MAX);
    let mut assigned_sampler = None;
    let mut assigned_evaluators = Vec::new();
    let mut free_iter = free_nodes.into_iter();

    if !sampler_already_assigned {
        if let Some(node_name) = free_iter.next() {
            store
                .upsert_desired_assignment(&node_name, WorkerRole::SamplerAggregator, run_id)
                .await?;
            assigned_sampler = Some(node_name);
        }
    }

    for node_name in free_iter.take(evaluator_limit) {
        store
            .upsert_desired_assignment(&node_name, WorkerRole::Evaluator, run_id)
            .await?;
        assigned_evaluators.push(node_name);
    }

    Ok(AutoAssignResult {
        run_id,
        run_name: run.run_name,
        sampler_already_assigned,
        assigned_sampler,
        assigned_evaluators,
    })
}
