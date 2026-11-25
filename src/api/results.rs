//! Portable run result bundles, shared by the CLI and deferred local exports.
use crate::{
    PgStore,
    api::ApiError,
    core::{RunReadStore, RunTaskStore},
};
use serde_json::{Value, json};

pub async fn run_results(
    store: &PgStore,
    run_id: i32,
    task_id: Option<i64>,
) -> Result<Value, ApiError> {
    let run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id}")))?;
    let mut tasks = Vec::new();
    for task in store.list_run_tasks(run_id).await? {
        if task_id.is_some_and(|id| id != task.id) {
            continue;
        }
        let snapshot = store
            // Result exports must retain histograms and diagnostics. The compact
            // plot-history cache deliberately strips these potentially large fields.
            .get_task_output_snapshots(run_id, task.id, None, 1)
            .await?
            .into_iter()
            .next();
        tasks.push(json!({"id": task.id.to_string(), "name": task.name, "state": task.state, "failure_reason": task.failure_reason,
            "samples": task.nr_completed_samples, "snapshot": snapshot,
            "measurement": task.measurement_output, "controller": task.controller_output}));
    }
    if task_id.is_some() && tasks.is_empty() {
        return Err(ApiError::NotFound("task does not belong to run".into()));
    }
    Ok(json!({"format_version": 1, "run": run, "tasks": tasks}))
}
