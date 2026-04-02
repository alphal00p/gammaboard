use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelState, PanelWidth, key_value, key_value_panel,
    panel_spec, replace_panel, with_panel_width,
};
use crate::stores::RegisteredWorkerEntry;
use serde_json::Value as JsonValue;

pub fn build_worker_panel_response(worker: &RegisteredWorkerEntry) -> PanelResponse {
    let source_id = format!("node:{}:details", worker.node_name);
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
    let mut panels = vec![with_panel_width(
        panel_spec(
            "worker_overview",
            "Node Overview",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
        ),
        PanelWidth::Half,
    )];

    match worker.current_role.as_deref() {
        Some("sampler_aggregator") => {
            if json_has_object_fields(worker.sampler_engine_diagnostics.as_ref()) {
                panels.push(with_panel_width(
                    panel_spec(
                        "sampler_diagnostics",
                        "Sampler Queue",
                        PanelKind::KeyValue,
                        PanelHistoryMode::None,
                    ),
                    PanelWidth::Full,
                ));
            } else {
                panels.push(with_panel_width(
                    panel_spec(
                        "sampler_diagnostics_status",
                        "Sampler Queue",
                        PanelKind::Text,
                        PanelHistoryMode::None,
                    ),
                    PanelWidth::Full,
                ));
            }
        }
        _ => {
            panels.push(with_panel_width(
                panel_spec(
                    "worker_role_status",
                    "Role Details",
                    PanelKind::Text,
                    PanelHistoryMode::None,
                ),
                PanelWidth::Half,
            ));
        }
    }

    panels
}

fn worker_panel_states(worker: &RegisteredWorkerEntry) -> Vec<PanelState> {
    let mut panels = vec![key_value_panel(
        "worker_overview",
        vec![
            key_value("node_name", "Node Name", worker.node_name.as_str()),
            key_value("node_uuid", "Node UUID", worker.node_uuid.as_str()),
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
        Some("evaluator") => {}
        Some("sampler_aggregator") => {
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

fn diagnostics_panel(value: Option<&JsonValue>) -> Option<PanelState> {
    let runner = value?.as_object()?.get("runner")?.as_object()?;
    let entries = [
        ("queue_buffer", "Queue Buffer"),
        ("active_evaluator_count", "Active Evaluators"),
        ("target_pending_batches", "Target Pending Batches"),
        ("pending_batches", "Pending Batches"),
        ("pending_shortfall", "Pending Shortfall"),
        ("claimed_batches", "Claimed Batches"),
        ("completed_batches", "Completed Batches"),
        ("open_batches", "Open Batches"),
        ("observable_checkpoint_state", "Checkpoint State"),
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
