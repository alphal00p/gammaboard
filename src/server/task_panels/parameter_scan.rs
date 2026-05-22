use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, PlotPoint, PlotSeries, multi_timeseries_panel,
    panel_spec, progress_panel, table_panel_with_payload_and_options, with_panel_width,
};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;

const SCAN_PROGRESS_PANEL_ID: &str = "scan_progress";
const SCAN_MEAN_PANEL_ID: &str = "scan_mean";
const SCAN_POINTS_PANEL_ID: &str = "scan_points";

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    vec![
        scan_progress_projector(),
        scan_measurements_projector(),
        scan_points_projector(),
    ]
}

fn scan_progress_projector() -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            SCAN_PROGRESS_PANEL_ID,
            "Scan Progress",
            PanelKind::Progress,
            PanelHistoryMode::None,
        ),
        |ctx| {
            let Some(output) = ctx.task.controller_output.as_ref() else {
                return Ok(Some(progress_panel(
                    SCAN_PROGRESS_PANEL_ID,
                    0.0,
                    total_points_from_task(ctx).map(|value| value as f64),
                    Some("points"),
                    None,
                )));
            };
            Ok(Some(progress_panel(
                SCAN_PROGRESS_PANEL_ID,
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

fn scan_measurements_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                SCAN_MEAN_PANEL_ID,
                "Central Values over Parameter",
                PanelKind::MultiTimeseries,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let series = scan_points(ctx)
                .map(|points| {
                    build_measurement_series(points, |result| {
                        result.get("name").and_then(JsonValue::as_str) == Some("mean")
                    })
                })
                .unwrap_or_default();
            Ok(Some(multi_timeseries_panel(SCAN_MEAN_PANEL_ID, series)))
        },
        |_ctx| Ok(None),
    )
}

fn scan_points_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                SCAN_POINTS_PANEL_ID,
                "Scan Points",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let rows = scan_points(ctx)
                .map(|points| {
                    points
                        .iter()
                        .flat_map(scan_point_to_table_rows)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(Some(table_panel_with_payload_and_options(
                SCAN_POINTS_PANEL_ID,
                vec![
                    "index".to_string(),
                    "parameter".to_string(),
                    "status".to_string(),
                    "run".to_string(),
                    "metric".to_string(),
                    "component".to_string(),
                    "value".to_string(),
                    "uncertainty".to_string(),
                    "samples".to_string(),
                ],
                rows,
                ctx.task.controller_output.as_ref().map(|output| {
                    json!({
                        "parameter_name": output.get("parameter_name").cloned().unwrap_or(JsonValue::Null),
                        "row_action": {
                            "kind": "select_run",
                            "column": "run",
                        },
                    })
                }),
                Default::default(),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn scan_points<'a>(ctx: &'a TaskPanelContext<'_>) -> Option<&'a Vec<JsonValue>> {
    ctx.task
        .controller_output
        .as_ref()
        .and_then(|output| output.get("points"))
        .and_then(JsonValue::as_array)
}

fn build_measurement_series(
    points: &[JsonValue],
    include_result: impl Fn(&JsonValue) -> bool,
) -> Vec<PlotSeries> {
    let mut series_by_id = BTreeMap::<String, PlotSeries>::new();
    for point in points {
        let Some(x) = point.get("parameter_value").and_then(json_number) else {
            continue;
        };
        for result in measurement_results(point).filter(|result| include_result(result)) {
            let Some(id) = measurement_result_series_id(result) else {
                continue;
            };
            let Some(plot_point) = scan_result_to_plot_point(x, result) else {
                continue;
            };
            series_by_id
                .entry(id.clone())
                .or_insert_with(|| PlotSeries {
                    id: id.clone(),
                    label: measurement_result_label(result),
                    color: None,
                    smooth: None,
                    points: Vec::new(),
                })
                .points
                .push(plot_point);
        }
    }
    series_by_id.into_values().collect()
}

fn scan_result_to_plot_point(x: f64, result: &JsonValue) -> Option<PlotPoint> {
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

fn scan_point_to_table_rows(point: &JsonValue) -> Vec<Vec<JsonValue>> {
    let common = [
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
    ];
    let rows = measurement_results(point)
        .map(|result| {
            let mut row = common.to_vec();
            row.extend([
                result.get("name").cloned().unwrap_or(JsonValue::Null),
                result.get("component").cloned().unwrap_or(JsonValue::Null),
                result.get("value").cloned().unwrap_or(JsonValue::Null),
                result
                    .get("uncertainty")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                result
                    .get("sample_count")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            ]);
            row
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        let mut row = common.to_vec();
        row.extend([
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
        ]);
        vec![row]
    } else {
        rows
    }
}

fn measurement_results(point: &JsonValue) -> impl Iterator<Item = &JsonValue> {
    point
        .get("measurement")
        .and_then(|measurement| measurement.get("results"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
}

fn measurement_result_series_id(result: &JsonValue) -> Option<String> {
    let name = result.get("name")?.as_str()?;
    let component = result.get("component").and_then(JsonValue::as_str);
    Some(match component {
        Some(component) if !component.is_empty() => format!("{name}:{component}"),
        _ => name.to_string(),
    })
}

fn measurement_result_label(result: &JsonValue) -> String {
    let name = result
        .get("name")
        .and_then(JsonValue::as_str)
        .unwrap_or("measurement");
    match result.get("component").and_then(JsonValue::as_str) {
        Some(component) if !component.is_empty() => format!("{component} {name}"),
        _ => name.to_string(),
    }
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
        let panel = scan_measurements_projector()
            .current(&panel_ctx(&task, &JsonValue::Null))
            .expect("projector")
            .expect("panel");
        let PanelState::MultiTimeseries { series, .. } = panel else {
            panic!("expected multi timeseries");
        };
        assert_eq!(series.len(), 1);
        let points = &series[0].points;
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].x, 0.0);
        assert_eq!(points[0].y, 1.0);
        assert_eq!(points[0].y_min, Some(0.9));
        assert_eq!(points[1].x, 1.0);
        assert_eq!(points[1].x_completed_samples_total, Some(200.0));
    }

    #[test]
    fn scan_mean_plot_uses_one_series_per_component() {
        let task = scan_task(Some(json!({
            "parameter_name": "scale",
            "completed_points": 1,
            "total_points": 1,
            "points": [
                {
                    "index": 0,
                    "parameter_value": 2.0,
                    "child_run_id": 11,
                    "status": "completed",
                    "measurement": {
                        "status": "completed",
                        "results": [
                            {"name": "mean", "component": "real", "value": 2.0, "sample_count": 100},
                            {"name": "mean", "component": "imag", "value": -1.0, "sample_count": 100},
                            {"name": "variance", "component": "real", "value": 0.5, "sample_count": 100}
                        ]
                    }
                }
            ]
        })));
        let panel = scan_measurements_projector()
            .current(&panel_ctx(&task, &JsonValue::Null))
            .expect("projector")
            .expect("panel");
        let PanelState::MultiTimeseries { series, .. } = panel else {
            panic!("expected multi timeseries");
        };
        assert_eq!(
            series
                .iter()
                .map(|series| series.id.as_str())
                .collect::<Vec<_>>(),
            vec!["mean:imag", "mean:real"]
        );
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
