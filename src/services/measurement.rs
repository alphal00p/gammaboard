use crate::core::{
    AccumulatorMetricName, AccumulatorMetricSelector, AggregationStore, MeasurementResult,
    RunReadStore, RunTask, RunTaskState, RunTaskStore, SamplerRuntimeMetrics,
    TaskMeasurementOutput, TaskMeasurementSpec,
};
use crate::evaluation::{
    AccumulatorMetricValue, AccumulatorState, extract_accumulator_metric_with_runtime,
};
use crate::service_error::ServiceError as ApiError;
use crate::stores::SamplerPerformanceHistoryEntry;

#[derive(Debug, Clone)]
pub struct ExtractedMeasurement {
    pub results: Vec<MeasurementResult>,
}

#[derive(Debug, Clone)]
pub struct PersistedTaskMeasurement {
    pub task_id: i64,
    pub task_state: RunTaskState,
    pub output: Option<TaskMeasurementOutput>,
    pub source_task: RunTask,
}

pub async fn load_task_measurement_output(
    store: &impl RunTaskStore,
    run_id: i32,
    task_name: &str,
) -> Result<PersistedTaskMeasurement, ApiError> {
    let tasks = store.list_run_tasks(run_id).await?;
    let task = resolve_measurement_source_task(&tasks, task_name)?;
    Ok(PersistedTaskMeasurement {
        task_id: task.id,
        task_state: task.state,
        output: task.measurement_output.clone(),
        source_task: task.clone(),
    })
}

#[cfg(test)]
async fn extract_measurement(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    run_id: i32,
    measurement: &crate::core::MeasurementSpec,
) -> Result<ExtractedMeasurement, ApiError> {
    measurement.validate().map_err(ApiError::BadRequest)?;
    let tasks = store.list_run_tasks(run_id).await?;
    let source_task = resolve_measurement_source_task(&tasks, &measurement.source_task)?;
    extract_task_measurement_with_spec(
        store,
        run_id,
        &tasks,
        source_task,
        &measurement.task_measurement(),
    )
    .await
}

pub async fn extract_task_measurement(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    run_id: i32,
    source_task: &RunTask,
) -> Result<ExtractedMeasurement, ApiError> {
    let measurement = source_task
        .task
        .effective_sample_measurement()
        .ok_or_else(|| {
            ApiError::BadRequest("measurement is only supported for sample tasks".to_string())
        })?;
    measurement.validate().map_err(ApiError::BadRequest)?;
    let tasks = store.list_run_tasks(run_id).await?;
    extract_task_measurement_with_spec(store, run_id, &tasks, source_task, &measurement).await
}

async fn extract_task_measurement_with_spec(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    run_id: i32,
    tasks: &[RunTask],
    source_task: &RunTask,
    measurement: &TaskMeasurementSpec,
) -> Result<ExtractedMeasurement, ApiError> {
    let accumulator = load_measurement_accumulator(store, run_id, source_task).await?;
    let throughput =
        load_measurement_throughput(store, run_id, tasks, source_task, measurement).await?;
    let results = project_measurement_results(&accumulator, measurement, throughput, source_task)?;
    Ok(ExtractedMeasurement { results })
}

pub(crate) fn project_measurement_results(
    accumulator: &AccumulatorState,
    measurement: &TaskMeasurementSpec,
    completed_samples_per_second: Option<f64>,
    source_task: &RunTask,
) -> Result<Vec<MeasurementResult>, ApiError> {
    Ok(extract_measurement_metrics(
        accumulator,
        measurement,
        completed_samples_per_second,
        source_task,
    )?
    .into_iter()
    .map(measurement_result_from_metric)
    .collect())
}

/// Keep absolute diagnostics separate from signed campaign measurements and allocation.
pub(crate) fn absolute_component_results(
    accumulator: &AccumulatorState,
    results: &[MeasurementResult],
) -> Vec<MeasurementResult> {
    results
        .iter()
        .filter(|result| result.name == AccumulatorMetricName::Mean)
        .filter_map(|result| {
            crate::evaluation::extract_accumulator_metric(
                accumulator,
                &AccumulatorMetricSelector {
                    name: AccumulatorMetricName::AbsMean,
                    component: result.component.clone(),
                },
            )
            .ok()
            .flatten()
            .map(measurement_result_from_metric)
        })
        .collect()
}

/// Older controller outputs did not persist absolute means. Recover them only
/// from the same child task and sample revision as the signed measurement.
pub(crate) async fn hydrate_campaign_absolute_results(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    task: &mut RunTask,
) -> Result<(), ApiError> {
    let Some(crate::core::ControllerTaskOutput::IntegrationCampaign(output)) =
        task.controller_output.as_mut()
    else {
        return Ok(());
    };
    for child in &mut output.children {
        if child.absolute_results.is_some() {
            continue;
        }
        let Some(TaskMeasurementOutput::Completed { results }) = &child.child.measurement else {
            continue;
        };
        let Some(source) = &child.child.result_source else {
            continue;
        };
        let mut accumulator = if let Some(snapshot_id) = &source.snapshot_id {
            match snapshot_id.parse::<i64>() {
                Ok(id) => store
                    .load_stage_snapshot(id)
                    .await?
                    .filter(|snapshot| {
                        snapshot.run_id == source.run_id && snapshot.task_id == Some(source.task_id)
                    })
                    .and_then(|snapshot| snapshot.observable_state),
                Err(_) => None,
            }
        } else {
            store
                .get_latest_task_stage_snapshot(source.run_id, source.task_id)
                .await?
                .map(|snapshot| snapshot.observable_state)
                .filter(|state| state.sample_count() == source.sample_count)
        };
        if accumulator.is_none() && source.snapshot_id.is_none() {
            let tasks = store.list_run_tasks(source.run_id).await?;
            if tasks
                .iter()
                .any(|task| task.id == source.task_id && task.state == RunTaskState::Active)
            {
                accumulator = store
                    .load_current_accumulator(source.run_id)
                    .await?
                    .map(|json| AccumulatorState::from_json(&json))
                    .transpose()
                    .map_err(|error| ApiError::Internal(error.to_string()))?;
            }
        }
        if let Some(accumulator) =
            accumulator.filter(|state| state.sample_count() == source.sample_count)
        {
            child.absolute_results = Some(absolute_component_results(&accumulator, results));
        }
    }
    Ok(())
}

fn resolve_measurement_source_task<'a>(
    tasks: &'a [RunTask],
    source_task_name: &str,
) -> Result<&'a RunTask, ApiError> {
    let mut matches = tasks.iter().filter(|task| task.name == source_task_name);
    let Some(task) = matches.next() else {
        return Err(ApiError::BadRequest(format!(
            "measurement.source_task '{}' not found",
            source_task_name
        )));
    };
    if matches.next().is_some() {
        return Err(ApiError::BadRequest(format!(
            "measurement.source_task '{}' is ambiguous",
            source_task_name
        )));
    }
    Ok(task)
}

async fn load_measurement_accumulator(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    run_id: i32,
    source_task: &RunTask,
) -> Result<AccumulatorState, ApiError> {
    if source_task.state == RunTaskState::Active
        && let Some(current) = store.load_current_accumulator(run_id).await?
    {
        return AccumulatorState::from_json(&current)
            .map_err(|err| ApiError::Internal(err.to_string()));
    }
    let latest_stage_snapshot = store
        .get_latest_task_stage_snapshot(run_id, source_task.id)
        .await?;
    latest_stage_snapshot
        .map(|snapshot| snapshot.observable_state)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "measurement source task '{}' has no accumulator snapshot available",
                source_task.name
            ))
        })
}

async fn load_measurement_throughput(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    run_id: i32,
    tasks: &[RunTask],
    source_task: &RunTask,
    measurement: &TaskMeasurementSpec,
) -> Result<Option<f64>, ApiError> {
    let selector = measurement.metric_selector();
    if selector.name != AccumulatorMetricName::TimeNormalizedVariance {
        return Ok(None);
    }
    ensure_latest_sample_task(tasks, source_task)?;
    let latest = store
        .get_sampler_performance_history(run_id, 1, None)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "measurement metric {:?} requires sampler runtime throughput, but no sampler performance snapshot is available",
                selector.name
            ))
        })?;
    decode_completed_samples_per_second(&latest)
        .map(Some)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "measurement metric {:?} requires finite completed_samples_per_second",
                selector.name
            ))
        })
}

fn ensure_latest_sample_task(tasks: &[RunTask], source_task: &RunTask) -> Result<(), ApiError> {
    let latest_sample = tasks
        .iter()
        .filter(|task| matches!(task.task, crate::core::RunTaskSpec::Sample { .. }))
        .max_by_key(|task| task.sequence_nr);
    let Some(latest_sample) = latest_sample else {
        return Err(ApiError::BadRequest(
            "time_normalized_variance requires a sample source task".to_string(),
        ));
    };
    if latest_sample.id != source_task.id {
        return Err(ApiError::BadRequest(format!(
            "time_normalized_variance is only available for the latest sample task; '{}' is not the latest sample task",
            source_task.name
        )));
    }
    Ok(())
}

fn decode_completed_samples_per_second(entry: &SamplerPerformanceHistoryEntry) -> Option<f64> {
    let runtime: SamplerRuntimeMetrics =
        serde_json::from_value(entry.runtime_metrics.clone()).ok()?;
    let rate = runtime.completed_samples_per_second;
    if rate.is_finite() && rate > 0.0 {
        Some(rate)
    } else {
        None
    }
}

fn measurement_result_from_metric(metric: AccumulatorMetricValue) -> MeasurementResult {
    MeasurementResult {
        name: metric.name,
        component: metric.component,
        value: metric.value,
        uncertainty: metric.uncertainty,
        sample_count: metric.sample_count,
    }
}

fn extract_measurement_metrics(
    accumulator: &AccumulatorState,
    measurement: &TaskMeasurementSpec,
    completed_samples_per_second: Option<f64>,
    source_task: &RunTask,
) -> Result<Vec<AccumulatorMetricValue>, ApiError> {
    if let Some(selector) = measurement.explicit_metric_selector() {
        return extract_single_metric(
            accumulator,
            &selector,
            completed_samples_per_second,
            source_task,
        )
        .map(|metric| vec![metric]);
    }
    extract_central_value_metrics(accumulator, source_task)
}

fn extract_single_metric(
    accumulator: &AccumulatorState,
    selector: &AccumulatorMetricSelector,
    completed_samples_per_second: Option<f64>,
    source_task: &RunTask,
) -> Result<AccumulatorMetricValue, ApiError> {
    extract_accumulator_metric_with_runtime(accumulator, selector, completed_samples_per_second)
        .map_err(|err| ApiError::Internal(err.to_string()))?
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "measurement metric {} is unavailable for task '{}' in state {}",
                metric_selector_label(selector),
                source_task.name,
                source_task.state.as_str()
            ))
        })
}

fn metric_selector_label(selector: &AccumulatorMetricSelector) -> String {
    match selector.component.as_deref() {
        Some(component) => format!("{:?}(component={component})", selector.name),
        None => format!("{:?}", selector.name),
    }
}

fn extract_central_value_metrics(
    accumulator: &AccumulatorState,
    source_task: &RunTask,
) -> Result<Vec<AccumulatorMetricValue>, ApiError> {
    let selectors = match accumulator {
        AccumulatorState::Vector(vector) => vector
            .components
            .iter()
            .map(|component| AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: Some(component.name.clone()),
            })
            .collect(),
        AccumulatorState::Gammaloop(gammaloop) => gammaloop
            .estimate
            .components
            .iter()
            .map(|component| AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: Some(component.name.clone()),
            })
            .collect(),
        AccumulatorState::Empty(_) | AccumulatorState::FullVector(_) => Vec::new(),
    };
    let mut metrics = Vec::new();
    for selector in selectors {
        metrics.push(extract_single_metric(
            accumulator,
            &selector,
            None,
            source_task,
        )?);
    }
    if metrics.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "central_value measurement is unavailable for task '{}' accumulator {}",
            source_task.name,
            accumulator.kind_str()
        )));
    }
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::{
        AggregationStore, ControlPlaneStore, RunReadStore, RunSpecStore, RunTaskStore,
    };
    use crate::core::{
        AccumulatorConfig, MeasurementMetricSpec, MeasurementMode, MeasurementQuantitySpec,
        MeasurementSpec, RunTaskInput, RunTaskSpec, SampleStopCondition, SamplerPerformanceMetrics,
        TaskMeasurementSpec, TrainingProjection,
    };
    use crate::evaluation::Point;
    use crate::stores::{
        EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress, RuntimeLogPage,
        TaskOutputSnapshot, TaskStageSnapshot,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::{Value as JsonValue, json};
    use std::sync::Arc;

    #[derive(Default, Clone)]
    struct TestStore {
        tasks: Arc<Vec<RunTask>>,
        current_accumulator: Option<JsonValue>,
        latest_stage_snapshot: Option<TaskStageSnapshot>,
        sampler_history: Arc<Vec<SamplerPerformanceHistoryEntry>>,
        stage_snapshot: Option<crate::core::RunStageSnapshot>,
    }

    #[async_trait]
    impl AggregationStore for TestStore {
        async fn load_current_accumulator(
            &self,
            _run_id: i32,
        ) -> Result<Option<JsonValue>, crate::core::StoreError> {
            Ok(self.current_accumulator.clone())
        }
        async fn persist_task_result_snapshot(
            &self,
            _run_id: i32,
            _task_id: i64,
            _result: &JsonValue,
        ) -> Result<i64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn load_sampler_checkpoint(
            &self,
            _run_id: i32,
        ) -> Result<Option<crate::core::SamplerAggregatorCheckpoint>, crate::core::StoreError>
        {
            unreachable!("unused")
        }
        async fn load_stage_snapshot(
            &self,
            snapshot_id: i64,
        ) -> Result<Option<crate::core::RunStageSnapshot>, crate::core::StoreError> {
            Ok(self
                .stage_snapshot
                .clone()
                .filter(|snapshot| snapshot.id == Some(snapshot_id)))
        }
        async fn load_latest_stage_snapshot_before_sequence(
            &self,
            _run_id: i32,
            _sequence_nr: i32,
        ) -> Result<Option<crate::core::RunStageSnapshot>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn load_task_activation_snapshot(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<Option<crate::core::RunStageSnapshot>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn load_run_sample_progress(
            &self,
            _run_id: i32,
        ) -> Result<Option<crate::core::RunSampleProgress>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn save_aggregation(
            &self,
            _run_id: i32,
            _task_id: i64,
            _current_accumulator: &JsonValue,
            _persisted_observable: Option<&JsonValue>,
            _delta_batches_completed: i32,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn save_sampler_checkpoint(
            &self,
            _run_id: i32,
            _checkpoint: &crate::core::SamplerAggregatorCheckpoint,
            _stage: Option<&crate::core::RunStageSnapshot>,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn save_run_sample_progress(
            &self,
            _run_id: i32,
            _nr_produced_samples: i64,
            _nr_completed_samples: i64,
            _sampler_runner_uptime_ms: f64,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn save_run_stage_snapshot(
            &self,
            _snapshot: &crate::core::RunStageSnapshot,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
    }

    #[async_trait]
    impl RunTaskStore for TestStore {
        async fn append_run_tasks(
            &self,
            _run_id: i32,
            _tasks: &[RunTaskInput],
        ) -> Result<Vec<RunTask>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn list_run_tasks(
            &self,
            _run_id: i32,
        ) -> Result<Vec<RunTask>, crate::core::StoreError> {
            Ok(self.tasks.as_ref().clone())
        }
        async fn load_run_task(
            &self,
            _task_id: i64,
        ) -> Result<Option<RunTask>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn remove_pending_run_task(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<bool, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn update_run_task_queue_tuning(
            &self,
            _run_id: i32,
            _task_id: i64,
            _queue_tuning: Option<crate::core::SamplerQueueTuning>,
        ) -> Result<RunTask, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn load_active_run_task(
            &self,
            _run_id: i32,
        ) -> Result<Option<RunTask>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn activate_next_run_task(
            &self,
            _run_id: i32,
        ) -> Result<Option<RunTask>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn update_run_task_progress(
            &self,
            _task_id: i64,
            _nr_produced_samples: i64,
            _nr_completed_samples: i64,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn set_run_task_spawn_origin(
            &self,
            _task_id: i64,
            _spawned_from_snapshot_id: Option<i64>,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn complete_run_task(&self, _task_id: i64) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn persist_task_measurement_output(
            &self,
            _task_id: i64,
            _output: &crate::core::TaskMeasurementOutput,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn persist_task_controller_output(
            &self,
            _task_id: i64,
            _output: &crate::core::ControllerTaskOutput,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn fail_run_task(
            &self,
            _task_id: i64,
            _reason: &str,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
    }

    #[async_trait]
    impl RunReadStore for TestStore {
        async fn health_check(&self) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_all_runs(&self) -> Result<Vec<RunProgress>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_run_progress(
            &self,
            _run_id: i32,
        ) -> Result<Option<RunProgress>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_task_output_snapshots(
            &self,
            _run_id: i32,
            _task_id: i64,
            _after_snapshot_id: Option<i64>,
            _limit: i64,
        ) -> Result<Vec<TaskOutputSnapshot>, crate::core::StoreError> {
            Ok(Vec::new())
        }
        async fn get_latest_task_stage_snapshot(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<Option<TaskStageSnapshot>, crate::core::StoreError> {
            Ok(self.latest_stage_snapshot.clone())
        }
        async fn get_latest_task_stage_snapshot_id(
            &self,
            _run_id: i32,
            _task_id: i64,
        ) -> Result<Option<String>, crate::core::StoreError> {
            Ok(self
                .latest_stage_snapshot
                .as_ref()
                .map(|snapshot| snapshot.id.clone()))
        }
        async fn get_runtime_logs(
            &self,
            _limit: i64,
            _source: Option<&str>,
            _run_id: Option<i32>,
            _include_child_runs: bool,
            _node_name: Option<&str>,
            _node_uuid: Option<&str>,
            _level: Option<&str>,
            _query: Option<&str>,
            _before_id: Option<i64>,
        ) -> Result<RuntimeLogPage, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_registered_workers(
            &self,
            _run_id: Option<i32>,
        ) -> Result<Vec<RegisteredWorkerEntry>, crate::core::StoreError> {
            Ok(Vec::new())
        }
        async fn get_evaluator_performance_history(
            &self,
            _run_id: i32,
            _limit: i64,
            _worker_id: Option<&str>,
        ) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_sampler_performance_history(
            &self,
            _run_id: i32,
            _limit: i64,
            _worker_id: Option<&str>,
        ) -> Result<Vec<SamplerPerformanceHistoryEntry>, crate::core::StoreError> {
            Ok(self.sampler_history.as_ref().clone())
        }
    }

    #[async_trait]
    impl RunSpecStore for TestStore {
        async fn load_run_spec(
            &self,
            _run_id: i32,
        ) -> Result<Option<crate::core::RunSpec>, crate::core::StoreError> {
            unreachable!("unused")
        }
    }

    #[async_trait]
    impl ControlPlaneStore for TestStore {
        async fn try_lock_task_control(
            &self,
        ) -> Result<Option<Box<dyn Send>>, crate::core::StoreError> {
            Ok(Some(Box::new(())))
        }

        async fn update_desired_assignments(
            &self,
            _updates: &[crate::core::NodeAssignmentUpdate],
        ) -> Result<bool, crate::core::StoreError> {
            unreachable!("unused in test")
        }

        async fn assign_worker_pool(
            &self,
            _node_name: &str,
            _role: crate::core::WorkerRole,
            _run_id: i32,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn announce_node(
            &self,
            _node_name: &str,
            _node_uuid: &str,
            _capabilities: &crate::core::NodeCapabilities,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn set_current_assignment(
            &self,
            _node_uuid: &str,
            _role: crate::core::WorkerRole,
            _run_id: i32,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn clear_current_assignment(
            &self,
            _node_uuid: &str,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn clear_desired_assignment(
            &self,
            _node_name: &str,
        ) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn clear_desired_assignments_for_run(
            &self,
            _run_id: i32,
        ) -> Result<u64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn clear_desired_assignments_for_run_except_node(
            &self,
            _run_id: i32,
            _keep_node_name: &str,
        ) -> Result<u64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn clear_all_desired_assignments(&self) -> Result<u64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_desired_assignment(
            &self,
            _node_name: &str,
        ) -> Result<Option<crate::core::DesiredAssignment>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn list_desired_assignments(
            &self,
            _node_name: Option<&str>,
        ) -> Result<Vec<crate::core::DesiredAssignment>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn list_nodes(
            &self,
            _node_name: Option<&str>,
        ) -> Result<Vec<crate::core::RegisteredNode>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn create_node_launch_request(
            &self,
            _backend: &str,
            _requested_count: i32,
            _name_prefix: Option<&str>,
            _args: &JsonValue,
        ) -> Result<crate::core::NodeLaunchRequest, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn claim_external_node_launch_request(
            &self,
        ) -> Result<Option<crate::core::NodeLaunchRequest>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn list_node_launch_requests(
            &self,
        ) -> Result<Vec<crate::core::NodeLaunchRequest>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn update_node_launch_request_state(
            &self,
            _id: i64,
            _state: &str,
            _started_count: i32,
            _result: &JsonValue,
            _error: Option<&str>,
        ) -> Result<crate::core::NodeLaunchRequest, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn count_active_evaluator_nodes(
            &self,
            _run_id: i32,
        ) -> Result<i64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn request_node_shutdown(
            &self,
            _node_name: &str,
        ) -> Result<u64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn request_all_nodes_shutdown(&self) -> Result<u64, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn consume_node_shutdown_request(
            &self,
            _node_uuid: &str,
        ) -> Result<bool, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn expire_node_lease(&self, _node_uuid: &str) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn create_run(
            &self,
            _name: &str,
            _run_toml: &str,
            _provenance: &JsonValue,
            _integration_params: &JsonValue,
            _target: Option<&JsonValue>,
            _domain: &crate::utils::domain::Domain,
            _initial_stage_snapshot: &crate::core::RunStageSnapshot,
            _initial_tasks: &[RunTaskInput],
            _parent: Option<&crate::core::traits::RunParentMetadata>,
        ) -> Result<i32, crate::core::StoreError> {
            unreachable!("unused")
        }

        async fn remove_run(&self, _run_id: i32) -> Result<(), crate::core::StoreError> {
            unreachable!("unused")
        }
    }

    fn sample_task(id: i64, name: &str, sequence_nr: i32) -> RunTask {
        RunTask {
            id,
            run_id: 1,
            name: name.to_string(),
            sequence_nr,
            task: RunTaskSpec::Sample {
                publish_result: true,
                stop_condition: SampleStopCondition {
                    max_samples: Some(100),
                    ..SampleStopCondition::default()
                },
                measurement: None,
                evaluator: None,
                sampler_aggregator: None,
                accumulator: None,
                queue_tuning: None,
                batch_transforms: None,
            },
            spawned_from_snapshot_id: None,
            state: RunTaskState::Completed,
            nr_produced_samples: 4,
            nr_completed_samples: 4,
            nr_produced_samples_including_children: 4,
            nr_completed_samples_including_children: 4,
            cpu_seconds: 0.0,
            cpu_seconds_including_children: 0.0,
            failure_reason: None,
            started_at: None,
            completed_at: None,
            failed_at: None,
            created_at: Utc::now(),
            task_toml: String::new(),
            measurement_output: None,
            controller_output: None,
        }
    }

    fn scalar_state() -> AccumulatorState {
        let config = AccumulatorConfig::Scalar {
            discrete_projections: None,
            moments: crate::core::AccumulatorMomentConfig::MaxOrder4,
        };
        let AccumulatorState::Vector(mut state) = AccumulatorState::from_config(&config) else {
            panic!("expected vector accumulator");
        };
        let point = Point::new(vec![], vec![], 1.0);
        for value in [1.0, 2.0, 4.0, 8.0] {
            state.ingest_vector(&[value], &point).unwrap();
        }
        AccumulatorState::Vector(state)
    }

    fn vector_state() -> AccumulatorState {
        let mut state = crate::evaluation::VectorAccumulatorState::from_config(
            vec!["real".to_string(), "imag".to_string()],
            TrainingProjection::Norm,
            None,
            crate::core::AccumulatorMomentConfig::MaxOrder4,
        );
        let point = Point::new(vec![], vec![], 1.0);
        for values in [[1.0, 0.5], [2.0, 1.0], [4.0, 2.0], [8.0, 4.0]] {
            state.ingest_vector(&values, &point).expect("vector ingest");
        }
        AccumulatorState::Vector(state)
    }

    fn central_value_measurement_spec() -> MeasurementSpec {
        MeasurementSpec {
            source_task: "sample".to_string(),
            quantity: MeasurementQuantitySpec::default(),
            metric: None,
            mode: MeasurementMode::Minimize,
        }
    }

    #[test]
    fn absolute_component_measurements_use_absolute_moments_for_the_error() {
        let mut state = crate::evaluation::VectorAccumulatorState::from_config(
            vec!["real".into(), "imag".into()],
            TrainingProjection::Norm,
            None,
            crate::core::AccumulatorMomentConfig::MaxOrder4,
        );
        let point = Point::new(vec![], vec![], 1.0);
        for values in [[3.0, -2.0], [-12.0, 2.0]] {
            state.ingest_vector(&values, &point).unwrap();
        }
        let accumulator = AccumulatorState::Vector(state);
        let signed = project_measurement_results(
            &accumulator,
            &TaskMeasurementSpec::default(),
            None,
            &sample_task(1, "sample", 1),
        )
        .unwrap();
        let absolute = absolute_component_results(&accumulator, &signed);
        assert_eq!(signed[0].value, -4.5);
        assert_eq!(absolute[0].value, 7.5);
        assert!((absolute[0].uncertainty.unwrap() - 4.5 / 2.0_f64.sqrt()).abs() < 1e-12);
        assert_ne!(absolute[0].uncertainty, signed[0].uncertainty);
        assert_eq!(signed[1].value, 0.0);
        assert_eq!(absolute[1].value, 2.0);
        assert_eq!(absolute[1].uncertainty, Some(0.0));
        assert!(absolute.iter().all(
            |result| result.name == AccumulatorMetricName::AbsMean && result.sample_count == 2
        ));
    }

    fn legacy_campaign_task(snapshot_id: Option<&str>, sample_count: i64) -> RunTask {
        let mut task = sample_task(10, "campaign", 1);
        // Decode a pre-change payload to exercise backwards compatibility too.
        task.controller_output = Some(crate::core::ControllerTaskOutput::IntegrationCampaign(
            serde_json::from_value(serde_json::json!({
                "completed_children": 0, "running_children": 1, "total_children": 1,
                "total_samples": sample_count, "selected_child_run_ids": [1],
                "allocation_started_total_samples": 0, "combined_measurement": null,
                "children": [{
                    "name": "graph", "coefficient": 1.0, "selected": true, "score": null,
                    "child_run_id": 1, "status": "active",
                    "result_source": {"run_id": 1, "task_id": "1", "snapshot_id": snapshot_id, "sample_count": sample_count},
                    "measurement": {"status": "completed", "results": [{"name": "mean", "component": "real", "value": 3.75, "uncertainty": 1.34, "sample_count": sample_count}]}
                }]
            })).unwrap()
        ));
        task
    }

    fn recovered_absolute_results(task: &RunTask) -> Option<&Vec<MeasurementResult>> {
        task.controller_output
            .as_ref()
            .unwrap()
            .integration_campaign()
            .unwrap()
            .children[0]
            .absolute_results
            .as_ref()
    }

    #[tokio::test]
    async fn legacy_campaign_recovers_exact_snapshot_and_rejects_other_revisions() {
        let store = TestStore {
            stage_snapshot: Some(crate::core::RunStageSnapshot {
                id: Some(7),
                run_id: 1,
                task_id: Some(1),
                name: "sample".into(),
                sequence_nr: Some(1),
                queue_empty: true,
                sampler_snapshot: None,
                observable_state: Some(vector_state()),
                evaluator: None,
                sampler_aggregator: None,
                batch_transforms: vec![],
            }),
            ..Default::default()
        };
        let mut task = legacy_campaign_task(Some("7"), 4);
        hydrate_campaign_absolute_results(&store, &mut task)
            .await
            .unwrap();
        assert_eq!(recovered_absolute_results(&task).unwrap()[0].value, 3.75);
        let mut other_revision = legacy_campaign_task(Some("7"), 3);
        hydrate_campaign_absolute_results(&store, &mut other_revision)
            .await
            .unwrap();
        assert!(recovered_absolute_results(&other_revision).is_none());
        let mut other_task = legacy_campaign_task(Some("7"), 4);
        let mut wrong_task_store = store.clone();
        wrong_task_store.stage_snapshot.as_mut().unwrap().task_id = Some(2);
        hydrate_campaign_absolute_results(&wrong_task_store, &mut other_task)
            .await
            .unwrap();
        assert!(recovered_absolute_results(&other_task).is_none());
    }

    #[tokio::test]
    async fn legacy_live_campaign_requires_matching_active_task_and_sample_count() {
        let mut child = sample_task(1, "sample", 1);
        child.state = RunTaskState::Active;
        let store = TestStore {
            tasks: Arc::new(vec![child]),
            current_accumulator: Some(vector_state().to_json().unwrap()),
            ..Default::default()
        };
        let mut task = legacy_campaign_task(None, 4);
        hydrate_campaign_absolute_results(&store, &mut task)
            .await
            .unwrap();
        assert!(recovered_absolute_results(&task).is_some());
        let mut stale = legacy_campaign_task(None, 3);
        hydrate_campaign_absolute_results(&store, &mut stale)
            .await
            .unwrap();
        assert!(recovered_absolute_results(&stale).is_none());
        let other_task_store = TestStore {
            tasks: Arc::new(vec![]),
            ..store
        };
        let mut other_task = legacy_campaign_task(None, 4);
        hydrate_campaign_absolute_results(&other_task_store, &mut other_task)
            .await
            .unwrap();
        assert!(recovered_absolute_results(&other_task).is_none());
    }

    fn measurement_spec(metric: MeasurementMetricSpec) -> MeasurementSpec {
        MeasurementSpec {
            source_task: "sample".to_string(),
            quantity: MeasurementQuantitySpec::default(),
            metric: Some(metric),
            mode: MeasurementMode::Minimize,
        }
    }

    #[tokio::test]
    async fn extracts_completed_variance_measurement_from_stage_snapshot() {
        let store = TestStore {
            tasks: Arc::new(vec![sample_task(7, "sample", 1)]),
            latest_stage_snapshot: Some(TaskStageSnapshot {
                id: "1".to_string(),
                run_id: 1,
                task_id: "7".to_string(),
                observable_state: scalar_state(),
                created_at: Some(Utc::now()),
            }),
            ..TestStore::default()
        };

        let extracted = extract_measurement(
            &store,
            1,
            &measurement_spec(MeasurementMetricSpec::Name(AccumulatorMetricName::Variance)),
        )
        .await
        .expect("measurement");

        assert_eq!(extracted.results.len(), 1);
        let result = &extracted.results[0];
        assert_eq!(result.name, AccumulatorMetricName::Variance);
        assert!(result.value > 0.0);
        assert!(result.uncertainty.is_some());
    }

    #[tokio::test]
    async fn loads_persisted_task_measurement_output_by_task_name() {
        let mut task = sample_task(7, "sample", 1);
        task.measurement_output = Some(TaskMeasurementOutput::Completed {
            results: vec![MeasurementResult {
                name: AccumulatorMetricName::Mean,
                component: None,
                value: 2.0,
                uncertainty: None,
                sample_count: 4,
            }],
        });
        let store = TestStore {
            tasks: Arc::new(vec![task]),
            ..TestStore::default()
        };

        let measurement = load_task_measurement_output(&store, 1, "sample")
            .await
            .expect("measurement output");

        assert_eq!(measurement.task_id, 7);
        let Some(TaskMeasurementOutput::Completed { results }) = measurement.output else {
            panic!("expected completed output");
        };
        assert_eq!(results[0].value, 2.0);
    }

    #[tokio::test]
    async fn rejects_ambiguous_persisted_task_measurement_lookup() {
        let store = TestStore {
            tasks: Arc::new(vec![
                sample_task(7, "sample", 1),
                sample_task(8, "sample", 2),
            ]),
            ..TestStore::default()
        };

        let err = load_task_measurement_output(&store, 1, "sample")
            .await
            .expect_err("ambiguous task names should fail");

        assert!(err.to_string().contains("ambiguous"));
    }

    #[tokio::test]
    async fn extracts_default_central_values_for_vector_accumulator() {
        let store = TestStore {
            tasks: Arc::new(vec![sample_task(7, "sample", 1)]),
            latest_stage_snapshot: Some(TaskStageSnapshot {
                id: "1".to_string(),
                run_id: 1,
                task_id: "7".to_string(),
                observable_state: vector_state(),
                created_at: Some(Utc::now()),
            }),
            ..TestStore::default()
        };

        let extracted = extract_measurement(&store, 1, &central_value_measurement_spec())
            .await
            .expect("measurement");

        assert_eq!(extracted.results.len(), 2);
        assert_eq!(extracted.results[0].name, AccumulatorMetricName::Mean);
        assert_eq!(extracted.results[0].component.as_deref(), Some("real"));
        assert_eq!(extracted.results[1].name, AccumulatorMetricName::Mean);
        assert_eq!(extracted.results[1].component.as_deref(), Some("imag"));
    }

    #[tokio::test]
    async fn extracts_default_task_local_central_values_for_vector_accumulator() {
        let task = sample_task(7, "sample", 1);
        let store = TestStore {
            tasks: Arc::new(vec![task.clone()]),
            latest_stage_snapshot: Some(TaskStageSnapshot {
                id: "1".to_string(),
                run_id: 1,
                task_id: "7".to_string(),
                observable_state: vector_state(),
                created_at: Some(Utc::now()),
            }),
            ..TestStore::default()
        };

        let extracted = extract_task_measurement(&store, 1, &task)
            .await
            .expect("measurement");

        assert_eq!(extracted.results.len(), 2);
        assert_eq!(extracted.results[0].name, AccumulatorMetricName::Mean);
        assert_eq!(extracted.results[0].component.as_deref(), Some("real"));
        assert_eq!(extracted.results[1].component.as_deref(), Some("imag"));
    }

    #[tokio::test]
    async fn extracts_task_local_metric_measurement() {
        let mut task = sample_task(7, "sample", 1);
        if let RunTaskSpec::Sample { measurement, .. } = &mut task.task {
            *measurement = Some(TaskMeasurementSpec {
                quantity: MeasurementQuantitySpec::default(),
                metric: Some(MeasurementMetricSpec::Name(AccumulatorMetricName::Variance)),
                mode: MeasurementMode::Minimize,
            });
        }
        let store = TestStore {
            tasks: Arc::new(vec![task.clone()]),
            latest_stage_snapshot: Some(TaskStageSnapshot {
                id: "1".to_string(),
                run_id: 1,
                task_id: "7".to_string(),
                observable_state: scalar_state(),
                created_at: Some(Utc::now()),
            }),
            ..TestStore::default()
        };

        let extracted = extract_task_measurement(&store, 1, &task)
            .await
            .expect("measurement");

        assert_eq!(extracted.results.len(), 1);
        assert_eq!(extracted.results[0].name, AccumulatorMetricName::Variance);
    }

    #[tokio::test]
    async fn extracts_time_normalized_variance_measurement_with_sampler_throughput() {
        let runtime_metrics = json!({
            "produced_batches_total": 0,
            "produced_samples_total": 0,
            "ingested_batches_total": 0,
            "ingested_samples_total": 0,
            "completed_samples_total": 4,
            "sampler_uptime_ms": 1000.0,
            "completed_samples_per_second": 2.0,
            "eta_seconds": null,
            "batch_size_current": 1,
            "sampler_tick_busy_ratio": null,
            "avg_evaluator_utilization": null,
            "active_evaluator_count": null,
            "avg_evaluator_rss_bytes": null,
            "total_evaluator_rss_bytes": null,
            "sampler": {},
            "queue": {}
        });
        let store = TestStore {
            tasks: Arc::new(vec![sample_task(7, "sample", 1)]),
            latest_stage_snapshot: Some(TaskStageSnapshot {
                id: "1".to_string(),
                run_id: 1,
                task_id: "7".to_string(),
                observable_state: scalar_state(),
                created_at: Some(Utc::now()),
            }),
            sampler_history: Arc::new(vec![SamplerPerformanceHistoryEntry {
                id: 1,
                run_id: 1,
                worker_id: "sampler".to_string(),
                metrics: SamplerPerformanceMetrics::default(),
                runtime_metrics,
                engine_diagnostics: JsonValue::Null,
                rss_bytes: None,
                created_at: Utc::now(),
            }]),
            ..TestStore::default()
        };

        let extracted = extract_measurement(
            &store,
            1,
            &measurement_spec(MeasurementMetricSpec::Name(
                AccumulatorMetricName::TimeNormalizedVariance,
            )),
        )
        .await
        .expect("measurement");

        assert_eq!(
            extracted.results[0].name,
            AccumulatorMetricName::TimeNormalizedVariance
        );
        assert!(extracted.results[0].value > 0.0);
        assert!(extracted.results[0].uncertainty.is_some());
    }
}
