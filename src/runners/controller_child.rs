use crate::api::measurement::load_task_measurement_output;
use crate::core::StoreResultExt;
use crate::core::{
    AggregationStore, ControlPlaneStore, DesiredAssignment, RegisteredNode, ResultSourceRef,
    RunReadStore, RunTaskState, RunTaskStore, StoreError, TaskMeasurementOutput, WorkerRole,
};
use crate::evaluation::AccumulatorState;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct ChildTaskResult {
    pub task_state: RunTaskState,
    pub output: Option<TaskMeasurementOutput>,
    pub source: ResultSourceRef,
    pub accumulator: Option<AccumulatorState>,
    pub source_task: crate::core::RunTask,
}

#[derive(Debug, Clone)]
pub struct ControllerAssignmentPlan {
    pub parent_run_id: i32,
    pub managed_child_run_ids: Vec<i32>,
    pub selected_child_run_ids: Vec<i32>,
    pub preserve_selected_assignments: bool,
}

impl ControllerAssignmentPlan {
    pub fn preserving(parent_run_id: i32, selected_child_run_ids: Vec<i32>) -> Self {
        Self {
            parent_run_id,
            managed_child_run_ids: selected_child_run_ids.clone(),
            selected_child_run_ids,
            preserve_selected_assignments: true,
        }
    }

    pub fn replacing(
        parent_run_id: i32,
        managed_child_run_ids: Vec<i32>,
        selected_child_run_ids: Vec<i32>,
    ) -> Self {
        Self {
            parent_run_id,
            managed_child_run_ids,
            selected_child_run_ids,
            preserve_selected_assignments: false,
        }
    }
}

pub async fn apply_controller_assignment_plan(
    store: &impl ControlPlaneStore,
    plan: ControllerAssignmentPlan,
) -> Result<(), StoreError> {
    let managed = plan
        .managed_child_run_ids
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut selected_seen = BTreeSet::new();
    let selected = plan
        .selected_child_run_ids
        .into_iter()
        .filter(|run_id| managed.contains(run_id) && selected_seen.insert(*run_id))
        .collect::<Vec<_>>();
    let all_assignments = store.list_desired_assignments(None).await?;
    let reusable_assignments = all_assignments
        .into_iter()
        .filter(|assignment| {
            assignment.run_id == plan.parent_run_id
                || (!plan.preserve_selected_assignments && managed.contains(&assignment.run_id))
        })
        .collect::<Vec<_>>();

    store
        .clear_desired_assignments_for_run(plan.parent_run_id)
        .await?;
    if !plan.preserve_selected_assignments {
        for run_id in &managed {
            store.clear_desired_assignments_for_run(*run_id).await?;
        }
    }
    if selected.is_empty() {
        return Ok(());
    }

    let nodes = store.list_nodes(None).await?;
    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    let mut activated = if plan.preserve_selected_assignments {
        child_sampler_run_ids(&nodes, &selected_set)
    } else {
        BTreeSet::new()
    };
    let reusable_assignments = if reusable_assignments.is_empty() {
        idle_node_assignments_from_nodes(&nodes)
    } else {
        reusable_assignments
    };
    let mut sampler_nodes = reusable_assignments
        .iter()
        .filter(|assignment| assignment.role == WorkerRole::SamplerAggregator)
        .map(|assignment| assignment.node_name.as_str())
        .collect::<Vec<_>>();
    let evaluator_nodes = reusable_assignments
        .iter()
        .filter(|assignment| assignment.role == WorkerRole::Evaluator)
        .map(|assignment| assignment.node_name.as_str())
        .collect::<Vec<_>>();
    sampler_nodes.sort_unstable();

    let children_needing_sampler = selected
        .iter()
        .copied()
        .filter(|run_id| !activated.contains(run_id))
        .collect::<Vec<_>>();
    for (run_id, node_name) in children_needing_sampler.into_iter().zip(sampler_nodes) {
        store
            .upsert_desired_assignment(node_name, WorkerRole::SamplerAggregator, run_id)
            .await?;
        activated.insert(run_id);
    }
    let activated = selected
        .iter()
        .copied()
        .filter(|run_id| activated.contains(run_id))
        .collect::<Vec<_>>();
    if activated.is_empty() {
        return Ok(());
    }
    for (index, node_name) in evaluator_nodes.into_iter().enumerate() {
        store
            .upsert_desired_assignment(
                node_name,
                WorkerRole::Evaluator,
                activated[index % activated.len()],
            )
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

pub async fn load_child_task_result(
    store: &(impl AggregationStore + RunReadStore + RunTaskStore),
    child_run_id: i32,
    source_task: &str,
) -> Result<ChildTaskResult, StoreError> {
    load_child_task_result_inner(store, child_run_id, source_task, true).await
}

pub async fn load_child_task_result_reference(
    store: &(impl AggregationStore + RunReadStore + RunTaskStore),
    child_run_id: i32,
    source_task: &str,
) -> Result<ChildTaskResult, StoreError> {
    load_child_task_result_inner(store, child_run_id, source_task, false).await
}

async fn load_child_task_result_inner(
    store: &(impl AggregationStore + RunReadStore + RunTaskStore),
    child_run_id: i32,
    source_task: &str,
    include_accumulator: bool,
) -> Result<ChildTaskResult, StoreError> {
    let output = load_task_measurement_output(store, child_run_id, source_task)
        .await
        .store_err()?;
    let (accumulator, snapshot_id) = if !include_accumulator {
        let snapshot_id = if output.task_state == RunTaskState::Active {
            None
        } else {
            store
                .get_latest_task_stage_snapshot_id(child_run_id, output.task_id)
                .await?
        };
        (None, snapshot_id)
    } else if output.task_state == RunTaskState::Active {
        let accumulator = store
            .load_current_accumulator(child_run_id)
            .await?
            .map(|value| AccumulatorState::from_json(&value))
            .transpose()
            .map_err(|err| StoreError::store(err.to_string()))?;
        (accumulator, None)
    } else {
        let snapshot = store
            .get_latest_task_stage_snapshot(child_run_id, output.task_id)
            .await?;
        let snapshot_id = snapshot.as_ref().map(|snapshot| snapshot.id.clone());
        (
            snapshot.map(|snapshot| snapshot.observable_state),
            snapshot_id,
        )
    };
    let sample_count = accumulator
        .as_ref()
        .map(AccumulatorState::sample_count)
        .or_else(|| {
            output
                .output
                .as_ref()
                .and_then(|measurement| match measurement {
                    TaskMeasurementOutput::Completed { results } => {
                        results.iter().map(|result| result.sample_count).max()
                    }
                    TaskMeasurementOutput::Failed { .. } => None,
                })
        })
        .unwrap_or(output.source_task.nr_completed_samples);
    Ok(ChildTaskResult {
        task_state: output.task_state,
        output: output.output,
        source: ResultSourceRef {
            run_id: child_run_id,
            task_id: output.task_id,
            snapshot_id,
            sample_count,
        },
        accumulator,
        source_task: output.source_task,
    })
}
