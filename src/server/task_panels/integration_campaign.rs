use super::controller::progress_projector;
use super::controller_output::{measurement_results, select_run_payload};
use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelProjector, panel_projector,
    panel_projector_with_source,
};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelWidth, key_value, key_value_panel, panel_spec,
    sized_panel_spec, table_panel_with_payload_and_options,
};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeSet;

const PROGRESS_ID: &str = "campaign_progress";
const SUMMARY_ID: &str = "campaign_combined_result";
const HISTOGRAMS_ID: &str = "campaign_histograms";
const CHILDREN_ID: &str = "campaign_children";

pub(super) fn projectors() -> Vec<TaskPanelProjector> {
    projectors_with_workers(None)
}

pub(super) fn projectors_for_workers(
    workers: &[crate::core::RegisteredNode],
) -> Vec<TaskPanelProjector> {
    projectors_with_workers(Some(CampaignWorkers::from_live_nodes(workers)))
}

#[derive(Default)]
struct CampaignWorkers {
    desired_samplers: BTreeSet<i32>,
    active_samplers: BTreeSet<i32>,
}

impl CampaignWorkers {
    // ControlPlaneStore::list_nodes excludes expired leases.
    fn from_live_nodes(workers: &[crate::core::RegisteredNode]) -> Self {
        let sampler_run = |assignment: Option<&crate::core::DesiredAssignment>| {
            assignment
                .filter(|assignment| assignment.role == crate::core::WorkerRole::SamplerAggregator)
                .map(|assignment| assignment.run_id)
        };
        Self {
            desired_samplers: workers
                .iter()
                .filter_map(|node| sampler_run(node.desired_assignment.as_ref()))
                .collect(),
            active_samplers: workers
                .iter()
                .filter_map(|node| sampler_run(node.current_assignment.as_ref()))
                .collect(),
        }
    }

    fn child_status(&self, run_id: Option<i32>) -> &'static str {
        if run_id.is_some_and(|id| self.active_samplers.contains(&id)) {
            "running"
        } else if run_id.is_some_and(|id| self.desired_samplers.contains(&id)) {
            "starting"
        } else {
            "waiting"
        }
    }
}

fn projectors_with_workers(workers: Option<CampaignWorkers>) -> Vec<TaskPanelProjector> {
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
        children_projector(workers),
        histograms_projector(),
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
                            Some("imag") => "Imag",
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
            if let Some(target) = super::sample::run_target_from_json(ctx.run_target) {
                for (index, result) in results.into_iter().flatten().enumerate() {
                    let component = result.component.as_deref();
                    let names = match component {
                        Some("real") => vec!["real", "value"],
                        Some(name) => vec![name],
                        None => vec!["value", "real"],
                    };
                    if let (Some(value), Some(error)) =
                        (target.component(&names), result.uncertainty)
                    {
                        let key = component
                            .map(str::to_owned)
                            .unwrap_or_else(|| index.to_string());
                        let label = match component {
                            Some("real") => "Real vs Target".to_string(),
                            Some("imag") => "Imag vs Target".to_string(),
                            Some(name) => format!("{name} vs Target"),
                            None => "vs Target".to_string(),
                        };
                        entries.push(super::sample::target_comparison_entry(
                            &format!("target_comparison_{key}"),
                            &label,
                            result.value,
                            error,
                            value,
                        ));
                    }
                }
            }
            Ok(Some(key_value_panel(SUMMARY_ID, entries)))
        },
        |_ctx| Ok(None),
    )
}

fn children_projector(workers: Option<CampaignWorkers>) -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            CHILDREN_ID,
            "Campaign Sub-runs",
            PanelKind::Table,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        move |ctx| {
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
                .map(|child| {
                    child_row(
                        child,
                        campaign_stopped,
                        &result_keys,
                        total_variance,
                        false,
                        workers.as_ref(),
                    )
                })
                .collect::<Vec<_>>();
            let columns = child_columns(&result_keys, false);
            let absolute_columns = child_columns(&result_keys, true);
            let absolute_rows = children
                .iter()
                .map(|child| {
                    child_row(
                        child,
                        campaign_stopped,
                        &result_keys,
                        total_variance,
                        true,
                        workers.as_ref(),
                    )
                })
                .collect::<Vec<_>>();
            let mut payload = select_run_payload();
            payload["sortable"] = json!(true);
            payload["row_numbers"] = json!(true);
            let mut formats = serde_json::Map::new();
            for columns in [&columns, &absolute_columns] {
                for index in 0..result_keys.len() {
                    formats.insert(columns[4 + 3 * index].clone(), json!("scientific"));
                    formats.insert(columns[5 + 3 * index].clone(), json!("scientific"));
                }
            }
            payload["column_formats"] = json!(formats);
            if result_keys
                .iter()
                .any(|key| key.0 == crate::core::AccumulatorMetricName::Mean)
            {
                payload["absolute_components"] = json!({
                    "columns": absolute_columns,
                    "rows": absolute_rows,
                });
            }
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

fn child_columns(result_keys: &[ResultKey], absolute: bool) -> Vec<String> {
    let mut columns = ["name", "status", "run", "coeff"]
        .map(str::to_string)
        .to_vec();
    for key in result_keys {
        let label = result_key_label(key, result_keys.len());
        let label = if absolute && key.0 == crate::core::AccumulatorMetricName::Mean {
            format!("abs {label}")
        } else {
            label
        };
        columns.push(label.clone());
        columns.push(format!("{label} err"));
        columns.push(match key.1.as_deref() {
            Some("imag") => "imag err (%)".to_string(),
            Some("real") => "real err (%)".to_string(),
            None => "rel err (%)".to_string(),
            _ => format!("{label} err (%)"),
        });
    }
    columns.extend(["var ctrb (%)".to_string(), "samples".to_string()]);
    columns
}

fn campaign_result_keys(
    children: &[crate::core::IntegrationCampaignChildOutput],
) -> Vec<ResultKey> {
    let mut keys = Vec::new();
    for result in children
        .iter()
        .filter_map(|child| measurement_results(&child.child))
        .flatten()
    {
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
    measurement_results(&child.child)?
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
    absolute: bool,
    workers: Option<&CampaignWorkers>,
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
            (crate::core::ControllerChildState::Active, true) => {
                json!(workers.map_or("running", |workers| {
                    workers.child_status(child.child.child_run_id)
                }))
            }
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
    let results = measurement_results(&child.child).unwrap_or_default();
    for key in result_keys {
        let (results, metric) = if absolute && key.0 == crate::core::AccumulatorMetricName::Mean {
            (
                child.absolute_results.as_deref().unwrap_or_default(),
                crate::core::AccumulatorMetricName::AbsMean,
            )
        } else {
            (results, key.0)
        };
        let result = results
            .iter()
            .find(|result| result.name == metric && result.component == key.1);
        row.push(result.map_or(JsonValue::Null, |result| json!(result.value)));
        row.push(
            result
                .and_then(|result| result.uncertainty)
                .map_or(JsonValue::Null, |uncertainty| json!(uncertainty)),
        );
        row.push(
            result
                .and_then(|result| {
                    result.uncertainty.map(|error| {
                        let relative =
                            100.0 * crate::evaluation::relative_error(result.value, error);
                        if relative.is_infinite() {
                            json!("∞")
                        } else {
                            json!(relative)
                        }
                    })
                })
                .unwrap_or(JsonValue::Null),
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
            absolute_results: None,
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
    fn selected_child_is_starting_until_a_live_sampler_is_active() {
        use crate::core::{DesiredAssignment, RegisteredNode, WorkerRole};
        let mut child = child(None);
        child.selected = true;
        let sampler = DesiredAssignment {
            node_name: "sampler".into(),
            role: WorkerRole::SamplerAggregator,
            run_id: 2,
            run_name: None,
        };
        let mut node = RegisteredNode {
            name: "sampler".into(),
            uuid: "sampler".into(),
            capabilities: Default::default(),
            pool_assignment: None,
            desired_assignment: Some(sampler.clone()),
            current_assignment: None,
            last_seen: None,
        };
        let status = |child: &IntegrationCampaignChildOutput, stopped, nodes: &[RegisteredNode]| {
            let workers = CampaignWorkers::from_live_nodes(nodes);
            child_row(child, stopped, &[], None, false, Some(&workers))[1].clone()
        };
        assert_eq!(status(&child, false, &[]), json!("waiting"));
        assert_eq!(status(&child, false, &[node.clone()]), json!("starting"));
        node.current_assignment = Some(DesiredAssignment {
            role: WorkerRole::Evaluator,
            ..sampler.clone()
        });
        assert_eq!(
            status(&child, false, &[node.clone()]),
            json!("starting"),
            "an evaluator is not evidence that the sampler started"
        );
        node.current_assignment = Some(sampler);
        assert_eq!(status(&child, false, &[node.clone()]), json!("running"));
        assert_eq!(status(&child, true, &[node.clone()]), json!("stopped"));
        child.selected = false;
        assert_eq!(status(&child, false, &[node]), json!("waiting"));
    }

    #[test]
    fn stopped_campaign_does_not_show_active_child() {
        let child = child(None);
        let children = [child];

        assert_eq!(
            child_row(&children[0], false, &[], None, false, None)[1],
            json!("waiting")
        );
        assert_eq!(
            child_row(&children[0], true, &[], None, false, None)[1],
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
        let row = child_row(&children[0], false, &keys, Some(variance), false, None);

        assert_eq!(row.len(), 12);
        assert_eq!(row[1], json!("running"));
        assert_eq!(row[4], json!(3.0));
        assert_eq!(row[5], json!(0.4));
        assert!((row[6].as_f64().unwrap() - 100.0 * 0.4 / 3.0).abs() < 1e-12);
        assert_eq!(row[7], json!(-2.0));
        assert_eq!(row[8], json!(0.2));
        assert_eq!(row[9], json!(10.0));
        assert_eq!(row[10], json!(100.0));
        assert_eq!(row[11], json!(10));
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

        let first_row = child_row(
            &children[0],
            false,
            &keys,
            Some(total_variance),
            false,
            None,
        );
        let second_row = child_row(
            &children[1],
            false,
            &keys,
            Some(total_variance),
            false,
            None,
        );
        assert!((first_row[10].as_f64().unwrap() - 88.888_888_888_888_89).abs() < 1e-12);
        assert!((second_row[10].as_f64().unwrap() - 11.111_111_111_111_11).abs() < 1e-12);
    }

    #[test]
    fn absolute_mode_uses_its_own_errors_and_preserves_signed_variance_contributions() {
        let mut child = child(Some(complex_measurement((-2.0, 1.0), (0.0, 0.5), 10)));
        let TaskMeasurementOutput::Completed { mut results } =
            complex_measurement((4.0, 0.5), (1.0, 0.2), 10)
        else {
            unreachable!()
        };
        for result in &mut results {
            result.name = AccumulatorMetricName::AbsMean;
        }
        child.absolute_results = Some(results);
        let keys = campaign_result_keys(std::slice::from_ref(&child));
        let variance = child_variance(&child);
        let signed = child_row(&child, false, &keys, variance, false, None);
        let absolute = child_row(&child, false, &keys, variance, true, None);
        assert_eq!(signed[6], json!(50.0));
        assert_eq!(signed[9], json!("∞"));
        assert_eq!(
            absolute[4..10],
            [
                json!(4.0),
                json!(0.5),
                json!(12.5),
                json!(1.0),
                json!(0.2),
                json!(20.0)
            ]
        );
        assert_eq!(absolute[10..], signed[10..]);
        assert_eq!(
            child_columns(&keys, false),
            [
                "name",
                "status",
                "run",
                "coeff",
                "real",
                "real err",
                "real err (%)",
                "imag",
                "imag err",
                "imag err (%)",
                "var ctrb (%)",
                "samples"
            ]
        );
        assert_eq!(child_columns(&keys, true)[4], "abs real");
        assert_eq!(child_columns(&keys, true)[8], "abs imag err");
    }

    #[test]
    fn missing_absolute_results_are_unavailable_and_zero_relative_error_is_zero() {
        let child = child(Some(complex_measurement((0.0, 0.0), (0.0, 0.0), 10)));
        let keys = campaign_result_keys(std::slice::from_ref(&child));
        let signed = child_row(&child, false, &keys, None, false, None);
        assert_eq!(signed[6], json!(0.0));
        assert_eq!(signed[9], json!(0.0));
        let absolute = child_row(&child, false, &keys, None, true, None);
        assert!(absolute[4..10].iter().all(JsonValue::is_null));
    }
}
