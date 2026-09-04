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
            let result = output
                .and_then(|output| output.combined_measurement.as_ref())
                .and_then(|measurement| match measurement {
                    crate::core::TaskMeasurementOutput::Completed { results } => results.first(),
                    crate::core::TaskMeasurementOutput::Failed { .. } => None,
                });
            let estimate = result.and_then(|result| {
                Some(json!({
                    "kind": "estimate",
                    "value": result.value,
                    "error": result.uncertainty?,
                }))
            });
            Ok(Some(key_value_panel(
                SUMMARY_ID,
                vec![
                    key_value("estimate", "Estimate", estimate.unwrap_or(JsonValue::Null)),
                    key_value(
                        "samples",
                        "Total Samples",
                        output
                            .map(|output| json!(output.total_samples))
                            .unwrap_or(JsonValue::Null),
                    ),
                ],
            )))
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
                        .map(|child| child_row(child, campaign_stopped))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let payload = child_table_payload(&rows, 7, Default::default());
            Ok(Some(table_panel_with_payload_and_options(
                CHILDREN_ID,
                vec![
                    "name".to_string(),
                    "status".to_string(),
                    "selected".to_string(),
                    "run".to_string(),
                    "coefficient".to_string(),
                    "value".to_string(),
                    "uncertainty".to_string(),
                    "variance contribution".to_string(),
                    "samples".to_string(),
                ],
                rows,
                Some(payload),
                crate::server::panels::TableStateOptions {
                    visible_column_indices: vec![0, 1, 2, 4, 5, 6, 7, 8],
                    row_keys: None,
                },
            )))
        },
        |_ctx| Ok(None),
    )
}

fn child_row(
    child: &crate::core::IntegrationCampaignChildOutput,
    campaign_stopped: bool,
) -> Vec<JsonValue> {
    let result = child
        .child
        .measurement
        .as_ref()
        .and_then(|measurement| match measurement {
            crate::core::TaskMeasurementOutput::Completed { results } => results.first(),
            crate::core::TaskMeasurementOutput::Failed { .. } => None,
        });
    let uncertainty = result.and_then(|result| result.uncertainty);
    vec![
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
        result
            .map(|result| json!(result.value))
            .unwrap_or(JsonValue::Null),
        uncertainty.map_or(JsonValue::Null, |value| json!(value)),
        uncertainty.map_or(JsonValue::Null, |value| {
            json!(child.coefficient.powi(2) * value.powi(2))
        }),
        result
            .map(|result| json!(result.sample_count))
            .unwrap_or(JsonValue::Null),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        ControllerChildOutput, ControllerChildState, IntegrationCampaignChildOutput,
    };

    #[test]
    fn stopped_campaign_does_not_show_active_child() {
        let child = IntegrationCampaignChildOutput {
            name: "graph".to_string(),
            coefficient: 1.0,
            child: ControllerChildOutput {
                child_run_id: Some(2),
                status: ControllerChildState::Active,
                result_source: None,
                completed_samples_per_second: None,
                measurement: None,
                failure_reason: None,
            },
            selected: false,
            score: None,
        };

        assert_eq!(child_row(&child, false)[1], json!("active"));
        assert_eq!(child_row(&child, true)[1], json!("stopped"));
    }
}
