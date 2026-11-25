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
        offline: bool,
    },
    WorkerInspect {
        name: String,
    },
    WorkerLaunch {
        count: Option<usize>,
        toml: Option<String>,
    },
    WorkerStart {
        name: String,
    },
    WorkerRestore {},
    WorkerCancelRestoration {
        name: String,
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
        name: String,
    },
    WorkerRequests {
        id: Option<i64>,
    },
    WorkerRequestCancel {
        id: i64,
    },
    DatabaseStatus {},
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
    Inspect { id: i64 },
    Export { id: i64 },
    Append { toml: String },
    Duplicate { id: i64 },
    Edit { id: i64, toml: String },
    Tune { id: i64, toml: String },
    Remove { id: i64 },
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
