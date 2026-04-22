use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunExposedInfoCache {
    #[serde(flatten)]
    pub entries: BTreeMap<String, RunExposedInfoEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunExposedInfoEntry {
    pub kind: String,
    pub title: String,
    pub status: RunExposedInfoStatus,
    pub cache_key: String,
    pub scope: RunExposedInfoScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<RunExposedInfoContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunExposedInfoStatus {
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunExposedInfoScope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunExposedInfoContent {
    ArtifactBundle {
        primary: RunExposedArtifact,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<NamedRunExposedArtifact>,
    },
    Text {
        mime: String,
        data: String,
    },
    Json {
        data: JsonValue,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunExposedArtifact {
    Svg { data: String },
    Text { mime: String, data: String },
    Json { data: JsonValue },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedRunExposedArtifact {
    pub name: String,
    #[serde(flatten)]
    pub artifact: RunExposedArtifact,
}
