use crate::core::{EvaluatorPerformanceMetrics, SamplerRuntimeMetrics};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelState, key_value, key_value_panel, panel_spec,
    replace_panel,
};
use crate::stores::RegisteredWorkerEntry;
use serde_json::Value as JsonValue;

pub fn build_worker_panel_response(worker: &RegisteredWorkerEntry) -> PanelResponse {
    let source_id = format!(
        "node:{}:details",
        worker.node_id.as_deref().unwrap_or(&worker.worker_id)
    );
    let panels = worker_panel_specs(worker);
    let updates = worker_panel_states(worker)
        .into_iter()
        .map(replace_panel)
        .collect();

    PanelResponse {
        source_id,
        cursor: None,
        reset_required: false,
        panels,
        updates,
    }
}

fn worker_panel_specs(worker: &RegisteredWorkerEntry) -> Vec<crate::server::panels::PanelSpec> {
    let mut panels = vec![panel_spec(
        "worker_overview",
        "Node Overview",
        PanelKind::KeyValue,
        PanelHistoryMode::None,
    )];

    match worker.current_role.as_deref() {
        Some("evaluator") => {
            if worker.evaluator_metrics.is_some() {
                panels.push(panel_spec(
                    "evaluator_metrics",
                    "Evaluator Metrics",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ));
            } else {
                panels.push(panel_spec(
                    "evaluator_metrics_status",
                    "Evaluator Metrics",
                    PanelKind::Text,
                    PanelHistoryMode::None,
                ));
            }
        }
        Some("sampler_aggregator") => {
            if worker.sampler_metrics.is_some() {
                panels.push(panel_spec(
                    "sampler_metrics",
                    "Sampler Aggregator Metrics",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ));
            } else {
                panels.push(panel_spec(
                    "sampler_metrics_status",
                    "Sampler Aggregator Metrics",
                    PanelKind::Text,
                    PanelHistoryMode::None,
                ));
            }

            match decode_sampler_runtime_metrics(worker) {
                Some(_) => {
                    panels.push(panel_spec(
                        "sampler_runtime",
                        "Sampler Runtime Metrics",
                        PanelKind::KeyValue,
                        PanelHistoryMode::None,
                    ));
                }
                None => {
                    panels.push(panel_spec(
                        "sampler_runtime_status",
                        "Sampler Runtime Metrics",
                        PanelKind::Text,
                        PanelHistoryMode::None,
                    ));
                }
            }

            if json_has_object_fields(worker.sampler_engine_diagnostics.as_ref()) {
                panels.push(panel_spec(
                    "sampler_diagnostics",
                    "Sampler Diagnostics",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                ));
            } else {
                panels.push(panel_spec(
                    "sampler_diagnostics_status",
                    "Sampler Diagnostics",
                    PanelKind::Text,
                    PanelHistoryMode::None,
                ));
            }
        }
        _ => {
            panels.push(panel_spec(
                "worker_role_status",
                "Role Details",
                PanelKind::Text,
                PanelHistoryMode::None,
            ));
        }
    }

    panels
}

fn worker_panel_states(worker: &RegisteredWorkerEntry) -> Vec<PanelState> {
    let mut panels = vec![key_value_panel(
        "worker_overview",
        vec![
            key_value(
                "node_id",
                "Node ID",
                worker.node_id.as_deref().unwrap_or(&worker.worker_id),
            ),
            key_value(
                "current_role",
                "Current Role",
                worker.current_role.as_deref().unwrap_or("none"),
            ),
            key_value("status", "Status", worker.status.as_str()),
            key_value("current_run_id", "Current Run ID", worker.current_run_id),
            key_value(
                "desired_role",
                "Desired Role",
                worker.desired_role.as_deref().unwrap_or("none"),
            ),
            key_value("desired_run_id", "Desired Run ID", worker.desired_run_id),
            key_value(
                "implementation",
                "Implementation",
                worker.implementation.as_str(),
            ),
            key_value("version", "Version", worker.version.as_str()),
            key_value("last_seen", "Last Seen", worker.last_seen),
        ],
    )];

    match worker.current_role.as_deref() {
        Some("evaluator") => {
            if let Some(metrics) = worker.evaluator_metrics.as_ref() {
                panels.push(evaluator_metrics_panel(metrics));
            } else {
                panels.push(text_panel(
                    "evaluator_metrics_status",
                    unavailable_metrics_message(worker, "evaluator"),
                ));
            }
        }
        Some("sampler_aggregator") => {
            if let Some(metrics) = worker.sampler_metrics.as_ref() {
                panels.push(sampler_metrics_panel(metrics));
            } else {
                panels.push(text_panel(
                    "sampler_metrics_status",
                    unavailable_metrics_message(worker, "sampler"),
                ));
            }

            if let Some(runtime) = decode_sampler_runtime_metrics(worker) {
                panels.push(sampler_runtime_panel(&runtime));
            } else {
                panels.push(text_panel(
                    "sampler_runtime_status",
                    unavailable_runtime_message(worker),
                ));
            }

            if let Some(diagnostics) = diagnostics_panel(worker.sampler_engine_diagnostics.as_ref())
            {
                panels.push(diagnostics);
            } else {
                panels.push(text_panel(
                    "sampler_diagnostics_status",
                    "No sampler diagnostics reported.",
                ));
            }
        }
        _ => {
            panels.push(text_panel(
                "worker_role_status",
                "No role-specific panels are available for this node while it is idle.",
            ));
        }
    }

    panels
}

fn evaluator_metrics_panel(metrics: &EvaluatorPerformanceMetrics) -> PanelState {
    key_value_panel(
        "evaluator_metrics",
        vec![
            key_value(
                "batches_completed",
                "Batches Completed",
                metrics.batches_completed,
            ),
            key_value(
                "samples_evaluated",
                "Samples Evaluated",
                metrics.samples_evaluated,
            ),
            key_value(
                "avg_time_per_sample_ms",
                "Avg Time Per Sample (ms)",
                metrics.avg_time_per_sample_ms,
            ),
            key_value(
                "std_time_per_sample_ms",
                "Std Time Per Sample (ms)",
                metrics.std_time_per_sample_ms,
            ),
            key_value(
                "idle_ratio",
                "Idle Ratio",
                metrics
                    .idle_profile
                    .as_ref()
                    .map(|profile| profile.idle_ratio),
            ),
        ],
    )
}

fn sampler_metrics_panel(metrics: &crate::core::SamplerPerformanceMetrics) -> PanelState {
    key_value_panel(
        "sampler_metrics",
        vec![
            key_value(
                "produced_batches",
                "Produced Batches",
                metrics.produced_batches,
            ),
            key_value(
                "produced_samples",
                "Produced Samples",
                metrics.produced_samples,
            ),
            key_value(
                "avg_produce_time_per_sample_ms",
                "Avg Produce Time Per Sample (ms)",
                metrics.avg_produce_time_per_sample_ms,
            ),
            key_value(
                "std_produce_time_per_sample_ms",
                "Std Produce Time Per Sample (ms)",
                metrics.std_produce_time_per_sample_ms,
            ),
            key_value(
                "ingested_batches",
                "Ingested Batches",
                metrics.ingested_batches,
            ),
            key_value(
                "ingested_samples",
                "Ingested Samples",
                metrics.ingested_samples,
            ),
            key_value(
                "avg_ingest_time_per_sample_ms",
                "Avg Ingest Time Per Sample (ms)",
                metrics.avg_ingest_time_per_sample_ms,
            ),
            key_value(
                "std_ingest_time_per_sample_ms",
                "Std Ingest Time Per Sample (ms)",
                metrics.std_ingest_time_per_sample_ms,
            ),
        ],
    )
}

fn sampler_runtime_panel(runtime: &SamplerRuntimeMetrics) -> PanelState {
    key_value_panel(
        "sampler_runtime",
        vec![
            key_value(
                "batch_size_current",
                "Batch Size Current",
                runtime.batch_size_current,
            ),
            key_value(
                "completed_samples_per_second",
                "Completed Samples Per Second",
                runtime.completed_samples_per_second,
            ),
            key_value(
                "eval_ms_per_sample",
                "Eval Ms Per Sample",
                runtime.rolling.eval_ms_per_sample.mean,
            ),
            key_value(
                "eval_ms_per_batch",
                "Eval Ms Per Batch",
                runtime.rolling.eval_ms_per_batch.mean,
            ),
            key_value(
                "sampler_produce_ms_per_sample",
                "Sampler Produce Ms Per Sample",
                runtime.rolling.sampler_produce_ms_per_sample.mean,
            ),
            key_value(
                "sampler_ingest_ms_per_sample",
                "Sampler Ingest Ms Per Sample",
                runtime.rolling.sampler_ingest_ms_per_sample.mean,
            ),
            key_value(
                "queue_remaining_ratio",
                "Queue Remaining Ratio",
                runtime.rolling.queue_remaining_ratio.mean,
            ),
            key_value(
                "batches_consumed_per_tick",
                "Batches Consumed Per Tick",
                runtime.rolling.batches_consumed_per_tick.mean,
            ),
        ],
    )
}

fn diagnostics_panel(value: Option<&JsonValue>) -> Option<PanelState> {
    let object = value?.as_object()?;
    let entries = object
        .iter()
        .map(|(key, value)| key_value(key, &title_label(key), value.clone()))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    Some(key_value_panel("sampler_diagnostics", entries))
}

fn decode_sampler_runtime_metrics(worker: &RegisteredWorkerEntry) -> Option<SamplerRuntimeMetrics> {
    serde_json::from_value(worker.sampler_runtime_metrics.clone()?).ok()
}

fn text_panel(panel_id: &str, text: impl Into<String>) -> PanelState {
    PanelState::Text {
        panel_id: panel_id.to_string(),
        text: text.into(),
    }
}

fn unavailable_metrics_message(worker: &RegisteredWorkerEntry, role_label: &str) -> String {
    if worker.current_role.is_none() || worker.current_run_id.is_none() {
        if worker.desired_run_id.is_none() {
            return format!(
                "No run is currently assigned to this {role_label} node. Metrics will appear after assignment."
            );
        }
        return format!(
            "This {role_label} node is assigned but not currently active. Metrics will appear after the role starts."
        );
    }
    if worker.status.eq_ignore_ascii_case("inactive") {
        return format!(
            "This {role_label} node is inactive. Metrics will appear when the node becomes active again."
        );
    }
    format!("No {role_label} metrics are currently available for this node.")
}

fn unavailable_runtime_message(worker: &RegisteredWorkerEntry) -> String {
    if worker.current_role.as_deref() != Some("sampler_aggregator") {
        return "Sampler runtime metrics are only available for active sampler nodes.".to_string();
    }
    if worker.status.eq_ignore_ascii_case("inactive") {
        return "This sampler node is inactive. Runtime metrics will appear when it becomes active again.".to_string();
    }
    "No live sampler runtime metrics are currently available for this worker.".to_string()
}

fn json_has_object_fields(value: Option<&JsonValue>) -> bool {
    value
        .and_then(JsonValue::as_object)
        .is_some_and(|object| !object.is_empty())
}

fn title_label(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
