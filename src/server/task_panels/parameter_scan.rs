use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, PlotPoint, progress_panel, scalar_timeseries_panel,
    table_panel_with_payload_and_options, with_panel_width,
};
use serde_json::{Value as JsonValue, json};

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    vec![
        scan_progress_projector(),
        scan_mean_projector(),
        scan_points_projector(),
    ]
}

fn scan_progress_projector() -> TaskPanelProjector {
    panel_projector(
        crate::server::panels::panel_spec(
            "scan_progress",
            "Scan Progress",
            PanelKind::Progress,
            PanelHistoryMode::None,
        ),
        |ctx| {
            let Some(output) = ctx.task.controller_output.as_ref() else {
                return Ok(Some(progress_panel(
                    "scan_progress",
                    0.0,
                    total_points_from_task(ctx).map(|value| value as f64),
                    Some("points"),
                    None,
                )));
            };
            Ok(Some(progress_panel(
                "scan_progress",
                output
                    .get("completed_points")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as f64,
                output
                    .get("total_points")
                    .and_then(JsonValue::as_u64)
                    .map(|value| value as f64),
                Some("points"),
                None,
            )))
        },
        |_ctx| Ok(None),
    )
}

fn scan_mean_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            crate::server::panels::panel_spec(
                "scan_mean",
                "Mean over Parameter",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let points = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(|output| output.get("points"))
                .and_then(JsonValue::as_array)
                .map(|points| {
                    points
                        .iter()
                        .filter_map(scan_point_to_plot_point)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(Some(scalar_timeseries_panel("scan_mean", points)))
        },
        |_ctx| Ok(None),
    )
}

fn scan_points_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            crate::server::panels::panel_spec(
                "scan_points",
                "Scan Points",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let rows = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(|output| output.get("points"))
                .and_then(JsonValue::as_array)
                .map(|points| {
                    points
                        .iter()
                        .map(scan_point_to_table_row)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(Some(table_panel_with_payload_and_options(
                "scan_points",
                vec![
                    "index".to_string(),
                    "parameter".to_string(),
                    "status".to_string(),
                    "run".to_string(),
                    "mean".to_string(),
                    "uncertainty".to_string(),
                    "samples".to_string(),
                ],
                rows,
                ctx.task.controller_output.as_ref().map(|output| {
                    json!({
                        "parameter_name": output.get("parameter_name").cloned().unwrap_or(JsonValue::Null),
                    })
                }),
                Default::default(),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn scan_point_to_plot_point(point: &JsonValue) -> Option<PlotPoint> {
    let x = json_number(point.get("parameter_value")?)?;
    let result = mean_result(point)?;
    let y = result.get("value").and_then(JsonValue::as_f64)?;
    let uncertainty = result.get("uncertainty").and_then(JsonValue::as_f64);
    Some(PlotPoint {
        x,
        y,
        x_sampler_uptime_ms: None,
        x_completed_samples_total: result
            .get("sample_count")
            .and_then(JsonValue::as_i64)
            .map(|value| value as f64),
        y_min: uncertainty.map(|error| y - error),
        y_max: uncertainty.map(|error| y + error),
    })
}

fn scan_point_to_table_row(point: &JsonValue) -> Vec<JsonValue> {
    let result = mean_result(point);
    vec![
        point.get("index").cloned().unwrap_or(JsonValue::Null),
        point
            .get("parameter_value")
            .cloned()
            .unwrap_or(JsonValue::Null),
        point.get("status").cloned().unwrap_or(JsonValue::Null),
        point
            .get("child_run_id")
            .cloned()
            .unwrap_or(JsonValue::Null),
        result
            .and_then(|value| value.get("value"))
            .cloned()
            .unwrap_or(JsonValue::Null),
        result
            .and_then(|value| value.get("uncertainty"))
            .cloned()
            .unwrap_or(JsonValue::Null),
        result
            .and_then(|value| value.get("sample_count"))
            .cloned()
            .unwrap_or(JsonValue::Null),
    ]
}

fn mean_result(point: &JsonValue) -> Option<&JsonValue> {
    point
        .get("measurement")?
        .get("results")?
        .as_array()?
        .iter()
        .find(|result| result.get("name").and_then(JsonValue::as_str) == Some("mean"))
}

fn json_number(value: &JsonValue) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|value| value as f64))
        .or_else(|| value.as_u64().map(|value| value as f64))
}

fn total_points_from_task(ctx: &TaskPanelContext<'_>) -> Option<usize> {
    match &ctx.task.task {
        crate::core::RunTaskSpec::ParameterScan { parameter, .. } => Some(parameter.values.len()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        ParameterScanMeasurementSpec, ParameterScanParameterSpec, RunTask, RunTaskInput,
        RunTaskSpec, RunTaskState, canonical_task_toml,
    };
    use crate::server::panels::PanelState;
    use chrono::Utc;

    fn scan_task(controller_output: Option<JsonValue>) -> RunTask {
        let task = RunTaskSpec::ParameterScan {
            parameter: ParameterScanParameterSpec {
                name: "scale".to_string(),
                values: vec![
                    toml::Value::Float(0.0),
                    toml::Value::Float(1.0),
                    toml::Value::Float(2.0),
                ],
            },
            measurement: ParameterScanMeasurementSpec {
                source_task: "sample".to_string(),
            },
            trial_run_toml: "name = \"trial\"".to_string(),
            max_concurrent_runs: 1,
        };
        RunTask {
            id: 1,
            run_id: 1,
            name: "scan".to_string(),
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
                name: Some("scan".to_string()),
                task,
            })
            .expect("task toml"),
            measurement_output: None,
            controller_output,
        }
    }

    fn panel_ctx<'a>(task: &'a RunTask, panel_state: &'a JsonValue) -> TaskPanelContext<'a> {
        TaskPanelContext {
            task,
            source: super::super::TaskPanelCurrentSource::Empty,
            panel_state,
            run_target: None,
            completed_samples_per_second: None,
            smoothed_eta_seconds: None,
            sampler_engine_diagnostics: None,
        }
    }

    #[test]
    fn scan_mean_plot_uses_numeric_parameter_values_and_mean_results() {
        let task = scan_task(Some(json!({
            "parameter_name": "scale",
            "completed_points": 2,
            "total_points": 3,
            "points": [
                {
                    "index": 0,
                    "parameter_value": 0.0,
                    "child_run_id": 11,
                    "status": "completed",
                    "measurement": {
                        "status": "completed",
                        "results": [
                            {"name": "mean", "value": 1.0, "uncertainty": 0.1, "sample_count": 100}
                        ]
                    }
                },
                {
                    "index": 1,
                    "parameter_value": 1.0,
                    "child_run_id": 12,
                    "status": "completed",
                    "measurement": {
                        "status": "completed",
                        "results": [
                            {"name": "mean", "value": 1.5, "uncertainty": 0.2, "sample_count": 200}
                        ]
                    }
                },
                {
                    "index": 2,
                    "parameter_value": 2.0,
                    "child_run_id": null,
                    "status": "pending"
                }
            ]
        })));
        let panel = scan_mean_projector()
            .current(&panel_ctx(&task, &JsonValue::Null))
            .expect("projector")
            .expect("panel");
        let PanelState::ScalarTimeseries { points, .. } = panel else {
            panic!("expected scalar timeseries");
        };
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].x, 0.0);
        assert_eq!(points[0].y, 1.0);
        assert_eq!(points[0].y_min, Some(0.9));
        assert_eq!(points[1].x, 1.0);
        assert_eq!(points[1].x_completed_samples_total, Some(200.0));
    }

    #[test]
    fn scan_progress_falls_back_to_task_parameter_count_before_first_tick() {
        let task = scan_task(None);
        let panel = scan_progress_projector()
            .current(&panel_ctx(&task, &JsonValue::Null))
            .expect("projector")
            .expect("panel");
        let PanelState::Progress { current, total, .. } = panel else {
            panic!("expected progress panel");
        };
        assert_eq!(current, 0.0);
        assert_eq!(total, Some(3.0));
    }
}
