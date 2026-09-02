use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, min_max_row_tones, panel_spec, progress_panel, row_tone_labels,
};
use serde_json::{Map, Value as JsonValue, json};

pub(super) fn progress_projector(
    panel_id: &'static str,
    title: &'static str,
    current_value: fn(&crate::core::ControllerTaskOutput) -> Option<f64>,
    total_value: fn(&crate::core::ControllerTaskOutput) -> Option<f64>,
    unit: &'static str,
    fallback_total: fn(&TaskPanelContext<'_>) -> Option<f64>,
) -> TaskPanelProjector {
    panel_projector(
        panel_spec(panel_id, title, PanelKind::Progress, PanelHistoryMode::None),
        move |ctx| {
            let current = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(current_value)
                .unwrap_or(0.0);
            let total = ctx
                .task
                .controller_output
                .as_ref()
                .and_then(total_value)
                .or_else(|| fallback_total(ctx));
            Ok(Some(progress_panel(
                panel_id,
                current,
                total,
                Some(unit),
                None,
            )))
        },
        |_ctx| Ok(None),
    )
}

pub(super) fn child_table_payload(
    rows: &[Vec<JsonValue>],
    value_column: usize,
    extra: Map<String, JsonValue>,
) -> JsonValue {
    let mut payload = extra;
    payload.insert(
        "row_action".to_string(),
        json!({ "kind": "select_run", "column": "run" }),
    );
    payload.insert(
        "row_tones".to_string(),
        json!(min_max_row_tones(rows, value_column)),
    );
    payload.insert("row_tone_labels".to_string(), row_tone_labels());
    JsonValue::Object(payload)
}
