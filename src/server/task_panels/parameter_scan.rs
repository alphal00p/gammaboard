use super::controller::{child_table_payload, progress_projector};
use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    ImageColorMode, ImageNormalizationMode, PanelHistoryMode, PanelKind, PanelState, PanelWidth,
    PlotPoint, PlotSeries, multi_timeseries_panel, sized_panel_spec,
    table_panel_with_payload_and_options,
};
use serde_json::{Map, Value as JsonValue, json};
use std::collections::{BTreeMap, BTreeSet};
const SCAN_PROGRESS_PANEL_ID: &str = "scan_progress";
const SCAN_MEAN_PANEL_ID: &str = "scan_mean";
const SCAN_MEAN_HEATMAP_PANEL_ID: &str = "scan_mean_heatmap";
const SCAN_POINTS_PANEL_ID: &str = "scan_points";

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    vec![
        scan_progress_projector(),
        scan_measurements_projector(),
        scan_heatmap_projector(),
        scan_points_projector(),
    ]
}

fn scan_progress_projector() -> TaskPanelProjector {
    progress_projector(
        SCAN_PROGRESS_PANEL_ID,
        "Scan Progress",
        |output| {
            output
                .parameter_scan()
                .map(|output| output.completed_points as f64)
        },
        |output| {
            output
                .parameter_scan()
                .map(|output| output.total_points as f64)
        },
        "points",
        |ctx| total_points_from_task(ctx).map(|value| value as f64),
    )
}

fn scan_measurements_projector() -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            SCAN_MEAN_PANEL_ID,
            "Observables by Parameter",
            PanelKind::MultiTimeseries,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        |ctx| {
            let parameter_names = scan_parameter_names(ctx);
            let series = scan_points(ctx)
                .map(|points| {
                    if parameter_names.len() == 1 {
                        build_measurement_series(&points, &parameter_names[0], |result| {
                            result.get("name").and_then(JsonValue::as_str) == Some("mean")
                        })
                    } else {
                        Vec::new()
                    }
                })
                .unwrap_or_default();
            if series.is_empty() {
                return Ok(None);
            }
            Ok(Some(multi_timeseries_panel(SCAN_MEAN_PANEL_ID, series)))
        },
        |_ctx| Ok(None),
    )
}

fn scan_heatmap_projector() -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            SCAN_MEAN_HEATMAP_PANEL_ID,
            "Observable Heatmap",
            PanelKind::Image2d,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        |ctx| {
            let parameter_names = scan_parameter_names(ctx);
            let Some(points) = scan_points(ctx) else {
                return Ok(None);
            };
            Ok(scan_mean_heatmap_panel(&points, &parameter_names))
        },
        |_ctx| Ok(None),
    )
}

fn scan_points_projector() -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            SCAN_POINTS_PANEL_ID,
            "Scan Points",
            PanelKind::Table,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        |ctx| {
            let parameter_names = scan_parameter_names(ctx);
            let rows = scan_points(ctx)
                .map(|points| {
                    points
                        .iter()
                        .flat_map(|point| scan_point_to_table_rows(point, &parameter_names))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let value_column = scan_table_value_column(parameter_names.len());
            let mut columns = vec!["index".to_string()];
            columns.extend(parameter_names.iter().cloned());
            columns.extend([
                "status".to_string(),
                "run".to_string(),
                "component".to_string(),
                "value".to_string(),
                "uncertainty".to_string(),
                "samples".to_string(),
            ]);
            let payload = child_table_payload(
                &rows,
                value_column,
                Map::from_iter([("parameters".to_string(), json!(scan_parameter_names(ctx)))]),
            );

            let run_column = parameter_names.len() + 2;
            let visible_column_indices = (0..columns.len())
                .filter(|index| *index != run_column)
                .collect();
            Ok(Some(table_panel_with_payload_and_options(
                SCAN_POINTS_PANEL_ID,
                columns,
                rows,
                Some(payload),
                crate::server::panels::TableStateOptions {
                    visible_column_indices,
                    row_keys: None,
                },
            )))
        },
        |_ctx| Ok(None),
    )
}

fn scan_mean_heatmap_panel(points: &[JsonValue], parameter_names: &[String]) -> Option<PanelState> {
    if parameter_names.len() != 2 {
        return None;
    }
    let x_name = &parameter_names[0];
    let y_name = &parameter_names[1];
    let mut x_values = BTreeSet::<OrderedF64>::new();
    let mut y_values = BTreeSet::<OrderedF64>::new();
    let mut values_by_coord = BTreeMap::<(OrderedF64, OrderedF64), f64>::new();
    for point in points {
        let Some(x) =
            scan_point_parameter_value(point, x_name).and_then(|value| json_number(&value))
        else {
            continue;
        };
        let Some(y) =
            scan_point_parameter_value(point, y_name).and_then(|value| json_number(&value))
        else {
            continue;
        };
        let Some(result) = measurement_results(point).find(|result| {
            result.get("name").and_then(JsonValue::as_str) == Some("mean")
                && result.get("value").and_then(JsonValue::as_f64).is_some()
        }) else {
            continue;
        };
        let Some(value) = result.get("value").and_then(JsonValue::as_f64) else {
            continue;
        };
        let x = OrderedF64(x);
        let y = OrderedF64(y);
        x_values.insert(x);
        y_values.insert(y);
        values_by_coord.insert((x, y), value);
    }
    if x_values.len() < 2 || y_values.len() < 2 {
        return None;
    }
    let x_values = x_values.into_iter().collect::<Vec<_>>();
    let y_values = y_values.into_iter().collect::<Vec<_>>();
    let width = x_values.len();
    let height = y_values.len();
    let mut values = Vec::with_capacity(width * height);
    let mut invalid_indices = Vec::new();
    for (row, y) in y_values.iter().enumerate() {
        for (col, x) in x_values.iter().enumerate() {
            match values_by_coord.get(&(*x, *y)).copied() {
                Some(value) if value.is_finite() => values.push(value as f32),
                _ => {
                    invalid_indices.push(row * width + col);
                    values.push(f32::NAN);
                }
            }
        }
    }
    Some(PanelState::Image2d {
        panel_id: SCAN_MEAN_HEATMAP_PANEL_ID.to_string(),
        width,
        height,
        values,
        imag_values: None,
        invalid_indices: if invalid_indices.is_empty() {
            None
        } else {
            Some(invalid_indices)
        },
        x_range: [x_values.first()?.0, x_values.last()?.0],
        y_range: [y_values.first()?.0, y_values.last()?.0],
        color_mode: ImageColorMode::ScalarHeatmap,
        normalization_mode: ImageNormalizationMode::MinMax,
        metric_label: Some("mean".to_string()),
        metric_mode: None,
        x_label: Some(x_name.clone()),
        y_label: Some(y_name.clone()),
    })
}

fn scan_points(ctx: &TaskPanelContext<'_>) -> Option<Vec<JsonValue>> {
    ctx.task
        .controller_output
        .as_ref()
        .and_then(crate::core::ControllerTaskOutput::parameter_scan)
        .map(|output| {
            output
                .points
                .iter()
                .filter_map(|point| serde_json::to_value(point).ok())
                .collect()
        })
}

fn scan_table_value_column(parameter_count: usize) -> usize {
    parameter_count + 4
}

fn build_measurement_series(
    points: &[JsonValue],
    parameter_name: &str,
    include_result: impl Fn(&JsonValue) -> bool,
) -> Vec<PlotSeries> {
    let mut series_by_id = BTreeMap::<String, PlotSeries>::new();
    for point in points {
        let Some(x) =
            scan_point_parameter_value(point, parameter_name).and_then(|value| json_number(&value))
        else {
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

fn scan_point_to_table_rows(point: &JsonValue, parameter_names: &[String]) -> Vec<Vec<JsonValue>> {
    let mut common = vec![point.get("index").cloned().unwrap_or(JsonValue::Null)];
    common.extend(
        parameter_names
            .iter()
            .map(|name| scan_point_parameter_value(point, name).unwrap_or(JsonValue::Null)),
    );
    common.extend([
        point.get("status").cloned().unwrap_or(JsonValue::Null),
        point
            .get("child_run_id")
            .cloned()
            .unwrap_or(JsonValue::Null),
    ]);
    let rows = measurement_results(point)
        .map(|result| {
            let mut row = common.to_vec();
            row.extend([
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
        crate::core::RunTaskSpec::ParameterScan { .. } => ctx
            .task
            .task
            .nr_expected_samples()
            .map(|value| value as usize),
        _ => None,
    }
}

fn scan_parameter_names(ctx: &TaskPanelContext<'_>) -> Vec<String> {
    if let Some(output) = ctx
        .task
        .controller_output
        .as_ref()
        .and_then(crate::core::ControllerTaskOutput::parameter_scan)
    {
        return output.parameters.clone();
    }
    match &ctx.task.task {
        crate::core::RunTaskSpec::ParameterScan { parameters, .. } => parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn scan_point_parameter_value(point: &JsonValue, parameter_name: &str) -> Option<JsonValue> {
    point
        .get("parameter_values")
        .and_then(|values| values.get(parameter_name))
        .cloned()
        .or_else(|| point.get("parameter_value").cloned())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedF64(f64);

impl Eq for OrderedF64 {}

impl PartialOrd for OrderedF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        MeasurementSpec, ParameterScanParameterSpec, RunTask, RunTaskInput, RunTaskSpec,
        RunTaskState, canonical_task_toml,
    };
    use crate::server::panels::PanelState;
    use chrono::Utc;

    fn scan_task(controller_output: Option<JsonValue>) -> RunTask {
        let task = RunTaskSpec::ParameterScan {
            parameters: vec![ParameterScanParameterSpec {
                name: "scale".to_string(),
                source: crate::core::ParameterValueSourceSpec {
                    values: vec![
                        toml::Value::Float(0.0),
                        toml::Value::Float(1.0),
                        toml::Value::Float(2.0),
                    ],
                    ..Default::default()
                },
            }],
            measurement: MeasurementSpec::default(),
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
            nr_produced_samples_including_children: 0,
            nr_completed_samples_including_children: 0,
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
            controller_output: controller_output.map(|output| {
                serde_json::from_value(output).expect("typed scan controller output")
            }),
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
            "parameters": ["scale"],
            "completed_points": 2,
            "running_points": 0,
            "total_points": 3,
            "points": [
                {
                    "index": 0,
                    "parameter_values": {"scale": 0.0},
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
                    "parameter_values": {"scale": 1.0},
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
                    "parameter_values": {"scale": 2.0},
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
    fn scan_observable_plot_combines_components_and_includes_error_bars() {
        let task = scan_task(Some(json!({
            "parameters": ["scale"],
            "completed_points": 1,
            "running_points": 0,
            "total_points": 1,
            "points": [
                {
                    "index": 0,
                    "parameter_values": {"scale": 2.0},
                    "child_run_id": 11,
                    "status": "completed",
                    "measurement": {
                        "status": "completed",
                        "results": [
                            {"name": "mean", "component": "real", "value": 2.0, "uncertainty": 0.2, "sample_count": 100},
                            {"name": "mean", "component": "imag", "value": -1.0, "uncertainty": 0.1, "sample_count": 100},
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
        assert_eq!(series.len(), 2);
        let real = series
            .iter()
            .find(|series| series.id == "mean:real")
            .unwrap();
        assert_eq!(real.points[0].y, 2.0);
        assert_eq!(real.points[0].y_min, Some(1.8));
        assert_eq!(real.points[0].y_max, Some(2.2));
        let imag = series
            .iter()
            .find(|series| series.id == "mean:imag")
            .unwrap();
        assert_eq!(imag.points[0].y, -1.0);
        assert_eq!(imag.points[0].y_min, Some(-1.1));
        assert_eq!(imag.points[0].y_max, Some(-0.9));
    }

    #[test]
    fn scan_heatmap_uses_two_numeric_parameters() {
        let task = scan_task(Some(json!({
            "parameters": ["scale", "offset"],
            "completed_points": 4,
            "running_points": 0,
            "total_points": 4,
            "points": [
                {
                    "index": 0,
                    "parameter_values": {"scale": 0.0, "offset": 1.0},
                    "status": "completed",
                    "measurement": {"status": "completed", "results": [{"name": "mean", "value": 1.0, "sample_count": 0}]}
                },
                {
                    "index": 1,
                    "parameter_values": {"scale": 1.0, "offset": 1.0},
                    "status": "completed",
                    "measurement": {"status": "completed", "results": [{"name": "mean", "value": 1.5, "sample_count": 0}]}
                },
                {
                    "index": 2,
                    "parameter_values": {"scale": 0.0, "offset": 2.0},
                    "status": "completed",
                    "measurement": {"status": "completed", "results": [{"name": "mean", "value": 2.0, "sample_count": 0}]}
                },
                {
                    "index": 3,
                    "parameter_values": {"scale": 1.0, "offset": 2.0},
                    "status": "completed",
                    "measurement": {"status": "completed", "results": [{"name": "mean", "value": 2.5, "sample_count": 0}]}
                }
            ]
        })));
        let panel = scan_heatmap_projector()
            .current(&panel_ctx(&task, &JsonValue::Null))
            .expect("projector")
            .expect("panel");
        let PanelState::Image2d {
            width,
            height,
            values,
            x_label,
            y_label,
            ..
        } = panel
        else {
            panic!("expected image panel");
        };
        assert_eq!(width, 2);
        assert_eq!(height, 2);
        assert_eq!(values, vec![1.0, 1.5, 2.0, 2.5]);
        assert_eq!(x_label.as_deref(), Some("scale"));
        assert_eq!(y_label.as_deref(), Some("offset"));
    }

    #[test]
    fn scan_table_marks_min_and_max_values() {
        let task = scan_task(Some(json!({
            "parameters": ["scale"],
            "completed_points": 2,
            "running_points": 0,
            "total_points": 2,
            "points": [
                {
                    "index": 0,
                    "parameter_values": {"scale": 0.0},
                    "child_run_id": 11,
                    "status": "completed",
                    "measurement": {"status": "completed", "results": [{"name": "mean", "value": 3.0, "sample_count": 0}]}
                },
                {
                    "index": 1,
                    "parameter_values": {"scale": 1.0},
                    "child_run_id": 12,
                    "status": "completed",
                    "measurement": {"status": "completed", "results": [{"name": "mean", "value": 2.0, "sample_count": 0}]}
                }
            ]
        })));
        let panel = scan_points_projector()
            .current(&panel_ctx(&task, &JsonValue::Null))
            .expect("projector")
            .expect("panel");
        let PanelState::Table { payload, .. } = panel else {
            panic!("expected table panel");
        };
        let row_tones = payload
            .as_ref()
            .and_then(|payload| payload.get("row_tones"))
            .and_then(JsonValue::as_array)
            .expect("row tones");
        assert_eq!(row_tones, &vec![json!("max"), json!("min")]);
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
