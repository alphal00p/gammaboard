use crate::core::StoreResultExt;
use crate::core::{
    AggregationStore, ControlPlaneStore, DesiredAssignment, RegisteredNode, ResultSourceRef,
    RunReadStore, RunTaskState, RunTaskStore, StoreError, TaskMeasurementOutput, WorkerRole,
};
use crate::evaluation::AccumulatorState;
use crate::services::measurement::load_task_measurement_output;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct ChildTaskResult {
    pub task_state: RunTaskState,
    pub output: Option<TaskMeasurementOutput>,
    pub source: ResultSourceRef,
    pub accumulator: Option<AccumulatorState>,
    pub source_task: crate::core::RunTask,
}

impl ChildTaskResult {
    pub fn task_failure_reason(&self) -> Option<String> {
        task_failure_reason(
            self.task_state,
            &self.source_task.name,
            self.source_task.failure_reason.as_deref(),
        )
    }
}

fn task_failure_reason(
    state: RunTaskState,
    task_name: &str,
    failure_reason: Option<&str>,
) -> Option<String> {
    (state == RunTaskState::Failed).then(|| {
        failure_reason
            .map(str::to_string)
            .unwrap_or_else(|| format!("child task '{task_name}' failed"))
    })
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
    // Plan from one snapshot, then publish only actual changes in one transaction.
    // A stale snapshot is harmless: the next controller tick retries it.
    let nodes = store.list_nodes(None).await?;
    let updates = controller_assignment_updates(&nodes, plan);
    store.update_desired_assignments(&updates).await?;
    Ok(())
}

fn controller_assignment_updates(
    nodes: &[RegisteredNode],
    plan: ControllerAssignmentPlan,
) -> Vec<crate::core::NodeAssignmentUpdate> {
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
    let reusable_assignments = nodes
        .iter()
        .filter_map(|node| node.desired_assignment.as_ref())
        .filter(|assignment| {
            assignment.run_id == plan.parent_run_id
                || (!plan.preserve_selected_assignments && managed.contains(&assignment.run_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut desired = nodes
        .iter()
        .filter_map(|node| {
            node.desired_assignment
                .as_ref()
                .map(|a| (node.name.clone(), a.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    for assignment in &reusable_assignments {
        desired.remove(&assignment.node_name);
    }
    if !selected.is_empty() {
        let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
        let mut activated = if plan.preserve_selected_assignments {
            child_sampler_run_ids(nodes, &selected_set)
        } else {
            BTreeSet::new()
        };
        // Only explicitly assigned nodes belong to this controller. Taking all
        // idle nodes would undo operator unassign/pause and steal other pools.
        if !plan.preserve_selected_assignments {
            // Keep samplers on children that are still selected, even when
            // priority order changes or another child leaves the selected set.
            for assignment in &reusable_assignments {
                if assignment.role == WorkerRole::SamplerAggregator
                    && selected_set.contains(&assignment.run_id)
                    && activated.insert(assignment.run_id)
                {
                    desired.insert(assignment.node_name.clone(), assignment.clone());
                }
            }
        }
        let mut sampler_nodes = reusable_assignments
            .iter()
            .filter(|a| {
                a.role == WorkerRole::SamplerAggregator && !desired.contains_key(&a.node_name)
            })
            .map(|a| a.node_name.as_str())
            .collect::<Vec<_>>();
        let mut evaluator_nodes = reusable_assignments
            .iter()
            .filter(|a| a.role == WorkerRole::Evaluator)
            .map(|a| a.node_name.as_str())
            .collect::<Vec<_>>();
        sampler_nodes.sort_unstable();
        evaluator_nodes.sort_unstable();
        let children_needing_sampler = selected
            .iter()
            .copied()
            .filter(|run_id| !activated.contains(run_id))
            .collect::<Vec<_>>();
        for (run_id, node_name) in children_needing_sampler.into_iter().zip(sampler_nodes) {
            desired.insert(
                node_name.to_owned(),
                DesiredAssignment {
                    node_name: node_name.to_owned(),
                    role: WorkerRole::SamplerAggregator,
                    run_id,
                    run_name: None,
                },
            );
            activated.insert(run_id);
        }
        // Stable evaluator distribution depends on membership, not priority.
        let activated = activated.into_iter().collect::<Vec<_>>();
        if !activated.is_empty() {
            for (index, node_name) in evaluator_nodes.into_iter().enumerate() {
                desired.insert(
                    node_name.to_owned(),
                    DesiredAssignment {
                        node_name: node_name.to_owned(),
                        role: WorkerRole::Evaluator,
                        run_id: activated[index % activated.len()],
                        run_name: None,
                    },
                );
            }
        }
    }
    if !selected.is_empty() {
        // Retain pool ownership while waiting for a sampler or for capacity in
        // another child. Freeing these nodes would lose them on the next tick.
        for assignment in &reusable_assignments {
            desired
                .entry(assignment.node_name.clone())
                .or_insert_with(|| DesiredAssignment {
                    run_id: plan.parent_run_id,
                    run_name: None,
                    ..assignment.clone()
                });
        }
    }
    nodes
        .iter()
        .filter_map(|node| {
            let next = desired.remove(&node.name);
            let target = |a: &DesiredAssignment| (a.role, a.run_id);
            (node.desired_assignment.as_ref().map(target) != next.as_ref().map(target)).then(|| {
                crate::core::NodeAssignmentUpdate {
                    node_uuid: node.uuid.clone(),
                    expected: node.desired_assignment.clone(),
                    desired: next,
                }
            })
        })
        .collect()
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

/// Resolve the newest usable publishing stage, retaining a previous result
/// while the next stage initializes. Lifecycle follows the entire child queue.
pub async fn load_published_child_result(
    store: &(impl AggregationStore + RunReadStore + RunTaskStore),
    run_id: i32,
) -> Result<(ChildTaskResult, bool, i64), StoreError> {
    let tasks = store.list_run_tasks(run_id).await?;
    let publishing = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.task,
                crate::core::RunTaskSpec::Sample {
                    publish_result: true,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    let final_task = publishing.last().ok_or_else(|| {
        StoreError::store(format!(
            "campaign child {run_id} has no publishing sample task"
        ))
    })?;
    let work_samples = tasks.iter().map(|task| task.nr_completed_samples).sum();
    let mut selected = None;
    for task in publishing
        .iter()
        .rev()
        .filter(|task| matches!(task.state, RunTaskState::Active | RunTaskState::Completed))
    {
        let result = load_child_task_result(store, run_id, &task.name).await?;
        if (task.state == RunTaskState::Completed || task.nr_completed_samples > 0)
            && result
                .accumulator
                .as_ref()
                .is_some_and(|a| a.sample_count() > 0)
        {
            selected = Some(result);
            break;
        }
    }
    let mut result = match selected {
        Some(result) => result,
        None => load_child_task_result(store, run_id, &final_task.name).await?,
    };
    let ready = result.source.task_id == final_task.id
        && matches!(
            final_task.state,
            RunTaskState::Active | RunTaskState::Completed
        )
        && result
            .accumulator
            .as_ref()
            .is_some_and(|a| a.sample_count() > 1);
    // Earlier failures must not leave a pending result stage scheduled forever.
    if let Some(failed) = tasks.iter().find(|task| task.state == RunTaskState::Failed) {
        result.task_state = RunTaskState::Failed;
        result.output = Some(TaskMeasurementOutput::Failed {
            reason: failed
                .failure_reason
                .clone()
                .unwrap_or_else(|| format!("task '{}' failed", failed.name)),
        });
    } else {
        result.task_state = if tasks
            .iter()
            .all(|task| task.state == RunTaskState::Completed)
        {
            RunTaskState::Completed
        } else {
            RunTaskState::Active
        };
    }
    Ok((result, ready, work_samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, role: WorkerRole, run_id: i32) -> RegisteredNode {
        RegisteredNode {
            name: name.into(),
            uuid: name.into(),
            capabilities: Default::default(),
            desired_assignment: Some(DesiredAssignment {
                node_name: name.into(),
                role,
                run_id,
                run_name: None,
            }),
            current_assignment: None,
            last_seen: None,
        }
    }

    #[test]
    fn unchanged_campaign_selection_does_not_reassign_workers() {
        let nodes = vec![
            node("s", WorkerRole::SamplerAggregator, 2),
            node("e1", WorkerRole::Evaluator, 2),
            node("e2", WorkerRole::Evaluator, 2),
        ];
        assert!(
            controller_assignment_updates(
                &nodes,
                ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![2])
            )
            .is_empty()
        );
    }

    #[test]
    fn selection_priority_changes_do_not_swap_worker_pools() {
        let nodes = vec![
            node("s1", WorkerRole::SamplerAggregator, 2),
            node("s2", WorkerRole::SamplerAggregator, 3),
            node("e1", WorkerRole::Evaluator, 2),
            node("e2", WorkerRole::Evaluator, 3),
        ];
        assert!(
            controller_assignment_updates(
                &nodes,
                ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![3, 2, 3])
            )
            .is_empty()
        );
    }

    #[test]
    fn campaign_respects_unassigned_and_unrelated_idle_workers() {
        let mut idle = node("idle", WorkerRole::Evaluator, 2);
        idle.desired_assignment = None;
        let nodes = vec![node("sampler", WorkerRole::SamplerAggregator, 2), idle];
        assert!(
            controller_assignment_updates(
                &nodes,
                ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![2])
            )
            .is_empty()
        );
    }

    #[test]
    fn retained_child_keeps_its_sampler_when_another_child_leaves() {
        let nodes = vec![
            node("s1", WorkerRole::SamplerAggregator, 2),
            node("s2", WorkerRole::SamplerAggregator, 3),
        ];
        let updates = controller_assignment_updates(
            &nodes,
            ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![3]),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].node_uuid, "s1");
        assert_eq!(updates[0].desired.as_ref().unwrap().run_id, 1);
    }

    #[test]
    fn new_samplers_follow_selection_priority() {
        let nodes = vec![node("s", WorkerRole::SamplerAggregator, 1)];
        let updates = controller_assignment_updates(
            &nodes,
            ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![3, 2]),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].desired.as_ref().unwrap().run_id, 3);
    }

    #[test]
    fn campaign_moves_only_its_pool_and_releases_it_on_completion() {
        let nodes = vec![
            node("s", WorkerRole::SamplerAggregator, 2),
            node("e", WorkerRole::Evaluator, 2),
            node("other", WorkerRole::Evaluator, 9),
        ];
        let moved = controller_assignment_updates(
            &nodes,
            ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![3]),
        );
        assert_eq!(moved.len(), 2);
        assert!(
            moved
                .iter()
                .all(|u| u.expected.as_ref().unwrap().run_id == 2
                    && u.desired.as_ref().unwrap().run_id == 3)
        );
        let stopped = controller_assignment_updates(
            &nodes,
            ControllerAssignmentPlan::replacing(1, vec![2, 3], vec![]),
        );
        assert_eq!(stopped.len(), 2);
        assert!(stopped.iter().all(|u| u.desired.is_none()));
    }

    #[test]
    fn preserving_controller_keeps_children_and_assigns_new_parent_workers() {
        let nodes = vec![
            node("s1", WorkerRole::SamplerAggregator, 2),
            node("e1", WorkerRole::Evaluator, 2),
            node("s2", WorkerRole::SamplerAggregator, 1),
            node("e2", WorkerRole::Evaluator, 1),
        ];
        let updates = controller_assignment_updates(
            &nodes,
            ControllerAssignmentPlan::preserving(1, vec![2, 3]),
        );
        assert_eq!(updates.len(), 2);
        assert!(
            updates
                .iter()
                .all(|u| u.expected.as_ref().unwrap().run_id == 1)
        );
        assert_eq!(
            updates
                .iter()
                .find(|u| u.node_uuid == "s2")
                .unwrap()
                .desired
                .as_ref()
                .unwrap()
                .run_id,
            3
        );
    }

    #[test]
    fn failed_child_task_overrides_cached_measurement_state() {
        assert_eq!(
            task_failure_reason(RunTaskState::Failed, "integrate", Some("engine stopped")),
            Some("engine stopped".to_string())
        );
        assert_eq!(
            task_failure_reason(RunTaskState::Active, "integrate", Some("stale")),
            None
        );
    }
}
