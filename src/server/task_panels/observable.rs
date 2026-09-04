use crate::server::panels::{PanelState, TableStateOptions, table_panel_with_payload_and_options};
use serde_json::{Value as JsonValue, json};

pub(super) fn histogram_bundle_panel(
    panel_id: &str,
    expanded_label: &str,
    mut payload: JsonValue,
) -> Option<PanelState> {
    let object = payload.as_object_mut()?;
    object.insert(
        "expands_to".to_string(),
        json!({ "kind": "histogram", "source": "selected_row" }),
    );
    object.insert("expanded_label".to_string(), json!(expanded_label));
    let histograms = object.get("histograms")?.as_object()?;
    let rows = histograms
        .iter()
        .map(|(name, histogram)| {
            vec![
                json!(name),
                histogram.get("title").cloned().unwrap_or(JsonValue::Null),
                histogram.get("phase").cloned().unwrap_or(JsonValue::Null),
                histogram
                    .get("sample_count")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                json!(
                    histogram
                        .get("bins")
                        .and_then(JsonValue::as_array)
                        .map_or(0, Vec::len)
                ),
            ]
        })
        .collect();
    let row_keys = histograms.keys().cloned().collect();

    Some(table_panel_with_payload_and_options(
        panel_id,
        vec![
            "Name".to_string(),
            "Title".to_string(),
            "Phase".to_string(),
            "Samples".to_string(),
            "Bins".to_string(),
        ],
        rows,
        Some(payload),
        TableStateOptions {
            visible_column_indices: vec![1, 2, 3, 4],
            row_keys: Some(row_keys),
        },
    ))
}
