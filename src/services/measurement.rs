#[cfg(test)]
use crate::core::MeasurementSelection;
use crate::core::{
    AccumulatorMetricName, AccumulatorMetricSelector, AggregationStore, MeasurementResult,
    RunReadStore, RunTask, RunTaskState, RunTaskStore, TaskMeasurementOutput, TaskMeasurementSpec,
};
use crate::evaluation::{AccumulatorState, extract_accumulator_metric_with_runtime};
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
    extract_measurement_metrics(
        accumulator,
        measurement,
        completed_samples_per_second,
        source_task,
    )
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
        })
        .collect()
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
    let Some(selector) = measurement.metric_selector() else {
        return Ok(None);
    };
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
    let rate = entry.metrics.completed_samples_per_second;
    if rate.is_finite() && rate > 0.0 {
        Some(rate)
    } else {
        None
    }
}

fn extract_measurement_metrics(
    accumulator: &AccumulatorState,
    measurement: &TaskMeasurementSpec,
    completed_samples_per_second: Option<f64>,
    source_task: &RunTask,
) -> Result<Vec<MeasurementResult>, ApiError> {
    if let Some(selector) = measurement.metric_selector() {
        return extract_single_metric(
            accumulator,
            selector,
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
) -> Result<MeasurementResult, ApiError> {
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
) -> Result<Vec<MeasurementResult>, ApiError> {
    let selectors = match accumulator {
        AccumulatorState::Stratified(state) => state
            .plan
            .components
            .iter()
            .map(|name| AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: Some(name.clone()),
            })
            .collect(),
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
    use crate::core::{
        AccumulatorConfig, MeasurementMode, MeasurementSpec, RunTaskSpec, SampleStopCondition,
        TaskMeasurementSpec, TrainingProjection,
    };
    use crate::evaluation::Point;
    use crate::stores::TaskStageSnapshot;
    use crate::test_support::RecordingStore as TestStore;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;

    fn sample_task(id: i64, name: &str, sequence_nr: i32) -> RunTask {
        RunTask {
            id,
            run_id: 1,
            name: name.to_string(),
            sequence_nr,
            task: RunTaskSpec::Sample {
                estimator: crate::core::Estimator::Pooled,
                publish_result: true,
                stop_condition: SampleStopCondition {
                    max_samples: Some(100),
                    ..SampleStopCondition::default()
                },
                measurement: None,
                evaluator: None,
                sampler: None,
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
        let config = AccumulatorConfig::Vector {
            components: vec!["value".into()],
            training_projection: TrainingProjection::component("value"),
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
            measurement: TaskMeasurementSpec {
                selection: MeasurementSelection::AllComponentMeans,
                mode: MeasurementMode::Minimize,
            },
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
        assert!((absolute[0].uncertainty.unwrap() - 4.5).abs() < 1e-12);
        assert_ne!(absolute[0].uncertainty, signed[0].uncertainty);
        assert_eq!(signed[1].value, 0.0);
        assert_eq!(absolute[1].value, 2.0);
        assert_eq!(absolute[1].uncertainty, Some(0.0));
        assert!(absolute.iter().all(
            |result| result.name == AccumulatorMetricName::AbsMean && result.sample_count == 2
        ));
    }

    fn measurement_spec(metric: MeasurementSelection) -> MeasurementSpec {
        MeasurementSpec {
            source_task: "sample".to_string(),
            measurement: TaskMeasurementSpec {
                selection: metric,
                mode: MeasurementMode::Minimize,
            },
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
            &measurement_spec(MeasurementSelection::metric(
                AccumulatorMetricName::Variance,
            )),
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
                estimator: crate::core::EstimateKind::Pooled,
                basic: None,
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
                selection: MeasurementSelection::metric(AccumulatorMetricName::Variance),
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
        let defaults: crate::core::IntegrationParams =
            toml::from_str(include_str!("../config_defaults/run.toml")).unwrap();
        let metrics = json!({
            "epoch": "test", "node_uuid": "sampler", "task_id": "7",
            "busy": crate::core::WorkerBusyMetrics::default(),
            "engine_diagnostics": {},
            "queue_config": defaults.sampler_aggregator_runner_params.queue,
            "target_pending_batches": null, "buffered_generated_samples": 0,
            "checkpoint_state": "Ready",
            "produced_batches_total": 0,
            "produced_samples_total": 0,
            "ingested_batches_total": 0,
            "ingested_samples_total": 0,
            "completed_samples_total": 4,
            "sampler_uptime_ms": 1000.0,
            "completed_samples_per_second": 2.0,
            "eta_seconds": null,
            "batch_size_current": 1,
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
                metrics: serde_json::from_value(metrics).unwrap(),
                rss_bytes: None,
                created_at: Utc::now(),
            }]),
            ..TestStore::default()
        };

        let extracted = extract_measurement(
            &store,
            1,
            &measurement_spec(MeasurementSelection::metric(
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
