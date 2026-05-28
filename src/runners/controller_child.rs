use crate::api::{
    ApiError,
    measurement::load_task_measurement_output,
    runs::{ChildRunRequest, CreatedRun, create_child_run},
};
use crate::core::{
    AggregationStore, ControlPlaneStore, RunReadStore, RunTaskState, RunTaskStore, StoreError,
    TaskMeasurementOutput, WorkerRole,
};
use crate::stores::RunProgress;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct ControllerChildRunRequest {
    pub parent_run_id: i32,
    pub parent_task_id: i64,
    pub spawn_kind: String,
    pub spawn_label: String,
    pub run_toml: String,
    pub replacements: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
pub struct ChildTaskMeasurement {
    pub task_state: RunTaskState,
    pub output: Option<TaskMeasurementOutput>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControllerProgressSummary {
    pub total: usize,
    pub planned: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
}

pub async fn list_child_runs_for_task(
    store: &impl RunReadStore,
    parent_run_id: i32,
    parent_task_id: i64,
    spawn_kind: &str,
) -> Result<Vec<RunProgress>, StoreError> {
    Ok(store
        .get_all_runs()
        .await?
        .into_iter()
        .filter(|run| {
            run.parent_run_id == Some(parent_run_id)
                && run.parent_task_id.as_deref() == Some(&parent_task_id.to_string())
                && run.spawn_kind.as_deref() == Some(spawn_kind)
        })
        .collect())
}

pub async fn create_controller_child_run(
    store: &(impl ControlPlaneStore + AggregationStore + RunTaskStore),
    request: ControllerChildRunRequest,
) -> Result<CreatedRun, ApiError> {
    create_child_run(
        store,
        ChildRunRequest {
            parent_run_id: request.parent_run_id,
            parent_task_id: Some(request.parent_task_id),
            spawn_kind: request.spawn_kind,
            spawn_label: Some(request.spawn_label),
            run_toml: request.run_toml,
            replacements: request.replacements,
        },
    )
    .await
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
    let child_run_ids = child_run_ids.into_iter().collect::<Vec<_>>();

    store
        .clear_desired_assignments_for_run(parent_run_id)
        .await?;
    if child_run_ids.is_empty() || parent_assignments.is_empty() {
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
    for (child_run_id, node_name) in child_run_ids.iter().copied().zip(sampler_nodes) {
        store
            .upsert_desired_assignment(node_name, WorkerRole::SamplerAggregator, child_run_id)
            .await?;
    }

    let sampler_capacity = child_run_ids.len().min(
        parent_assignments
            .iter()
            .filter(|assignment| assignment.role == WorkerRole::SamplerAggregator)
            .count(),
    );
    if sampler_capacity == 0 {
        return Ok(());
    }

    for (index, node_name) in evaluator_nodes.into_iter().enumerate() {
        let child_run_id = child_run_ids[index % sampler_capacity];
        store
            .upsert_desired_assignment(node_name, WorkerRole::Evaluator, child_run_id)
            .await?;
    }
    Ok(())
}

pub async fn load_child_task_measurement(
    store: &(impl RunReadStore + RunTaskStore),
    child_run_id: i32,
    source_task: &str,
) -> Result<ChildTaskMeasurement, StoreError> {
    let output = load_task_measurement_output(store, child_run_id, source_task)
        .await
        .map_err(|err| StoreError::store(err.to_string()))?;
    Ok(ChildTaskMeasurement {
        task_state: output.task_state,
        output: output.output,
    })
}

pub fn controller_progress_summary<'a>(
    statuses: impl IntoIterator<Item = &'a str>,
) -> ControllerProgressSummary {
    let mut summary = ControllerProgressSummary::default();
    for status in statuses {
        summary.total += 1;
        match status {
            "pending" | "planned" => summary.planned += 1,
            "completed" => summary.completed += 1,
            "failed" => summary.failed += 1,
            _ => summary.running += 1,
        }
    }
    summary
}

pub fn choose_child_capacity(max_concurrent_children: usize, running_children: usize) -> usize {
    max_concurrent_children.saturating_sub(running_children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_summary_counts_controller_statuses() {
        let summary = controller_progress_summary([
            "pending",
            "planned",
            "active",
            "running",
            "completed",
            "failed",
        ]);

        assert_eq!(
            summary,
            ControllerProgressSummary {
                total: 6,
                planned: 2,
                running: 2,
                completed: 1,
                failed: 1,
            }
        );
    }

    #[test]
    fn child_capacity_saturates_at_zero() {
        assert_eq!(choose_child_capacity(4, 2), 2);
        assert_eq!(choose_child_capacity(2, 4), 0);
    }
}
