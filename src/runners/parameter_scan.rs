use crate::api::{
    measurement::load_task_measurement_output,
    nodes,
    runs::{ChildRunRequest, create_child_run},
};
use crate::core::{
    AggregationStore, ControlPlaneStore, RunReadStore, RunSpecStore, RunTask, RunTaskSpec,
    RunTaskStore, StoreError, TaskMeasurementOutput,
};
use crate::stores::RunProgress;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterScanPointOutput {
    pub index: usize,
    pub parameter_value: JsonValue,
    pub child_run_id: Option<i32>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement: Option<TaskMeasurementOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterScanOutput {
    pub parameter_name: String,
    pub completed_points: usize,
    pub total_points: usize,
    pub points: Vec<ParameterScanPointOutput>,
}

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
        self.store
            .clear_desired_assignments_for_run(self.run_id)
            .await?;

        let RunTaskSpec::ParameterScan {
            parameter,
            measurement,
            trial_run_toml,
            max_concurrent_runs,
        } = &self.task.task
        else {
            return Err(StoreError::store("parameter scan runner got non-scan task"));
        };

        let child_runs = self.child_runs().await?;
        let child_runs_by_label = child_runs
            .iter()
            .filter_map(|run| run.spawn_label.as_deref().map(|label| (label, run)))
            .collect::<BTreeMap<_, _>>();
        let mut points = Vec::with_capacity(parameter.values.len());
        let mut running_count = 0usize;
        let mut completed_count = 0usize;
        let mut failed_reason = None;

        for (index, value) in parameter.values.iter().enumerate() {
            let index_label = index.to_string();
            let child = child_runs_by_label.get(index_label.as_str()).copied();
            let parameter_value = serde_json::to_value(value).map_err(|err| {
                StoreError::store(format!("failed to serialize parameter value: {err}"))
            })?;

            let Some(child) = child else {
                points.push(ParameterScanPointOutput {
                    index,
                    parameter_value,
                    child_run_id: None,
                    status: "pending".to_string(),
                    measurement: None,
                    failure_reason: None,
                });
                continue;
            };

            let measurement_output =
                load_task_measurement_output(&self.store, child.run_id, &measurement.source_task)
                    .await
                    .map_err(|err| StoreError::store(err.to_string()))?;
            match measurement_output.output {
                Some(TaskMeasurementOutput::Completed { results }) => {
                    completed_count += 1;
                    points.push(ParameterScanPointOutput {
                        index,
                        parameter_value,
                        child_run_id: Some(child.run_id),
                        status: "completed".to_string(),
                        measurement: Some(TaskMeasurementOutput::Completed { results }),
                        failure_reason: None,
                    });
                }
                Some(TaskMeasurementOutput::Failed { reason }) => {
                    failed_reason = Some(format!(
                        "scan point {index} child run {} measurement failed: {reason}",
                        child.run_id
                    ));
                    points.push(ParameterScanPointOutput {
                        index,
                        parameter_value,
                        child_run_id: Some(child.run_id),
                        status: "failed".to_string(),
                        measurement: Some(TaskMeasurementOutput::Failed {
                            reason: reason.clone(),
                        }),
                        failure_reason: Some(reason),
                    });
                }
                None => {
                    running_count += 1;
                    points.push(ParameterScanPointOutput {
                        index,
                        parameter_value,
                        child_run_id: Some(child.run_id),
                        status: measurement_output.task_state.as_str().to_string(),
                        measurement: None,
                        failure_reason: None,
                    });
                }
            }
        }

        self.persist_output(parameter.name.clone(), completed_count, points)
            .await?;

        if let Some(reason) = failed_reason {
            self.store.fail_run_task(self.task.id, &reason).await?;
            return Ok(true);
        }

        if completed_count == parameter.values.len() {
            self.store.complete_run_task(self.task.id).await?;
            return Ok(true);
        }

        let mut capacity = max_concurrent_runs.saturating_sub(running_count);
        if capacity > 0 {
            for (index, value) in parameter.values.iter().enumerate() {
                if capacity == 0 {
                    break;
                }
                let index_label = index.to_string();
                if child_runs_by_label.contains_key(index_label.as_str()) {
                    continue;
                }
                let mut replacements = BTreeMap::new();
                replacements.insert(parameter.name.clone(), value.clone());
                let child = create_child_run(
                    &self.store,
                    ChildRunRequest {
                        parent_run_id: self.run_id,
                        parent_task_id: Some(self.task.id),
                        spawn_kind: "parameter_scan".to_string(),
                        spawn_label: Some(index_label),
                        run_toml: trial_run_toml.clone(),
                        replacements,
                    },
                )
                .await
                .map_err(|err| StoreError::store(err.to_string()))?;
                let _ = nodes::auto_assign_run(&self.store, child.run_id, None).await;
                capacity -= 1;
            }
        }

        Ok(false)
    }

    async fn child_runs(&self) -> Result<Vec<RunProgress>, StoreError> {
        Ok(self
            .store
            .get_all_runs()
            .await?
            .into_iter()
            .filter(|run| {
                run.parent_run_id == Some(self.run_id)
                    && run.parent_task_id.as_deref() == Some(&self.task.id.to_string())
                    && run.spawn_kind.as_deref() == Some("parameter_scan")
            })
            .collect())
    }

    async fn persist_output(
        &self,
        parameter_name: String,
        completed_points: usize,
        points: Vec<ParameterScanPointOutput>,
    ) -> Result<(), StoreError> {
        let output = serde_json::to_value(ParameterScanOutput {
            total_points: points.len(),
            parameter_name,
            completed_points,
            points,
        })
        .map_err(|err| StoreError::store(format!("failed to serialize scan output: {err}")))?;
        self.store
            .persist_task_controller_output(self.task.id, &output)
            .await
    }
}
