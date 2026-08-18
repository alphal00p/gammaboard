use crate::api::measurement::load_task_measurement_output;
use crate::core::StoreResultExt;
use crate::core::{
    ControlPlaneStore, DesiredAssignment, RegisteredNode, RunReadStore, RunTaskState, RunTaskStore,
    StoreError, TaskMeasurementOutput, WorkerRole,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct ChildTaskMeasurement {
    pub task_state: RunTaskState,
    pub output: Option<TaskMeasurementOutput>,
}

pub async fn redistribute_parent_assignments_to_children(
    store: &impl ControlPlaneStore,
    parent_run_id: i32,
    child_run_ids: impl IntoIterator<Item = i32>,
) -> Result<(), StoreError> {
    let parent_assignments = store
        .list_desired_assignments(None)
        .await?
        .into_iter()
        .filter(|assignment| assignment.run_id == parent_run_id)
        .collect::<Vec<_>>();
    let mut seen_child_run_ids = BTreeSet::new();
    let child_run_ids = child_run_ids
        .into_iter()
        .filter(|run_id| seen_child_run_ids.insert(*run_id))
        .collect::<Vec<_>>();

    store
        .clear_desired_assignments_for_run(parent_run_id)
        .await?;
    if child_run_ids.is_empty() {
        return Ok(());
    }
    let nodes = store.list_nodes(None).await?;
    let child_run_id_set = child_run_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut sampler_child_run_ids = child_sampler_run_ids(&nodes, &child_run_id_set);
    let parent_assignments = if parent_assignments.is_empty() {
        idle_node_assignments_from_nodes(&nodes)
    } else {
        parent_assignments
    };
    if parent_assignments.is_empty() {
        return Ok(());
    }

    let mut sampler_nodes = parent_assignments
        .iter()
        .filter(|assignment| assignment.role == WorkerRole::SamplerAggregator)
        .map(|assignment| assignment.node_name.as_str())
        .collect::<Vec<_>>();
    let evaluator_nodes = parent_assignments
        .iter()
        .filter(|assignment| assignment.role == WorkerRole::Evaluator)
        .map(|assignment| assignment.node_name.as_str())
        .collect::<Vec<_>>();

    sampler_nodes.sort_unstable();
    let children_needing_sampler = child_run_ids
        .iter()
        .copied()
        .filter(|run_id| !sampler_child_run_ids.contains(run_id))
        .collect::<Vec<_>>();
    for (child_run_id, node_name) in children_needing_sampler.into_iter().zip(sampler_nodes) {
        store
            .upsert_desired_assignment(node_name, WorkerRole::SamplerAggregator, child_run_id)
            .await?;
        sampler_child_run_ids.insert(child_run_id);
    }

    let sampler_child_run_ids = child_run_ids
        .iter()
        .copied()
        .filter(|run_id| sampler_child_run_ids.contains(run_id))
        .collect::<Vec<_>>();
    if sampler_child_run_ids.is_empty() {
        return Ok(());
    }

    for (index, node_name) in evaluator_nodes.into_iter().enumerate() {
        let child_run_id = sampler_child_run_ids[index % sampler_child_run_ids.len()];
        store
            .upsert_desired_assignment(node_name, WorkerRole::Evaluator, child_run_id)
            .await?;
    }
    Ok(())
}

fn child_sampler_run_ids(nodes: &[RegisteredNode], child_run_ids: &BTreeSet<i32>) -> BTreeSet<i32> {
    nodes
        .iter()
        .flat_map(|node| {
            [
                node.desired_assignment.as_ref(),
                node.current_assignment.as_ref(),
            ]
        })
        .flatten()
        .filter(|assignment| {
            assignment.role == WorkerRole::SamplerAggregator
                && child_run_ids.contains(&assignment.run_id)
        })
        .map(|assignment| assignment.run_id)
        .collect()
}

fn idle_node_assignments_from_nodes(nodes: &[RegisteredNode]) -> Vec<DesiredAssignment> {
    let mut idle_nodes = nodes
        .iter()
        .filter(|node| node.desired_assignment.is_none() && node.current_assignment.is_none())
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();
    idle_nodes.sort();
    let sampler_count = usize::from(!idle_nodes.is_empty());
    let mut assignments = Vec::new();
    for node_name in idle_nodes.iter().take(sampler_count) {
        assignments.push(DesiredAssignment {
            node_name: node_name.clone(),
            role: WorkerRole::SamplerAggregator,
            run_id: 0,
            run_name: None,
        });
    }
    for node_name in idle_nodes.into_iter().skip(sampler_count) {
        assignments.push(DesiredAssignment {
            node_name,
            role: WorkerRole::Evaluator,
            run_id: 0,
            run_name: None,
        });
    }
    assignments
}

pub async fn load_child_task_measurement(
    store: &(impl RunReadStore + RunTaskStore),
    child_run_id: i32,
    source_task: &str,
) -> Result<ChildTaskMeasurement, StoreError> {
    let output = load_task_measurement_output(store, child_run_id, source_task)
        .await
        .store_err()?;
    Ok(ChildTaskMeasurement {
        task_state: output.task_state,
        output: output.output,
    })
}
