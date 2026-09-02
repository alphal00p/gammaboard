use crate::api::runs::{ChildRunRequest, create_child_run};
use crate::core::StoreResultExt;
use crate::core::{
    AggregationStore, ControlPlaneStore, ControllerChildOutput, ControllerChildState,
    ControllerTaskOutput, ParameterScanOutput, ParameterScanPointOutput, RunReadStore,
    RunSpecStore, RunTask, RunTaskSpec, RunTaskStore, StoreError, TaskMeasurementOutput,
};
use crate::runners::controller_child::{
    ControllerAssignmentPlan, apply_controller_assignment_plan, load_child_task_measurement,
};
use crate::runners::parameter_grid::{ParameterGridItem, cartesian_grid_len, cartesian_grid_point};
use std::collections::BTreeMap;

const PARAMETER_SCAN_SPAWN_KIND: &str = "parameter_scan";

pub struct ParameterScanRunner<S> {
    store: S,
    run_id: i32,
    task: RunTask,
}

impl<S> ParameterScanRunner<S> {
    pub fn new(store: S, run_id: i32, task: RunTask) -> Self {
        Self {
            store,
            run_id,
            task,
        }
    }
}

impl<S> ParameterScanRunner<S>
where
    S: AggregationStore
        + ControlPlaneStore
        + RunReadStore
        + RunSpecStore
        + RunTaskStore
        + Send
        + Sync,
{
    pub async fn tick(&mut self) -> Result<bool, StoreError> {
        let RunTaskSpec::ParameterScan {
            parameters,
            measurement,
            trial_run_toml,
            max_concurrent_runs,
        } = &self.task.task
        else {
            return Err(StoreError::store("parameter scan runner got non-scan task"));
        };
        let grid_items = parameter_scan_grid_items(parameters)?;
        let total_points = cartesian_grid_len(&grid_items)?;

        let child_runs = self
            .store
            .get_child_runs_for_task(self.run_id, self.task.id, PARAMETER_SCAN_SPAWN_KIND)
            .await?;
        let child_runs_by_label = child_runs
            .iter()
            .filter_map(|run| run.spawn_label.as_deref().map(|label| (label, run)))
            .collect::<BTreeMap<_, _>>();
        let mut points = Vec::with_capacity(total_points);
        let mut running_count = 0usize;
        let mut completed_count = 0usize;
        let mut failed_reason = None;

        for index in 0..total_points {
            let index_label = index.to_string();
            let child = child_runs_by_label.get(index_label.as_str()).copied();
            let replacements = cartesian_grid_point(&grid_items, index)?;
            let parameter_values = serde_json::to_value(&replacements).map_err(|err| {
                StoreError::store(format!("failed to serialize parameter values: {err}"))
            })?;

            let Some(child) = child else {
                points.push(ParameterScanPointOutput {
                    index,
                    parameter_values,
                    child: ControllerChildOutput {
                        child_run_id: None,
                        status: ControllerChildState::Pending,
                        measurement: None,
                        failure_reason: None,
                    },
                });
                continue;
            };

            let measurement_output =
                load_child_task_measurement(&self.store, child.run_id, &measurement.source_task)
                    .await?;
            match measurement_output.output {
                Some(TaskMeasurementOutput::Completed { results }) => {
                    completed_count += 1;
                    points.push(ParameterScanPointOutput {
                        index,
                        parameter_values,
                        child: ControllerChildOutput {
                            child_run_id: Some(child.run_id),
                            status: ControllerChildState::Completed,
                            measurement: Some(TaskMeasurementOutput::Completed { results }),
                            failure_reason: None,
                        },
                    });
                }
                Some(TaskMeasurementOutput::Failed { reason }) => {
                    failed_reason = Some(format!(
                        "scan point {index} child run {} measurement failed: {reason}",
                        child.run_id
                    ));
                    points.push(ParameterScanPointOutput {
                        index,
                        parameter_values,
                        child: ControllerChildOutput {
                            child_run_id: Some(child.run_id),
                            status: ControllerChildState::Failed,
                            measurement: Some(TaskMeasurementOutput::Failed {
                                reason: reason.clone(),
                            }),
                            failure_reason: Some(reason),
                        },
                    });
                }
                None => {
                    if measurement_output.task_state != crate::core::RunTaskState::Completed {
                        running_count += 1;
                    }
                    points.push(ParameterScanPointOutput {
                        index,
                        parameter_values,
                        child: ControllerChildOutput {
                            child_run_id: Some(child.run_id),
                            status: measurement_output.task_state.into(),
                            measurement: None,
                            failure_reason: None,
                        },
                    });
                }
            }
        }

        if let Some(reason) = failed_reason {
            self.persist_output(
                parameter_scan_names(&parameters),
                completed_count,
                running_count,
                points,
            )
            .await?;
            apply_controller_assignment_plan(
                &self.store,
                ControllerAssignmentPlan::preserving(self.run_id, Vec::new()),
            )
            .await?;
            self.store.fail_run_task(self.task.id, &reason).await?;
            return Ok(true);
        }

        if completed_count == total_points {
            self.persist_output(
                parameter_scan_names(&parameters),
                completed_count,
                running_count,
                points,
            )
            .await?;
            apply_controller_assignment_plan(
                &self.store,
                ControllerAssignmentPlan::preserving(self.run_id, Vec::new()),
            )
            .await?;
            self.store.complete_run_task(self.task.id).await?;
            return Ok(true);
        }

        let mut created_child_run_ids = Vec::new();
        let mut capacity = max_concurrent_runs.saturating_sub(running_count);
        if capacity > 0 {
            for index in 0..total_points {
                if capacity == 0 {
                    break;
                }
                let index_label = index.to_string();
                if child_runs_by_label.contains_key(index_label.as_str()) {
                    continue;
                }
                let replacements = cartesian_grid_point(&grid_items, index)?;
                let child = create_child_run(
                    &self.store,
                    ChildRunRequest {
                        parent_run_id: self.run_id,
                        parent_task_id: Some(self.task.id),
                        spawn_kind: PARAMETER_SCAN_SPAWN_KIND.to_string(),
                        spawn_label: Some(index_label),
                        run_toml: trial_run_toml.clone(),
                        replacements,
                    },
                )
                .await
                .store_err()?;
                created_child_run_ids.push(child.run_id);
                capacity -= 1;
            }
        }

        let mut runnable_child_run_ids = points
            .iter()
            .filter(|point| {
                !matches!(
                    point.child.status,
                    ControllerChildState::Completed | ControllerChildState::Failed
                )
            })
            .filter_map(|point| point.child.child_run_id)
            .collect::<Vec<_>>();
        runnable_child_run_ids.extend(created_child_run_ids);
        apply_controller_assignment_plan(
            &self.store,
            ControllerAssignmentPlan::preserving(self.run_id, runnable_child_run_ids),
        )
        .await?;

        self.persist_output(
            parameter_scan_names(&parameters),
            completed_count,
            running_count,
            points,
        )
        .await?;

        Ok(false)
    }

    async fn persist_output(
        &self,
        parameters: Vec<String>,
        completed_points: usize,
        running_points: usize,
        points: Vec<ParameterScanPointOutput>,
    ) -> Result<(), StoreError> {
        let output = ControllerTaskOutput::ParameterScan(ParameterScanOutput {
            total_points: points.len(),
            parameters,
            completed_points,
            running_points,
            points,
        });
        self.store
            .persist_task_controller_output(self.task.id, &output)
            .await
    }
}

fn parameter_scan_grid_items(
    parameters: &[crate::core::ParameterScanParameterSpec],
) -> Result<Vec<ParameterGridItem>, StoreError> {
    parameters
        .iter()
        .map(|parameter| {
            Ok(ParameterGridItem {
                name: parameter.name.clone(),
                values: parameter.values().map_err(StoreError::store)?,
            })
        })
        .collect()
}

fn parameter_scan_names(parameters: &[crate::core::ParameterScanParameterSpec]) -> Vec<String> {
    parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect()
}
