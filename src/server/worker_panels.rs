use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelState, PanelWidth, format_bytes_human,
    key_value, key_value_panel, replace_panel, sized_panel_spec,
};
use crate::{core::WorkerRole, stores::RegisteredWorkerEntry};
use serde_json::Value as JsonValue;

pub fn build_worker_panel_response(worker: &RegisteredWorkerEntry) -> PanelResponse {
    let source_id = format!("node:{}:details", worker.node_name);
    let panels = worker_panel_specs(worker);
    let updates = worker_panel_states(worker)
        .into_iter()
        .map(replace_panel)
        .collect();

    PanelResponse::new(source_id, None, panels, updates, Some(3000))
}

fn worker_panel_specs(worker: &RegisteredWorkerEntry) -> Vec<crate::server::panels::PanelSpec> {
    let mut panels = vec![sized_panel_spec(
        "worker_overview",
        "Node Overview",
        PanelKind::KeyValue,
        PanelHistoryMode::None,
        PanelWidth::Half,
    )];

    match worker.current_role {
        Some(WorkerRole::SamplerAggregator) => {
            if json_has_object_fields(worker.sampler_engine_diagnostics.as_ref()) {
                panels.push(sized_panel_spec(
                    "sampler_diagnostics",
                    "Sampler Queue",
                    PanelKind::KeyValue,
                    PanelHistoryMode::None,
                    PanelWidth::Full,
                ));
            } else {
                panels.push(sized_panel_spec(
                    "sampler_diagnostics_status",
                    "Sampler Queue",
                    PanelKind::Text,
                    PanelHistoryMode::None,
                    PanelWidth::Full,
                ));
            }
        }
        _ => {
            panels.push(sized_panel_spec(
                "worker_role_status",
                "Role Details",
                PanelKind::Text,
                PanelHistoryMode::None,
                PanelWidth::Half,
            ));
        }
    }

    if synthetic_diagnostics(worker).is_some() {
        panels.push(sized_panel_spec(
            "synthetic_timing",
            "Synthetic workload timing",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ));
    }
    panels
}

fn worker_panel_states(worker: &RegisteredWorkerEntry) -> Vec<PanelState> {
    let memory_usage = match worker.current_role {
        Some(WorkerRole::Evaluator) => worker.evaluator_rss_bytes,
        Some(WorkerRole::SamplerAggregator) => worker.sampler_rss_bytes,
        _ => None,
    };
    let mut panels = vec![key_value_panel(
        "worker_overview",
        vec![
            key_value("node_name", "Node Name", worker.node_name.as_str()),
            key_value("node_uuid", "Node UUID", worker.node_uuid.as_str()),
            key_value(
                "current_role",
                "Current Role",
                worker
                    .current_role
                    .map(WorkerRole::as_str)
                    .unwrap_or("none"),
            ),
            key_value("status", "Status", worker.status.as_str()),
            key_value("current_run_id", "Current Run ID", worker.current_run_id),
            key_value(
                "desired_role",
                "Desired Role",
                worker
                    .desired_role
                    .map(WorkerRole::as_str)
                    .unwrap_or("none"),
            ),
            key_value("pool_run_id", "Worker Pool ID", worker.pool_run_id),
            key_value("pool_run_name", "Worker Pool", worker.pool_run_name.clone()),
            key_value("desired_run_id", "Desired Run ID", worker.desired_run_id),
            key_value("last_seen", "Last Seen", worker.last_seen),
            key_value(
                "memory_usage",
                "Memory Usage",
                memory_usage.map(format_bytes_human),
            ),
        ],
    )];

    match worker.current_role {
        Some(WorkerRole::Evaluator) => {}
        Some(WorkerRole::SamplerAggregator) => {
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

    if let Some(diagnostics) = synthetic_diagnostics(worker) {
        let mut entries = vec![key_value("synthetic", "Synthetic workload", true)];
        for (field, label) in [
            ("timing", "Evaluation"),
            ("generation_timing", "Generation"),
            ("ingest_timing", "Result ingestion"),
            ("update_timing", "Training update"),
        ] {
            if let Some(stats) = diagnostics.get(field).and_then(JsonValue::as_object) {
                for (key, suffix) in [
                    ("calls", "calls"),
                    ("requested_seconds", "requested seconds"),
                    ("actual_seconds", "actual seconds"),
                    ("clipped_calls", "clipped delays"),
                ] {
                    if let Some(value) = stats.get(key) {
                        entries.push(key_value(
                            &format!("{field}_{key}"),
                            &format!("{label}: {suffix}"),
                            value.clone(),
                        ));
                    }
                }
            }
        }
        for key in [
            "training_updates",
            "training_barrier_seconds",
            "training_window_samples",
            "pending_training_samples",
        ] {
            if let Some(value) = diagnostics.get(key) {
                entries.push(key_value(key, &key.replace('_', " "), value.clone()));
            }
        }
        panels.push(key_value_panel("synthetic_timing", entries));
    }
    panels
}

fn diagnostics_panel(value: Option<&JsonValue>) -> Option<PanelState> {
    let runner = value?.as_object()?.get("runner")?.as_object()?;
    let entries = [
        ("queue_buffer", "Target Pending Batches / Evaluator"),
        ("active_evaluator_count", "Active Evaluators"),
        ("target_pending_batches", "Target DB Pending Batches"),
        ("db_pending_batches", "DB Pending Batches"),
        ("pending_shortfall", "DB Pending Shortfall"),
        ("local_pending_batches", "Local Pending Batches"),
        (
            "local_inflight_insert_tasks",
            "Local In-Flight Insert Tasks",
        ),
        (
            "local_inflight_insert_batches",
            "Local In-Flight Insert Batches",
        ),
        ("local_ready_processed_batches", "Completed Prefetch Buffer"),
        ("accumulator_checkpoint_state", "Checkpoint State"),
        ("training_samples_remaining", "Training Samples Remaining"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        runner
            .get(key)
            .cloned()
            .map(|value| key_value(key, label, value))
    })
    .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    Some(key_value_panel("sampler_diagnostics", entries))
}

fn text_panel(panel_id: &str, text: impl Into<String>) -> PanelState {
    PanelState::Text {
        panel_id: panel_id.to_string(),
        text: text.into(),
    }
}

fn json_has_object_fields(value: Option<&JsonValue>) -> bool {
    value
        .and_then(JsonValue::as_object)
        .is_some_and(|object| !object.is_empty())
}

pub fn append_activity_panel(response: &mut PanelResponse, activity: &JsonValue) {
    response.panels.push(sized_panel_spec(
        "worker_activity",
        "Current activity",
        PanelKind::KeyValue,
        PanelHistoryMode::None,
        PanelWidth::Half,
    ));
    let age = activity["last_completed_batch_at"]
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|t| {
            (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0)
        });
    response.updates.push(replace_panel(key_value_panel(
        "worker_activity",
        vec![
            key_value("activity", "Activity", &activity["activity"]),
            key_value("since", "Since", &activity["since"]),
            key_value("last_batch_age", "Seconds since last completed batch", age),
            key_value(
                "rate_interval",
                "Throughput interval",
                "60 seconds of active runner wall time (includes waits)",
            ),
        ],
    )));
}

fn synthetic_diagnostics(worker: &RegisteredWorkerEntry) -> Option<&JsonValue> {
    let diagnostics = match worker.current_role {
        Some(WorkerRole::Evaluator) => &worker.evaluator_metrics.as_ref()?.engine_diagnostics,
        Some(WorkerRole::SamplerAggregator) => worker.sampler_engine_diagnostics.as_ref()?,
        _ => return None,
    };
    (diagnostics.get("synthetic").and_then(JsonValue::as_bool) == Some(true)).then_some(diagnostics)
}
