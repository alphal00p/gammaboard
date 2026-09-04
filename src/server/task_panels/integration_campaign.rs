use super::controller::{child_table_payload, progress_projector};
use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelProjector, panel_projector,
    panel_projector_with_source,
};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, key_value, key_value_panel, panel_spec,
    sized_panel_spec, table_panel_with_payload_and_options,
};
use serde_json::{Value as JsonValue, json};

const PROGRESS_ID: &str = "campaign_progress";
const SUMMARY_ID: &str = "campaign_combined_result";
const HISTOGRAMS_ID: &str = "campaign_histograms";
const CHILDREN_ID: &str = "campaign_children";

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(
            PROGRESS_ID,
            "Campaign Progress",
            |output| {
                output
                    .integration_campaign()
                    .map(|output| output.total_samples as f64)
            },
            |_output| None,
            "samples",
            campaign_max_samples,
        ),
        summary_projector(),
        histograms_projector(),
        children_projector(),
    ]
}

fn histograms_projector() -> TaskPanelProjector {
    panel_projector_with_source(
        sized_panel_spec(
            HISTOGRAMS_ID,
            "Combined Observables",
            PanelKind::Table,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedAlways,
        |ctx| {
            let Some(payload) = ctx
                .source
                .persisted()
                .and_then(|result| result.get("observables"))
                .cloned()
            else {
                return Ok(None);
            };
            Ok(super::observable::histogram_bundle_panel(
                HISTOGRAMS_ID,
                "Combined Observable",
                payload,
            ))
        },
        |_ctx| Ok(None),
    )
}

fn campaign_max_samples(ctx: &TaskPanelContext<'_>) -> Option<f64> {
    if ctx.task.state == crate::core::RunTaskState::Completed {
        return ctx
            .task
            .controller_output
            .as_ref()
            .and_then(crate::core::ControllerTaskOutput::integration_campaign)
            .map(|output| output.total_samples as f64);
    }
    match &ctx.task.task {
        crate::core::RunTaskSpec::IntegrationCampaign { stop_condition, .. } => {
            stop_condition.max_total_samples.map(|value| value as f64)
        }
        _ => None,
    }
}

fn summary_projector() -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            SUMMARY_ID,
            "Result",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
        ),
        |ctx| {
            let output = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(crate::core::ControllerTaskOutput::integration_campaign);
            let results = output
                .and_then(|output| output.combined_measurement.as_ref())
                .and_then(|measurement| match measurement {
                    crate::core::TaskMeasurementOutput::Completed { results } => Some(results),
                    crate::core::TaskMeasurementOutput::Failed { .. } => None,
                });
            let mut entries = results
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, result)| {
                    let component = result.component.as_deref();
                    let key = component
                        .map(str::to_string)
                        .unwrap_or_else(|| index.to_string());
                    key_value(
                        &format!("estimate_{key}"),
                        match component {
                            Some("real") => "Real",
                            Some("imag") => "Imaginary",
                            Some(component) => component,
                            None => "Estimate",
                        },
                        result.uncertainty.map_or_else(
                            || json!(result.value),
                            |error| {
                                json!({
                                    "kind": "estimate",
                                    "value": result.value,
                                    "error": error,
                                })
                            },
                        ),
                    )
                })
                .collect::<Vec<_>>();
            entries.push(key_value(
                "samples",
                "Total Samples",
                output
                    .map(|output| json!(output.total_samples))
                    .unwrap_or(JsonValue::Null),
            ));
            Ok(Some(key_value_panel(SUMMARY_ID, entries)))
        },
        |_ctx| Ok(None),
    )
}

fn children_projector() -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            CHILDREN_ID,
            "Campaign Sub-runs",
            PanelKind::Table,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        |ctx| {
            let campaign_stopped = matches!(
                ctx.task.state,
                crate::core::RunTaskState::Completed | crate::core::RunTaskState::Failed
            );
            let rows = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(crate::core::ControllerTaskOutput::integration_campaign)
                .map(|output| {
                    output
                        .children
                        .iter()
                        .flat_map(|child| child_rows(child, campaign_stopped))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let payload = child_table_payload(&rows, 8, Default::default());
            Ok(Some(table_panel_with_payload_and_options(
                CHILDREN_ID,
                vec![
                    "name".to_string(),
                    "status".to_string(),
                    "selected".to_string(),
                    "run".to_string(),
                    "coefficient".to_string(),
                    "component".to_string(),
                    "value".to_string(),
                    "uncertainty".to_string(),
                    "variance contribution".to_string(),
                    "samples".to_string(),
                ],
                rows,
                Some(payload),
                crate::server::panels::TableStateOptions {
                    visible_column_indices: vec![0, 1, 2, 4, 5, 6, 7, 8, 9],
                    row_keys: None,
                },
            )))
        },
        |_ctx| Ok(None),
    )
}

fn child_rows(
    child: &crate::core::IntegrationCampaignChildOutput,
    campaign_stopped: bool,
) -> Vec<Vec<JsonValue>> {
    let results = child
        .child
        .measurement
        .as_ref()
        .and_then(|measurement| match measurement {
            crate::core::TaskMeasurementOutput::Completed { results } => Some(results.as_slice()),
            crate::core::TaskMeasurementOutput::Failed { .. } => None,
        });
    let common = vec![
        json!(child.name),
        if campaign_stopped
            && matches!(
                child.child.status,
                crate::core::ControllerChildState::Planned
                    | crate::core::ControllerChildState::Pending
                    | crate::core::ControllerChildState::Active
            )
        {
            json!("stopped")
        } else {
            json!(child.child.status)
        },
        json!(child.selected),
        json!(child.child.child_run_id),
        json!(child.coefficient),
    ];
    results
        .filter(|results| !results.is_empty())
        .map(|results| {
            results
                .iter()
                .map(|result| {
                    let uncertainty = result.uncertainty;
                    let mut row = common.clone();
                    row.extend([
                        json!(result.component),
                        json!(result.value),
                        uncertainty.map_or(JsonValue::Null, |value| json!(value)),
                        uncertainty.map_or(JsonValue::Null, |value| {
                            json!(child.coefficient.powi(2) * value.powi(2))
                        }),
                        json!(result.sample_count),
                    ]);
                    row
                })
                .collect()
        })
        .unwrap_or_else(|| {
            let mut row = common;
            row.extend(std::iter::repeat_n(JsonValue::Null, 5));
            vec![row]
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        AccumulatorMetricName, ControllerChildOutput, ControllerChildState,
        IntegrationCampaignChildOutput, MeasurementResult, TaskMeasurementOutput,
    };

    fn child(measurement: Option<TaskMeasurementOutput>) -> IntegrationCampaignChildOutput {
        IntegrationCampaignChildOutput {
            name: "graph".to_string(),
            coefficient: 2.0,
            child: ControllerChildOutput {
                child_run_id: Some(2),
                status: ControllerChildState::Active,
                result_source: None,
                completed_samples_per_second: None,
                measurement,
                failure_reason: None,
            },
            selected: false,
            score: None,
        }
    }

    #[test]
    fn stopped_campaign_does_not_show_active_child() {
        let child = child(None);

        assert_eq!(child_rows(&child, false)[0][1], json!("active"));
        assert_eq!(child_rows(&child, true)[0][1], json!("stopped"));
    }

    #[test]
    fn complex_measurement_exposes_one_row_per_component() {
        let measurement = TaskMeasurementOutput::Completed {
            results: vec![
                MeasurementResult {
                    name: AccumulatorMetricName::Mean,
                    component: Some("real".to_string()),
                    value: 3.0,
                    uncertainty: Some(0.4),
                    sample_count: 10,
                },
                MeasurementResult {
                    name: AccumulatorMetricName::Mean,
                    component: Some("imag".to_string()),
                    value: -2.0,
                    uncertainty: Some(0.2),
                    sample_count: 10,
                },
            ],
        };

        let rows = child_rows(&child(Some(measurement)), false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][5], json!("real"));
        assert_eq!(rows[0][6], json!(3.0));
        assert_eq!(rows[0][7], json!(0.4));
        assert!((rows[0][8].as_f64().unwrap() - 0.64).abs() < 1e-12);
        assert_eq!(rows[1][5], json!("imag"));
        assert_eq!(rows[1][6], json!(-2.0));
        assert_eq!(rows[1][7], json!(0.2));
        assert!((rows[1][8].as_f64().unwrap() - 0.16).abs() < 1e-12);
    }
}
