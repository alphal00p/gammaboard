use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, PlotPoint, PlotSeries, multi_timeseries_panel,
    panel_spec, progress_panel, table_panel_with_payload_and_options, with_panel_width,
};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeSet;

const TUNING_PROGRESS_PANEL_ID: &str = "tuning_progress";
const TUNING_OBJECTIVE_PANEL_ID: &str = "tuning_objective";
const TUNING_TRIALS_PANEL_ID: &str = "tuning_trials";

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    vec![
        tuning_progress_projector(),
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
            let mut columns = vec![
                "index".to_string(),
                "status".to_string(),
                "run".to_string(),
                "objective".to_string(),
                "uncertainty".to_string(),
                "samples".to_string(),
            ];
            columns.extend(parameter_names.iter().cloned());
            columns.push("failure".to_string());
            let rows = trials
                .into_iter()
                .map(|trial| trial_to_table_row(trial, &parameter_names))
                .collect::<Vec<_>>();
            Ok(Some(table_panel_with_payload_and_options(
                TUNING_TRIALS_PANEL_ID,
                columns,
                rows,
                Some(json!({
                    "row_action": {
                        "kind": "select_run",
                        "column": "run",
                    },
                })),
                Default::default(),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn tuning_total_trials(ctx: &TaskPanelContext<'_>) -> Option<u64> {
    ctx.task
        .controller_output
        .as_ref()
        .and_then(|output| output.get("total_trials"))
        .and_then(JsonValue::as_u64)
        .or_else(|| match &ctx.task.task {
            crate::core::RunTaskSpec::HyperparameterTuning { optimizer, .. } => {
                Some(optimizer.max_trials as u64)
            }
            _ => None,
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

fn trial_to_table_row(trial: &JsonValue, parameter_names: &[String]) -> Vec<JsonValue> {
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
    row.push(
        trial
            .get("failure_reason")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
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
                max_trials: 3,
                seed: Some(1),
                params: json!({}),
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
                            values: vec![toml::Value::Float(1.0)],
                        },
                    ),
                ),
            ]),
            trial_run_toml: "name = \"trial\"".to_string(),
            max_concurrent_trials: 1,
            max_failed_trials: 0,
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
        let PanelState::Table { columns, rows, .. } = panel else {
            panic!("expected table");
        };
        assert!(columns.contains(&"center".to_string()));
        assert!(columns.contains(&"scale".to_string()));
        assert!(columns.contains(&"samples".to_string()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][5], json!(4096));
    }
}
