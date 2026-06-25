use crate::api::ApiError;
use crate::core::{
    AccumulatorMetricName, AccumulatorMetricSelector, AggregationStore, MeasurementResult,
    MeasurementSpec, RunReadStore, RunTask, RunTaskState, RunTaskStore, SamplerRuntimeMetrics,
    TaskMeasurementOutput, TaskMeasurementSpec,
};
use crate::evaluation::{
    AccumulatorMetricValue, AccumulatorState, extract_accumulator_metric_with_runtime,
};
use crate::stores::SamplerPerformanceHistoryEntry;

#[derive(Debug, Clone)]
pub struct ExtractedMeasurement {
    pub source_task_id: i64,
    pub source_task_name: String,
    pub results: Vec<MeasurementResult>,
}

#[derive(Debug, Clone)]
pub struct PersistedTaskMeasurement {
    pub task_id: i64,
    pub task_name: String,
    pub task_state: RunTaskState,
    pub output: Option<TaskMeasurementOutput>,
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
        task_name: task.name.clone(),
        task_state: task.state,
        output: task.measurement_output.clone(),
    })
}

pub async fn extract_measurement(
    store: &(impl RunTaskStore + RunReadStore + AggregationStore),
    run_id: i32,
    measurement: &MeasurementSpec,
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
    let metrics = extract_measurement_metrics(&accumulator, measurement, throughput, source_task)?;
    Ok(ExtractedMeasurement {
        source_task_id: source_task.id,
        source_task_name: source_task.name.clone(),
        results: metrics
            .into_iter()
            .map(|metric| measurement_result_from_metric(metric, throughput))
            .collect(),
    })
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

fn measurement_result_from_metric(
    metric: AccumulatorMetricValue,
    completed_samples_per_second: Option<f64>,
) -> MeasurementResult {
    MeasurementResult {
        name: metric.name,
        component: metric.component,
        value: metric.value,
        uncertainty: metric.uncertainty,
        sample_count: metric.sample_count,
        completed_samples_per_second,
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
        RunTaskInput, RunTaskSpec, SampleStopCondition, SamplerPerformanceMetrics,
        TaskMeasurementSpec, TrainingProjection,
    };
    use crate::evaluation::Point;
    use crate::stores::{
        EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress, RuntimeLogPage,
        TaskOutputSnapshot, TaskStageSnapshot, WorkQueueStats,
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
    }

    #[async_trait]
    impl AggregationStore for TestStore {
        async fn load_current_accumulator(
            &self,
            _run_id: i32,
        ) -> Result<Option<JsonValue>, crate::core::StoreError> {
            Ok(self.current_accumulator.clone())
        }
        async fn load_sampler_checkpoint(
            &self,
            _run_id: i32,
        ) -> Result<
            Option<crate::runners::sampler_aggregator::SamplerAggregatorCheckpoint>,
            crate::core::StoreError,
        > {
            unreachable!("unused")
        }
        async fn load_stage_snapshot(
            &self,
            _snapshot_id: i64,
        ) -> Result<Option<crate::core::RunStageSnapshot>, crate::core::StoreError> {
            unreachable!("unused")
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
            _checkpoint: &crate::runners::sampler_aggregator::SamplerAggregatorCheckpoint,
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
            _output: &JsonValue,
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
        async fn get_work_queue_stats(
            &self,
            _run_id: i32,
        ) -> Result<Vec<WorkQueueStats>, crate::core::StoreError> {
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
        async fn get_worker_evaluator_performance_history(
            &self,
            _worker_id: &str,
            _limit: i64,
        ) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn get_worker_sampler_performance_history(
            &self,
            _worker_id: &str,
            _limit: i64,
        ) -> Result<Vec<SamplerPerformanceHistoryEntry>, crate::core::StoreError> {
            unreachable!("unused")
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
        async fn upsert_desired_assignment(
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
        async fn reconcile_running_node_launch_requests(
            &self,
        ) -> Result<u64, crate::core::StoreError> {
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
            _integration_params: &JsonValue,
            _target: Option<&JsonValue>,
            _domain: &crate::utils::domain::Domain,
            _initial_stage_snapshot: &crate::core::RunStageSnapshot,
            _initial_tasks: &[RunTaskInput],
        ) -> Result<i32, crate::core::StoreError> {
            unreachable!("unused")
        }
        async fn set_run_parent_metadata(
            &self,
            _run_id: i32,
            _parent_run_id: i32,
            _parent_task_id: Option<i64>,
            _spawn_kind: &str,
            _spawn_label: Option<&str>,
        ) -> Result<(), crate::core::StoreError> {
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

        assert_eq!(extracted.source_task_id, 7);
        assert_eq!(extracted.source_task_name, "sample");
        assert_eq!(extracted.results.len(), 1);
        let result = &extracted.results[0];
        assert_eq!(result.name, AccumulatorMetricName::Variance);
        assert!(result.value > 0.0);
        assert!(result.uncertainty.is_some());
        assert_eq!(result.completed_samples_per_second, None);
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
                completed_samples_per_second: None,
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
            "eta_completed_samples_per_second": 0.0,
            "eta_seconds_smoothed": null,
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
        assert_eq!(extracted.results[0].completed_samples_per_second, Some(2.0));
        assert!(extracted.results[0].value > 0.0);
        assert!(extracted.results[0].uncertainty.is_some());
    }
}
