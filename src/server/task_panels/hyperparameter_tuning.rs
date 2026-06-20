use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, PlotPoint, PlotSeries, key_value, key_value_panel,
    min_max_row_tones, multi_timeseries_panel, panel_spec, progress_panel, row_tone_labels,
    table_panel_with_payload_and_options, with_panel_width,
};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeSet;

const TUNING_PROGRESS_PANEL_ID: &str = "tuning_progress";
const TUNING_BEST_PANEL_ID: &str = "tuning_best";
const TUNING_OBJECTIVE_PANEL_ID: &str = "tuning_objective";
const TUNING_TRIALS_PANEL_ID: &str = "tuning_trials";

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    vec![
        tuning_progress_projector(),
        tuning_best_projector(),
        tuning_objective_projector(),
        tuning_trials_projector(),
    ]
}

fn tuning_progress_projector() -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            TUNING_PROGRESS_PANEL_ID,
            "Tuning Progress",
            PanelKind::Progress,
            PanelHistoryMode::None,
        ),
        |ctx| {
            Ok(Some(progress_panel(
                TUNING_PROGRESS_PANEL_ID,
                ctx.task
                    .controller_output
                    .as_ref()
                    .and_then(|output| output.get("completed_trials"))
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as f64,
                tuning_total_trials(ctx).map(|value| value as f64),
                Some("trials"),
                None,
            )))
        },
        |_ctx| Ok(None),
    )
}

fn tuning_best_projector() -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            TUNING_BEST_PANEL_ID,
            "Best Trial",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
        ),
        |ctx| {
            let trials = tuning_trials(ctx);
            let Some(best) = best_trial(&trials, tuning_mode(ctx)) else {
                return Ok(Some(key_value_panel(
                    TUNING_BEST_PANEL_ID,
                    vec![key_value("status", "Status", "No completed trials yet")],
                )));
            };
            let mut entries = vec![
                key_value(
                    "trial",
                    "Trial",
                    best.get("index").cloned().unwrap_or(JsonValue::Null),
                ),
                key_value(
                    "run",
                    "Run",
                    best.get("child_run_id").cloned().unwrap_or(JsonValue::Null),
                ),
                key_value(
                    "objective",
                    "Objective",
                    best.get("objective_value")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                key_value(
                    "uncertainty",
                    "Uncertainty",
                    best.get("objective_uncertainty")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                key_value("samples", "Samples", objective_sample_count(best)),
            ];
            for name in tuning_parameter_names(ctx, &trials) {
                let value = best
                    .get("parameters")
                    .and_then(JsonValue::as_object)
                    .and_then(|parameters| parameters.get(&name))
                    .cloned()
                    .unwrap_or(JsonValue::Null);
                entries.push(key_value(&format!("parameter_{name}"), &name, value));
            }
            Ok(Some(key_value_panel(TUNING_BEST_PANEL_ID, entries)))
        },
        |_ctx| Ok(None),
    )
}

fn tuning_objective_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                TUNING_OBJECTIVE_PANEL_ID,
                "Objective by Trial",
                PanelKind::MultiTimeseries,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let objective_points = tuning_trials(ctx)
                .into_iter()
                .filter_map(trial_to_plot_point)
                .collect::<Vec<_>>();
            let best_points = best_so_far_points(&objective_points, tuning_mode(ctx));
            Ok(Some(multi_timeseries_panel(
                TUNING_OBJECTIVE_PANEL_ID,
                vec![
                    PlotSeries {
                        id: "objective".to_string(),
                        label: "Objective".to_string(),
                        color: None,
                        smooth: None,
                        points: objective_points,
                    },
                    PlotSeries {
                        id: "best_so_far".to_string(),
                        label: "Best so far".to_string(),
                        color: Some("#0f766e".to_string()),
                        smooth: None,
                        points: best_points,
                    },
                ],
            )))
        },
        |_ctx| Ok(None),
    )
}

fn tuning_trials_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                TUNING_TRIALS_PANEL_ID,
                "Tuning Trials",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let trials = tuning_trials(ctx);
            let parameter_names = tuning_parameter_names(ctx, &trials);
            let show_failure = trials.iter().any(|trial| {
                trial
                    .get("failure_reason")
                    .is_some_and(|reason| !reason.is_null())
            });
            let mut columns = vec![
                "index".to_string(),
                "status".to_string(),
                "run".to_string(),
                "objective".to_string(),
                "uncertainty".to_string(),
                "samples".to_string(),
            ];
            columns.extend(parameter_names.iter().cloned());
            if show_failure {
                columns.push("failure".to_string());
            }
            let rows = trials
                .into_iter()
                .map(|trial| trial_to_table_row(trial, &parameter_names, show_failure))
                .collect::<Vec<_>>();
            let row_tones = min_max_row_tones(&rows, tuning_table_objective_column());
            Ok(Some(table_panel_with_payload_and_options(
                TUNING_TRIALS_PANEL_ID,
                columns,
                rows,
                Some(json!({
                    "row_action": {
                        "kind": "select_run",
                        "column": "run",
                    },
                    "row_tones": row_tones,
                    "row_tone_labels": row_tone_labels(),
                })),
                Default::default(),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn tuning_table_objective_column() -> usize {
    3
}

fn tuning_total_trials(ctx: &TaskPanelContext<'_>) -> Option<u64> {
    ctx.task
        .controller_output
        .as_ref()
        .and_then(|output| output.get("total_trials"))
        .and_then(JsonValue::as_u64)
        .or_else(|| {
            ctx.task
                .task
                .nr_expected_samples()
                .and_then(|count| u64::try_from(count).ok())
        })
}

fn tuning_mode(ctx: &TaskPanelContext<'_>) -> crate::core::MeasurementMode {
    match &ctx.task.task {
        crate::core::RunTaskSpec::HyperparameterTuning { objective, .. } => objective.mode,
        _ => crate::core::MeasurementMode::Minimize,
    }
}

fn tuning_trials<'a>(ctx: &'a TaskPanelContext<'_>) -> Vec<&'a JsonValue> {
    ctx.task
        .controller_output
        .as_ref()
        .and_then(|output| output.get("trials"))
        .and_then(JsonValue::as_array)
        .map(|trials| trials.iter().collect())
        .unwrap_or_default()
}

fn tuning_parameter_names(ctx: &TaskPanelContext<'_>, trials: &[&JsonValue]) -> Vec<String> {
    let mut names = match &ctx.task.task {
        crate::core::RunTaskSpec::HyperparameterTuning { parameters, .. } => {
            parameters.keys().cloned().collect::<BTreeSet<_>>()
        }
        _ => BTreeSet::new(),
    };
    for trial in trials {
        if let Some(parameters) = trial.get("parameters").and_then(JsonValue::as_object) {
            names.extend(parameters.keys().cloned());
        }
    }
    names.into_iter().collect()
}

fn best_trial<'a>(
    trials: &[&'a JsonValue],
    mode: crate::core::MeasurementMode,
) -> Option<&'a JsonValue> {
    trials
        .iter()
        .filter_map(|trial| {
            trial
                .get("objective_value")
                .and_then(JsonValue::as_f64)
                .map(|value| (*trial, value))
        })
        .min_by(|(_, left), (_, right)| {
            let ordering = left.total_cmp(right);
            match mode {
                crate::core::MeasurementMode::Minimize => ordering,
                crate::core::MeasurementMode::Maximize => ordering.reverse(),
            }
        })
        .map(|(trial, _)| trial)
}

fn trial_to_plot_point(trial: &JsonValue) -> Option<PlotPoint> {
    let x = trial.get("index").and_then(JsonValue::as_u64)? as f64;
    let y = trial.get("objective_value").and_then(JsonValue::as_f64)?;
    let uncertainty = trial
        .get("objective_uncertainty")
        .and_then(JsonValue::as_f64);
    Some(PlotPoint {
        x,
        y,
        x_sampler_uptime_ms: None,
        x_completed_samples_total: None,
        y_min: uncertainty.map(|error| y - error),
        y_max: uncertainty.map(|error| y + error),
    })
}

fn best_so_far_points(points: &[PlotPoint], mode: crate::core::MeasurementMode) -> Vec<PlotPoint> {
    let mut best: Option<f64> = None;
    let mut best_points = Vec::with_capacity(points.len());
    for point in points {
        let is_better = best
            .map(|current| match mode {
                crate::core::MeasurementMode::Minimize => point.y < current,
                crate::core::MeasurementMode::Maximize => point.y > current,
            })
            .unwrap_or(true);
        if is_better {
            best = Some(point.y);
        }
        let Some(y) = best else {
            continue;
        };
        best_points.push(PlotPoint {
            x: point.x,
            y,
            x_sampler_uptime_ms: None,
            x_completed_samples_total: None,
            y_min: None,
            y_max: None,
        });
    }
    best_points
}

fn trial_to_table_row(
    trial: &JsonValue,
    parameter_names: &[String],
    include_failure: bool,
) -> Vec<JsonValue> {
    let mut row = vec![
        trial.get("index").cloned().unwrap_or(JsonValue::Null),
        trial.get("status").cloned().unwrap_or(JsonValue::Null),
        trial
            .get("child_run_id")
            .cloned()
            .unwrap_or(JsonValue::Null),
        trial
            .get("objective_value")
            .cloned()
            .unwrap_or(JsonValue::Null),
        trial
            .get("objective_uncertainty")
            .cloned()
            .unwrap_or(JsonValue::Null),
        objective_sample_count(trial),
    ];
    let parameters = trial.get("parameters").and_then(JsonValue::as_object);
    row.extend(parameter_names.iter().map(|name| {
        parameters
            .and_then(|values| values.get(name))
            .cloned()
            .unwrap_or(JsonValue::Null)
    }));
    if include_failure {
        row.push(
            trial
                .get("failure_reason")
                .cloned()
                .unwrap_or(JsonValue::Null),
        );
    }
    row
}

fn objective_sample_count(trial: &JsonValue) -> JsonValue {
    trial
        .get("measurement")
        .and_then(|measurement| measurement.get("results"))
        .and_then(JsonValue::as_array)
        .and_then(|results| results.first())
        .and_then(|result| result.get("sample_count"))
        .cloned()
        .unwrap_or(JsonValue::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        HyperparameterTuningAlgorithm, HyperparameterTuningCategoricalDomain,
        HyperparameterTuningFloatDomain, HyperparameterTuningObjectiveSpec,
        HyperparameterTuningOptimizerSpec, HyperparameterTuningParameterDomain, MeasurementMode,
        MeasurementQuantitySpec, RunTask, RunTaskInput, RunTaskSpec, RunTaskState,
        canonical_task_toml,
    };
    use crate::server::panels::PanelState;
    use chrono::Utc;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn tuning_task(controller_output: Option<JsonValue>) -> RunTask {
        let task = RunTaskSpec::HyperparameterTuning {
            optimizer: HyperparameterTuningOptimizerSpec {
                algorithm: HyperparameterTuningAlgorithm::RandomSearch,
                params: json!({ "max_trials": 3, "seed": 1 }),
            },
            objective: HyperparameterTuningObjectiveSpec {
                source_task: "sample".to_string(),
                quantity: MeasurementQuantitySpec::default(),
                metric: None,
                mode: MeasurementMode::Minimize,
            },
            parameters: BTreeMap::from([
                (
                    "center".to_string(),
                    HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
                        min: 0.0,
                        max: 1.0,
                    }),
                ),
                (
                    "scale".to_string(),
                    HyperparameterTuningParameterDomain::Categorical(
                        HyperparameterTuningCategoricalDomain {
                            source: crate::core::ParameterValueSourceSpec {
                                values: vec![toml::Value::Float(1.0)],
                                ..Default::default()
                            },
                        },
                    ),
                ),
            ]),
            trial_run_toml: "name = \"trial\"".to_string(),
            max_concurrent_trials: 1,
        };
        RunTask {
            id: 1,
            run_id: 1,
            name: "tune".to_string(),
            sequence_nr: 1,
            task: task.clone(),
            spawned_from_snapshot_id: None,
            state: RunTaskState::Active,
            nr_produced_samples: 0,
            nr_completed_samples: 0,
            nr_produced_samples_including_children: 0,
            nr_completed_samples_including_children: 0,
            failure_reason: None,
            started_at: None,
            completed_at: None,
            failed_at: None,
            created_at: Utc::now(),
            task_toml: canonical_task_toml(&RunTaskInput {
                name: Some("tune".to_string()),
                task,
            })
            .expect("task toml"),
            measurement_output: None,
            controller_output,
        }
    }

    fn panel_ctx<'a>(task: &'a RunTask) -> TaskPanelContext<'a> {
        TaskPanelContext {
            task,
            source: super::super::TaskPanelCurrentSource::Empty,
            panel_state: &JsonValue::Null,
            run_target: None,
            completed_samples_per_second: None,
            smoothed_eta_seconds: None,
            sampler_engine_diagnostics: None,
        }
    }

    #[test]
    fn tuning_objective_panel_includes_best_so_far_series() {
        let task = tuning_task(Some(json!({
            "completed_trials": 3,
            "running_trials": 0,
            "failed_trials": 0,
            "total_trials": 3,
            "trials": [
                {"index": 0, "status": "completed", "objective_value": 3.0, "parameters": {"center": 0.1, "scale": 1.0}},
                {"index": 1, "status": "completed", "objective_value": 2.0, "parameters": {"center": 0.5, "scale": 1.0}},
                {"index": 2, "status": "completed", "objective_value": 2.5, "parameters": {"center": 0.8, "scale": 1.0}}
            ]
        })));
        let panel = tuning_objective_projector()
            .current(&panel_ctx(&task))
            .expect("projector")
            .expect("panel");
        let PanelState::MultiTimeseries { series, .. } = panel else {
            panic!("expected multi timeseries");
        };
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].id, "objective");
        assert_eq!(series[1].id, "best_so_far");
        assert_eq!(
            series[1]
                .points
                .iter()
                .map(|point| point.y)
                .collect::<Vec<_>>(),
            vec![3.0, 2.0, 2.0]
        );
    }

    #[test]
    fn tuning_table_expands_parameter_columns() {
        let task = tuning_task(Some(json!({
            "completed_trials": 1,
            "running_trials": 0,
            "failed_trials": 0,
            "total_trials": 1,
            "trials": [
                {
                    "index": 0,
                    "status": "completed",
                    "child_run_id": 11,
                    "objective_value": 1.25,
                    "objective_uncertainty": 0.1,
                    "parameters": {"center": 0.5, "scale": 1.0},
                    "measurement": {
                        "status": "completed",
                        "results": [{"name": "mean", "value": 1.25, "uncertainty": 0.1, "sample_count": 4096}]
                    }
                }
            ]
        })));
        let panel = tuning_trials_projector()
            .current(&panel_ctx(&task))
            .expect("projector")
            .expect("panel");
        let PanelState::Table {
            columns,
            rows,
            payload,
            ..
        } = panel
        else {
            panic!("expected table");
        };
        assert!(columns.contains(&"center".to_string()));
        assert!(columns.contains(&"scale".to_string()));
        assert!(columns.contains(&"samples".to_string()));
        assert!(!columns.contains(&"failure".to_string()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][5], json!(4096));
        let row_tones = payload
            .as_ref()
            .and_then(|payload| payload.get("row_tones"))
            .and_then(JsonValue::as_array)
            .expect("row tones");
        assert_eq!(row_tones, &vec![json!("min_max")]);
    }

    #[test]
    fn tuning_table_shows_failure_column_only_when_needed() {
        let task = tuning_task(Some(json!({
            "completed_trials": 0,
            "running_trials": 0,
            "failed_trials": 1,
            "total_trials": 1,
            "trials": [
                {
                    "index": 0,
                    "status": "failed",
                    "child_run_id": 11,
                    "parameters": {"center": 0.5, "scale": 1.0},
                    "failure_reason": "measurement failed"
                }
            ]
        })));
        let panel = tuning_trials_projector()
            .current(&panel_ctx(&task))
            .expect("projector")
            .expect("panel");
        let PanelState::Table { columns, rows, .. } = panel else {
            panic!("expected table");
        };
        assert_eq!(columns.last().map(String::as_str), Some("failure"));
        assert_eq!(
            rows[0].last().cloned().unwrap_or(JsonValue::Null),
            json!("measurement failed")
        );
    }

    #[test]
    fn tuning_best_panel_summarizes_best_trial() {
        let task = tuning_task(Some(json!({
            "completed_trials": 2,
            "running_trials": 0,
            "failed_trials": 0,
            "total_trials": 2,
            "trials": [
                {"index": 0, "status": "completed", "child_run_id": 11, "objective_value": 3.0, "parameters": {"center": 0.1, "scale": 1.0}},
                {
                    "index": 1,
                    "status": "completed",
                    "child_run_id": 12,
                    "objective_value": 2.0,
                    "objective_uncertainty": 0.2,
                    "parameters": {"center": 0.5, "scale": 1.0},
                    "measurement": {
                        "status": "completed",
                        "results": [{"name": "mean", "value": 2.0, "uncertainty": 0.2, "sample_count": 200}]
                    }
                }
            ]
        })));
        let panel = tuning_best_projector()
            .current(&panel_ctx(&task))
            .expect("projector")
            .expect("panel");
        let PanelState::KeyValue { entries, .. } = panel else {
            panic!("expected key-value panel");
        };
        let entries_by_key = entries
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(entries_by_key.get("trial"), Some(&json!(1)));
        assert_eq!(entries_by_key.get("run"), Some(&json!(12)));
        assert_eq!(entries_by_key.get("objective"), Some(&json!(2.0)));
        assert_eq!(entries_by_key.get("samples"), Some(&json!(200)));
        assert_eq!(entries_by_key.get("parameter_center"), Some(&json!(0.5)));
    }
}
