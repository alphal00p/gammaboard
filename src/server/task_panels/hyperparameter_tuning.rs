use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, PlotPoint, PlotSeries, multi_timeseries_panel,
    panel_spec, progress_panel, table_panel_with_payload_and_options, with_panel_width,
};
use serde_json::{Value as JsonValue, json};

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
            let points = tuning_trials(ctx)
                .into_iter()
                .filter_map(trial_to_plot_point)
                .collect::<Vec<_>>();
            Ok(Some(multi_timeseries_panel(
                TUNING_OBJECTIVE_PANEL_ID,
                vec![PlotSeries {
                    id: "objective".to_string(),
                    label: "Objective".to_string(),
                    color: None,
                    smooth: None,
                    points,
                }],
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
            let rows = tuning_trials(ctx)
                .into_iter()
                .map(trial_to_table_row)
                .collect::<Vec<_>>();
            Ok(Some(table_panel_with_payload_and_options(
                TUNING_TRIALS_PANEL_ID,
                vec![
                    "index".to_string(),
                    "status".to_string(),
                    "run".to_string(),
                    "objective".to_string(),
                    "uncertainty".to_string(),
                    "parameters".to_string(),
                    "failure".to_string(),
                ],
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

fn tuning_trials<'a>(ctx: &'a TaskPanelContext<'_>) -> Vec<&'a JsonValue> {
    ctx.task
        .controller_output
        .as_ref()
        .and_then(|output| output.get("trials"))
        .and_then(JsonValue::as_array)
        .map(|trials| trials.iter().collect())
        .unwrap_or_default()
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

fn trial_to_table_row(trial: &JsonValue) -> Vec<JsonValue> {
    vec![
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
        trial.get("parameters").cloned().unwrap_or(JsonValue::Null),
        trial
            .get("failure_reason")
            .cloned()
            .unwrap_or(JsonValue::Null),
    ]
}
