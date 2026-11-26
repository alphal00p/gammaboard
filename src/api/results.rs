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
        tasks.push(json!({"id": task.id.to_string(), "name": task.name, "definition":task.task, "state": task.state, "failure_reason": task.failure_reason,
            "samples": task.nr_completed_samples, "snapshot": snapshot,
            "measurement": task.measurement_output, "controller": task.controller_output}));
    }
    if task_id.is_some() && tasks.is_empty() {
        return Err(ApiError::NotFound("task does not belong to run".into()));
    }
    Ok(json!({"format_version": 2, "run": run, "tasks": tasks}))
}

/// Import data only. Never preprocess a card, load an evaluator, or restore a sampler.
pub async fn import_results(
    store: &PgStore,
    bundle: Value,
    name: Option<String>,
) -> anyhow::Result<Value> {
    use anyhow::{Context, ensure};
    ensure!(
        bundle["format_version"] == 2,
        "import requires result format 2; export again with the current gb run results command"
    );
    let source: crate::stores::RunProgress = serde_json::from_value(bundle["run"].clone())?;
    let name = name.unwrap_or_else(|| format!("{} (archive)", source.run_name));
    ensure!(!name.trim().is_empty(), "archive name must not be empty");
    let domain = source
        .domain
        .as_ref()
        .context("result bundle has no domain")?;
    domain.validate().map_err(anyhow::Error::msg)?;
    let tasks = bundle["tasks"]
        .as_array()
        .context("result bundle has no tasks")?;
    ensure!(!tasks.is_empty(), "result bundle contains no tasks");
    let mut validated = Vec::new();
    for task in tasks {
        let definition: crate::RunTaskSpec = serde_json::from_value(task["definition"].clone())?;
        ensure!(
            matches!(task["state"].as_str(), Some("completed" | "failed")),
            "only completed or failed task snapshots can be archived"
        );
        let samples = task["samples"]
            .as_i64()
            .context("invalid task sample count")?;
        ensure!(samples >= 0, "negative sample count");
        validated.push((task, definition, samples));
    }
    let mut tx = store.pool().begin().await?;
    let id:i32=sqlx::query_scalar("INSERT INTO runs(name,run_toml,provenance,integration_params,point_spec,target,target_error,nr_produced_samples,nr_completed_samples,started_at,completed_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING id")
        .bind(&name).bind(&source.run_toml).bind(json!({"archive":{"source_run_id":source.run_id,"source_name":source.run_name,"source_provenance":source.provenance,"imported_at":chrono::Utc::now(),"history":"latest task outputs only; not resumable"}}))
        .bind(&source.integration_params).bind(serde_json::to_value(domain)?).bind(&source.target).bind(&source.target_error)
        .bind(source.nr_produced_samples).bind(source.nr_completed_samples).bind(source.started_at).bind(source.completed_at)
        .fetch_one(&mut *tx).await?;
    for (index, (task, definition, samples)) in validated.into_iter().enumerate() {
        let task_id:i64=sqlx::query_scalar("INSERT INTO run_tasks(run_id,name,sequence_nr,task,task_toml,state,nr_produced_samples,nr_completed_samples,measurement_output,controller_output,failure_reason) VALUES($1,$2,$3,$4,$5,$6,$7,$7,$8,$9,$10) RETURNING id")
            .bind(id).bind(task["name"].as_str().context("task has no name")?).bind(index as i32+1)
            .bind(serde_json::to_value(&definition)?).bind(toml::to_string(&definition)?).bind(task["state"].as_str().unwrap()).bind(samples)
            .bind(task.get("measurement").filter(|v|!v.is_null())).bind(task.get("controller").filter(|v|!v.is_null())).bind(task["failure_reason"].as_str())
            .fetch_one(&mut *tx).await?;
        if !task["snapshot"].is_null() {
            let snapshot: crate::stores::TaskOutputSnapshot =
                serde_json::from_value(task["snapshot"].clone())?;
            sqlx::query("INSERT INTO persisted_observable_snapshots(run_id,task_id,persisted_observable,created_at) VALUES($1,$2,$3,COALESCE($4,now()))")
                .bind(id).bind(task_id).bind(&snapshot.persisted_output).bind(snapshot.created_at).execute(&mut *tx).await?;
        }
    }
    sqlx::query("UPDATE runs SET archived=true WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({"run_id":id,"run_name":name,"archived":true,"tasks_imported":tasks.len()}))
}
