//! Version 1 management protocol. No storage or computation types cross this boundary.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status {},
    Stop {
        wait: bool,
        timeout_ms: Option<u64>,
    },
    AcknowledgeCleanup {
        request_id: String,
    },
    RunCreate {
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        duplicate: bool,
        toml: String,
        /// Explicit directory on the deployment for relative resource references.
        base_dir: Option<String>,
        name: Option<String>,
        start: bool,
        max_evaluators: Option<usize>,
        export_path: Option<String>,
    },
    RunList {
        name: Option<String>,
    },
    RunStart {
        run: String,
        max_evaluators: Option<usize>,
    },
    RunImportResults {
        bundle: serde_json::Value,
        name: Option<String>,
    },
    RunResults {
        run: String,
        task: Option<i64>,
    },

    RunInspect {
        run: String,
        log_limit: i64,
    },
    RunValidate {
        toml: String,
        base_dir: Option<String>,
    },
    RunDuplicate {
        source: String,
        name: String,
    },
    RunExport {
        run: String,
        kind: ExportKind,
    },
    RunPause {
        runs: Vec<String>,
        all: bool,
    },
    RunRemove {
        runs: Vec<String>,
        all: bool,
        dry_run: bool,
    },
    RunRemovalStatus {
        operation_id: String,
    },
    RunPerformance {
        run: String,
        duration_ms: Option<u64>,
        interval_ms: u64,
        max_age_ms: u64,
        since: Option<String>,
        until: Option<String>,
        limit: i64,
    },
    RunReadiness {
        run: String,
        until: String,
        evaluators: usize,
        max_age_ms: u64,
    },
    Task {
        run: String,
        action: TaskAction,
    },
    WorkerList {
        name: Option<String>,
        #[serde(default)]
        state: WorkerFilter,
        #[serde(default)]
        run: Option<String>,
    },
    WorkerInspect {
        name: String,
    },
    WorkerLaunch {
        source: LaunchSource,
    },
    WorkerCancelRestoration {
        name: String,
        #[serde(default)]
        expected_uuid: Option<String>,
    },
    WorkerStop {
        names: Vec<String>,
        all: bool,
        idle: bool,
    },
    WorkerAssign {
        name: String,
        run: String,
        role: String,
    },
    WorkerUnassign {
        name: Option<String>,
        #[serde(default)]
        all: bool,
        #[serde(default)]
        expected_uuid: Option<String>,
    },
    WorkerRequests {
        id: Option<i64>,
    },
    WorkerRequestCancel {
        id: i64,
    },
    Template {
        kind: String,
        action: TemplateAction,
    },
    Logs {
        run: Option<String>,
        worker: Option<String>,
        service: Option<String>,
        limit: usize,
        before_id: Option<i64>,
    },
    BuildInfo {},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskAction {
    List {},
    Export {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
    },
    Append {
        toml: String,
        #[serde(default)]
        duplicate: bool,
    },
    Duplicate {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
    },
    Edit {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
        toml: String,
        #[serde(default)]
        expected_toml: Option<String>,
    },
    Tune {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
        queue_tuning: Option<serde_json::Value>,
    },
    CampaignScheduling {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
        settings: serde_json::Value,
    },
    CampaignTuning {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
        settings: serde_json::Value,
    },
    CampaignChildrenPreview {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
        edit: serde_json::Value,
    },
    CampaignChildrenApply {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
        edit: serde_json::Value,
    },
    Remove {
        #[serde(deserialize_with = "deserialize_id")]
        id: i64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TemplateAction {
    List {},
    Show {
        name: String,
    },
    Save {
        name: String,
        toml: String,
        overwrite: bool,
    },
    Remove {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    Definition,
    Results,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, clap::ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerFilter {
    #[default]
    Live,
    Offline,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LaunchSource {
    New {
        count: Option<usize>,
        toml: Option<String>,
        #[serde(default)]
        max_start_failures: Option<u32>,
        #[serde(default)]
        name_prefix: Option<String>,
        #[serde(default)]
        args: serde_json::Value,
    },
    Saved {
        name: String,
        #[serde(default)]
        expected_uuid: Option<String>,
        #[serde(default)]
        restore_only: bool,
    },
    RestoreSaved {},
}

// Browsers retain PostgreSQL bigint IDs as strings to avoid IEEE-754 rounding.
fn deserialize_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Integer(i64),
        Text(String),
    }
    match Id::deserialize(deserializer)? {
        Id::Integer(id) => Ok(id),
        Id::Text(id) => id.parse().map_err(serde::de::Error::custom),
    }
}
