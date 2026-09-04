use super::controller::progress_projector;
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
            let output = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(crate::core::ControllerTaskOutput::integration_campaign);
            let children = output.map_or(&[][..], |output| output.children.as_slice());
            let result_keys = campaign_result_keys(children);
            let total_variance = children
                .iter()
                .try_fold(0.0, |sum, child| Some(sum + child_variance(child)?));
            let rows = children
                .iter()
                .map(|child| child_row(child, campaign_stopped, &result_keys, total_variance))
                .collect::<Vec<_>>();
            let mut columns = vec![
                "name".to_string(),
                "status".to_string(),
                "run".to_string(),
                "coefficient".to_string(),
            ];
            for key in &result_keys {
                let label = result_key_label(key, result_keys.len());
                columns.push(label.clone());
                columns.push(format!("{label} uncertainty"));
            }
            columns.extend([
                "variance contribution (%)".to_string(),
                "samples".to_string(),
            ]);
            let payload = json!({
                "row_action": { "kind": "select_run", "column": "run" }
            });
            let visible_column_indices = (0..columns.len()).filter(|index| *index != 2).collect();
            Ok(Some(table_panel_with_payload_and_options(
                CHILDREN_ID,
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

type ResultKey = (crate::core::AccumulatorMetricName, Option<String>);

fn measurement_results(
    child: &crate::core::IntegrationCampaignChildOutput,
) -> Option<&[crate::core::MeasurementResult]> {
    match child.child.measurement.as_ref()? {
        crate::core::TaskMeasurementOutput::Completed { results } => Some(results),
        crate::core::TaskMeasurementOutput::Failed { .. } => None,
    }
}

fn campaign_result_keys(
    children: &[crate::core::IntegrationCampaignChildOutput],
) -> Vec<ResultKey> {
    let mut keys = Vec::new();
    for result in children.iter().filter_map(measurement_results).flatten() {
        let key = (result.name, result.component.clone());
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn result_key_label(key: &ResultKey, key_count: usize) -> String {
    key.1.clone().unwrap_or_else(|| {
        if key_count == 1 {
            "estimate".to_string()
        } else {
            format!("{:?}", key.0).to_lowercase()
        }
    })
}

fn child_variance(child: &crate::core::IntegrationCampaignChildOutput) -> Option<f64> {
    measurement_results(child)?
        .iter()
        .try_fold(0.0, |sum, result| {
            result
                .uncertainty
                .map(|uncertainty| sum + child.coefficient.powi(2) * uncertainty.powi(2))
        })
}

fn child_row(
    child: &crate::core::IntegrationCampaignChildOutput,
    campaign_stopped: bool,
    result_keys: &[ResultKey],
    total_variance: Option<f64>,
) -> Vec<JsonValue> {
    let status = if campaign_stopped
        && matches!(
            child.child.status,
            crate::core::ControllerChildState::Planned
                | crate::core::ControllerChildState::Pending
                | crate::core::ControllerChildState::Active
        ) {
        json!("stopped")
    } else {
        match (child.child.status, child.selected) {
            (crate::core::ControllerChildState::Active, true) => json!("running"),
            (crate::core::ControllerChildState::Active, false) => json!("waiting"),
            (
                crate::core::ControllerChildState::Planned
                | crate::core::ControllerChildState::Pending,
                true,
            ) => json!("starting"),
            (status, _) => json!(status),
        }
    };
    let mut row = vec![
        json!(child.name),
        status,
        json!(child.child.child_run_id),
        json!(child.coefficient),
    ];
    let results = measurement_results(child).unwrap_or_default();
    for key in result_keys {
        let result = results
            .iter()
            .find(|result| result.name == key.0 && result.component == key.1);
        row.push(result.map_or(JsonValue::Null, |result| json!(result.value)));
        row.push(
            result
                .and_then(|result| result.uncertainty)
                .map_or(JsonValue::Null, |uncertainty| json!(uncertainty)),
        );
    }
    row.push(
        child_variance(child)
            .zip(total_variance.filter(|total| total.is_finite() && *total > 0.0))
            .map_or(JsonValue::Null, |(variance, total)| {
                json!(100.0 * variance / total)
            }),
    );
    row.push(
        results
            .iter()
            .map(|result| result.sample_count)
            .max()
            .map_or(JsonValue::Null, |samples| json!(samples)),
    );
    row
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

    fn complex_measurement(
        real: (f64, f64),
        imag: (f64, f64),
        samples: i64,
    ) -> TaskMeasurementOutput {
        TaskMeasurementOutput::Completed {
            results: vec![
                MeasurementResult {
                    name: AccumulatorMetricName::Mean,
                    component: Some("real".to_string()),
                    value: real.0,
                    uncertainty: Some(real.1),
                    sample_count: samples,
                },
                MeasurementResult {
                    name: AccumulatorMetricName::Mean,
                    component: Some("imag".to_string()),
                    value: imag.0,
                    uncertainty: Some(imag.1),
                    sample_count: samples,
                },
            ],
        }
    }

    #[test]
    fn stopped_campaign_does_not_show_active_child() {
        let child = child(None);
        let children = [child];

        assert_eq!(
            child_row(&children[0], false, &[], None)[1],
            json!("waiting")
        );
        assert_eq!(
            child_row(&children[0], true, &[], None)[1],
            json!("stopped")
        );
    }

    #[test]
    fn complex_measurement_exposes_one_row_with_component_columns() {
        let mut campaign_child = child(Some(complex_measurement((3.0, 0.4), (-2.0, 0.2), 10)));
        campaign_child.selected = true;
        let children = [campaign_child];
        let keys = campaign_result_keys(&children);
        let variance = child_variance(&children[0]).unwrap();
        let row = child_row(&children[0], false, &keys, Some(variance));

        assert_eq!(row.len(), 10);
        assert_eq!(row[1], json!("running"));
        assert_eq!(row[4], json!(3.0));
        assert_eq!(row[5], json!(0.4));
        assert_eq!(row[6], json!(-2.0));
        assert_eq!(row[7], json!(0.2));
        assert_eq!(row[8], json!(100.0));
        assert_eq!(row[9], json!(10));
    }

    #[test]
    fn variance_contributions_are_percentages_of_the_campaign_total() {
        let first = child(Some(complex_measurement((3.0, 0.4), (-2.0, 0.2), 10)));
        let mut second = child(Some(complex_measurement((1.0, 0.3), (4.0, 0.1), 20)));
        second.name = "other".to_string();
        second.coefficient = 1.0;
        let children = [first, second];
        let keys = campaign_result_keys(&children);
        let total_variance = children.iter().filter_map(child_variance).sum();

        let first_row = child_row(&children[0], false, &keys, Some(total_variance));
        let second_row = child_row(&children[1], false, &keys, Some(total_variance));
        assert!((first_row[8].as_f64().unwrap() - 88.888_888_888_888_89).abs() < 1e-12);
        assert!((second_row[8].as_f64().unwrap() - 11.111_111_111_111_11).abs() < 1e-12);
    }
}
