use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    Select,
    ScalarTimeseries,
    MultiTimeseries,
    TickBreakdown,
    Svg,
    Image2d,
    Progress,
    KeyValue,
    Table,
    Histogram,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageColorMode {
    ScalarHeatmap,
    VectorMagnitude,
    ComplexPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageNormalizationMode {
    MinMax,
    Symmetric,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelHistoryMode {
    None,
    Append,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelWidth {
    Compact,
    Half,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelSpec {
    pub panel_id: String,
    pub label: String,
    pub kind: PanelKind,
    pub history: PanelHistoryMode,
    #[serde(default = "default_panel_width")]
    pub width: PanelWidth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<PanelStateSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PanelActionSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelStateSpec {
    Select {
        default_value: JsonValue,
        options: Vec<PanelStateOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<JsonValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelStateOption {
    pub value: JsonValue,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelActionSpec {
    pub action_id: String,
    pub label: String,
    pub kind: PanelActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelActionKind {
    Invoke,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PanelRequest {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "default_panel_state")]
    pub panel_state: JsonValue,
    #[serde(default)]
    pub panel_actions: Vec<PanelActionInvocation>,
}

fn default_panel_state() -> JsonValue {
    JsonValue::Object(Default::default())
}

fn default_panel_width() -> PanelWidth {
    PanelWidth::Half
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelActionInvocation {
    pub panel_id: String,
    pub action_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlotPoint {
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_sampler_uptime_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_completed_samples_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_max: Option<f64>,
}

impl PlotPoint {
    /// Point at `x` with an optional `y +- error` band and no alternate
    /// x-axis metadata.
    pub(crate) fn timeseries(x: f64, y: f64, error: Option<f64>) -> Self {
        Self {
            x,
            y,
            y_min: error.map(|error| y - error),
            y_max: error.map(|error| y + error),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotSeries {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<bool>,
    pub points: Vec<PlotPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValueEntry {
    pub key: String,
    pub label: String,
    pub value: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBin {
    pub start: f64,
    pub stop: f64,
    pub value: f64,
    pub error: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickBreakdownSegment {
    pub key: String,
    pub label: String,
    pub value_ms: f64,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableStateOptions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_column_indices: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelState {
    ScalarTimeseries {
        panel_id: String,
        points: Vec<PlotPoint>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        smooth: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<f64>,
    },
    MultiTimeseries {
        panel_id: String,
        series: Vec<PlotSeries>,
    },
    TickBreakdown {
        panel_id: String,
        total_ms: f64,
        segments: Vec<TickBreakdownSegment>,
    },
    Svg {
        panel_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        svg: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Image2d {
        panel_id: String,
        width: usize,
        height: usize,
        values: Vec<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        imag_values: Option<Vec<f32>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        invalid_indices: Option<Vec<usize>>,
        x_range: [f64; 2],
        y_range: [f64; 2],
        color_mode: ImageColorMode,
        normalization_mode: ImageNormalizationMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        metric_label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metric_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        x_label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        y_label: Option<String>,
    },
    Progress {
        panel_id: String,
        current: f64,
        total: Option<f64>,
        unit: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        eta_seconds: Option<f64>,
    },
    KeyValue {
        panel_id: String,
        entries: Vec<KeyValueEntry>,
    },
    Table {
        panel_id: String,
        columns: Vec<String>,
        rows: Vec<Vec<JsonValue>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        visible_column_indices: Vec<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        row_keys: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<JsonValue>,
    },
    Histogram {
        panel_id: String,
        bins: Vec<HistogramBin>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        controls: Option<JsonValue>,
    },
    Text {
        panel_id: String,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelUpdateMode {
    Replace,
    Append,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelUpdate {
    pub mode: PanelUpdateMode,
    pub panel: PanelState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelResponse {
    pub source_id: String,
    pub cursor: Option<String>,
    pub reset_required: bool,
    pub panels: Vec<PanelSpec>,
    pub updates: Vec<PanelUpdate>,
    pub poll_after_ms: Option<u64>,
}

pub(crate) fn panel_spec(
    panel_id: &str,
    label: &str,
    kind: PanelKind,
    history: PanelHistoryMode,
) -> PanelSpec {
    PanelSpec {
        panel_id: panel_id.to_string(),
        label: label.to_string(),
        kind,
        history,
        width: PanelWidth::Half,
        state: None,
        actions: Vec::new(),
    }
}

pub(crate) fn with_panel_width(mut spec: PanelSpec, width: PanelWidth) -> PanelSpec {
    spec.width = width;
    spec
}

/// `panel_spec` plus an explicit width, replacing the common
/// `with_panel_width(panel_spec(...), width)` nesting.
pub(crate) fn sized_panel_spec(
    panel_id: &str,
    label: &str,
    kind: PanelKind,
    history: PanelHistoryMode,
    width: PanelWidth,
) -> PanelSpec {
    with_panel_width(panel_spec(panel_id, label, kind, history), width)
}

pub(crate) fn select_state_spec(
    default_value: JsonValue,
    options: Vec<PanelStateOption>,
    payload: Option<JsonValue>,
) -> PanelStateSpec {
    PanelStateSpec::Select {
        default_value,
        options,
        payload,
    }
}

pub(crate) fn state_option<T: Serialize>(value: T, label: &str) -> PanelStateOption {
    PanelStateOption {
        value: serde_json::to_value(value).unwrap_or(JsonValue::Null),
        label: label.to_string(),
    }
}

pub(crate) fn key_value<T: Serialize>(key: &str, label: &str, value: T) -> KeyValueEntry {
    KeyValueEntry {
        key: key.to_string(),
        label: label.to_string(),
        value: serde_json::to_value(value).unwrap_or(JsonValue::Null),
        tone: None,
    }
}

pub(crate) fn key_value_with_tone<T: Serialize>(
    key: &str,
    label: &str,
    value: T,
    tone: Option<&str>,
) -> KeyValueEntry {
    KeyValueEntry {
        key: key.to_string(),
        label: label.to_string(),
        value: serde_json::to_value(value).unwrap_or(JsonValue::Null),
        tone: tone.map(str::to_string),
    }
}

pub(crate) fn scalar_timeseries_panel(panel_id: &str, points: Vec<PlotPoint>) -> PanelState {
    scalar_timeseries_panel_with_smoothing(panel_id, points, None)
}

pub(crate) fn scalar_timeseries_panel_with_smoothing(
    panel_id: &str,
    points: Vec<PlotPoint>,
    smooth: Option<bool>,
) -> PanelState {
    PanelState::ScalarTimeseries {
        panel_id: panel_id.to_string(),
        points,
        smooth,
        target: None,
    }
}

pub(crate) fn text_panel(panel_id: &str, text: impl Into<String>) -> PanelState {
    PanelState::Text {
        panel_id: panel_id.to_string(),
        text: text.into(),
    }
}

pub(crate) fn multi_timeseries_panel(panel_id: &str, series: Vec<PlotSeries>) -> PanelState {
    PanelState::MultiTimeseries {
        panel_id: panel_id.to_string(),
        series,
    }
}

pub(crate) fn progress_panel(
    panel_id: &str,
    current: f64,
    total: Option<f64>,
    unit: Option<&str>,
    eta_seconds: Option<f64>,
) -> PanelState {
    PanelState::Progress {
        panel_id: panel_id.to_string(),
        current,
        total,
        unit: unit.map(str::to_string),
        eta_seconds,
    }
}

pub(crate) fn key_value_panel(panel_id: &str, entries: Vec<KeyValueEntry>) -> PanelState {
    PanelState::KeyValue {
        panel_id: panel_id.to_string(),
        entries,
    }
}

pub(crate) fn tick_breakdown_panel(
    panel_id: &str,
    total_ms: f64,
    segments: Vec<TickBreakdownSegment>,
) -> PanelState {
    PanelState::TickBreakdown {
        panel_id: panel_id.to_string(),
        total_ms,
        segments,
    }
}

pub(crate) fn table_panel_with_payload(
    panel_id: &str,
    columns: Vec<String>,
    rows: Vec<Vec<JsonValue>>,
    payload: Option<JsonValue>,
) -> PanelState {
    table_panel_with_payload_and_options(
        panel_id,
        columns,
        rows,
        payload,
        TableStateOptions::default(),
    )
}

pub(crate) fn table_panel_with_payload_and_options(
    panel_id: &str,
    columns: Vec<String>,
    rows: Vec<Vec<JsonValue>>,
    payload: Option<JsonValue>,
    options: TableStateOptions,
) -> PanelState {
    PanelState::Table {
        panel_id: panel_id.to_string(),
        columns,
        rows,
        visible_column_indices: options.visible_column_indices,
        row_keys: options.row_keys,
        payload,
    }
}

pub(crate) fn min_max_row_tones(rows: &[Vec<JsonValue>], value_column: usize) -> Vec<JsonValue> {
    let finite_values = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            row.get(value_column)
                .and_then(JsonValue::as_f64)
                .filter(|value| value.is_finite())
                .map(|value| (index, value))
        })
        .collect::<Vec<_>>();
    let min_index = finite_values
        .iter()
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(index, _)| *index);
    let max_index = finite_values
        .iter()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(index, _)| *index);
    (0..rows.len())
        .map(
            |index| match (Some(index) == min_index, Some(index) == max_index) {
                (true, true) => JsonValue::String("min_max".to_string()),
                (true, false) => JsonValue::String("min".to_string()),
                (false, true) => JsonValue::String("max".to_string()),
                (false, false) => JsonValue::Null,
            },
        )
        .collect()
}

pub(crate) fn row_tone_labels() -> JsonValue {
    serde_json::json!({ "min": "min", "max": "max", "min_max": "min/max" })
}

pub(crate) fn replace_panel(panel: PanelState) -> PanelUpdate {
    PanelUpdate {
        mode: PanelUpdateMode::Replace,
        panel,
    }
}

pub(crate) fn append_panel(panel: PanelState) -> PanelUpdate {
    PanelUpdate {
        mode: PanelUpdateMode::Append,
        panel,
    }
}

impl PanelState {
    pub fn panel_id(&self) -> &str {
        match self {
            Self::ScalarTimeseries { panel_id, .. }
            | Self::MultiTimeseries { panel_id, .. }
            | Self::TickBreakdown { panel_id, .. }
            | Self::Svg { panel_id, .. }
            | Self::Image2d { panel_id, .. }
            | Self::Progress { panel_id, .. }
            | Self::KeyValue { panel_id, .. }
            | Self::Table { panel_id, .. }
            | Self::Histogram { panel_id, .. }
            | Self::Text { panel_id, .. } => panel_id,
        }
    }
}

pub(crate) fn merge_panel_state(target: &mut PanelState, delta: PanelState) {
    match (target, delta) {
        (
            PanelState::ScalarTimeseries { points, .. },
            PanelState::ScalarTimeseries {
                points: delta_points,
                ..
            },
        ) => merge_plot_points(points, delta_points),
        (
            PanelState::MultiTimeseries { series, .. },
            PanelState::MultiTimeseries {
                series: delta_series,
                ..
            },
        ) => {
            for delta in delta_series {
                if let Some(existing) = series.iter_mut().find(|item| item.id == delta.id) {
                    merge_plot_points(&mut existing.points, delta.points);
                } else {
                    series.push(delta);
                }
            }
        }
        (target, delta) => *target = delta,
    }
}

fn merge_plot_points(points: &mut Vec<PlotPoint>, delta_points: Vec<PlotPoint>) {
    for delta in delta_points {
        if let Some(existing) = points.iter_mut().find(|point| point.x == delta.x) {
            *existing = delta;
        } else {
            points.push(delta);
        }
    }
    points.sort_by(|left, right| left.x.total_cmp(&right.x));
}

pub(crate) fn history_x(created_at: chrono::DateTime<chrono::Utc>) -> f64 {
    created_at.timestamp_millis() as f64
}
