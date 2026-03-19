use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    ScalarTimeseries,
    MultiTimeseries,
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
    ComplexHueIntensity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelDescriptor {
    pub panel_id: String,
    pub label: String,
    pub kind: PanelKind,
    pub supports_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotPoint {
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y_max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotSeries {
    pub id: String,
    pub label: String,
    pub points: Vec<PlotPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValueEntry {
    pub key: String,
    pub label: String,
    pub value: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBin {
    pub start: f64,
    pub stop: f64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelState {
    ScalarTimeseries {
        panel_id: String,
        points: Vec<PlotPoint>,
    },
    MultiTimeseries {
        panel_id: String,
        series: Vec<PlotSeries>,
    },
    Image2d {
        panel_id: String,
        width: usize,
        height: usize,
        values: Vec<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        imag_values: Option<Vec<f32>>,
        x_range: [f64; 2],
        y_range: [f64; 2],
        color_mode: ImageColorMode,
    },
    Progress {
        panel_id: String,
        current: f64,
        total: Option<f64>,
        unit: Option<String>,
    },
    KeyValue {
        panel_id: String,
        entries: Vec<KeyValueEntry>,
    },
    Table {
        panel_id: String,
        columns: Vec<String>,
        rows: Vec<Vec<JsonValue>>,
    },
    Histogram {
        panel_id: String,
        bins: Vec<HistogramBin>,
    },
    Text {
        panel_id: String,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutputResponse {
    pub task_id: String,
    pub sequence_nr: i32,
    pub task_kind: String,
    pub task_state: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub panels: Vec<PanelDescriptor>,
    pub current: Vec<PanelState>,
    pub latest_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHistoryItem {
    pub snapshot_id: String,
    pub created_at: Option<DateTime<Utc>>,
    pub panels: Vec<PanelState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskHistoryResponse {
    pub task_id: String,
    pub latest_snapshot_id: Option<String>,
    pub reset_required: bool,
    pub items: Vec<TaskHistoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceHistoryResponse {
    pub scope_id: Option<String>,
    pub latest_snapshot_id: Option<String>,
    pub reset_required: bool,
    pub panels: Vec<PanelDescriptor>,
    pub current: Vec<PanelState>,
    pub items: Vec<TaskHistoryItem>,
}

pub(crate) fn panel_descriptor(
    panel_id: &str,
    label: &str,
    kind: PanelKind,
    supports_history: bool,
) -> PanelDescriptor {
    PanelDescriptor {
        panel_id: panel_id.to_string(),
        label: label.to_string(),
        kind,
        supports_history,
    }
}

pub(crate) fn single_point_band(
    panel_id: &str,
    x: f64,
    y: f64,
    y_min: Option<f64>,
    y_max: Option<f64>,
) -> PanelState {
    scalar_timeseries_panel(panel_id, vec![PlotPoint { x, y, y_min, y_max }])
}

pub(crate) fn key_value<T: Serialize>(key: &str, label: &str, value: T) -> KeyValueEntry {
    KeyValueEntry {
        key: key.to_string(),
        label: label.to_string(),
        value: serde_json::to_value(value).unwrap_or(JsonValue::Null),
    }
}

pub(crate) fn scalar_timeseries_panel(panel_id: &str, points: Vec<PlotPoint>) -> PanelState {
    PanelState::ScalarTimeseries {
        panel_id: panel_id.to_string(),
        points,
    }
}

pub(crate) fn progress_panel(
    panel_id: &str,
    current: f64,
    total: Option<f64>,
    unit: Option<&str>,
) -> PanelState {
    PanelState::Progress {
        panel_id: panel_id.to_string(),
        current,
        total,
        unit: unit.map(str::to_string),
    }
}

pub(crate) fn key_value_panel(panel_id: &str, entries: Vec<KeyValueEntry>) -> PanelState {
    PanelState::KeyValue {
        panel_id: panel_id.to_string(),
        entries,
    }
}

pub(crate) fn history_item(
    snapshot_id: impl ToString,
    created_at: Option<DateTime<Utc>>,
    panels: Vec<PanelState>,
) -> TaskHistoryItem {
    TaskHistoryItem {
        snapshot_id: snapshot_id.to_string(),
        created_at,
        panels,
    }
}

pub(crate) fn history_x(created_at: DateTime<Utc>) -> f64 {
    created_at.timestamp_millis() as f64
}
