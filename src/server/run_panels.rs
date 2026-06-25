use crate::core::{EngineError, RunSpec, RunTask};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, key_value,
    key_value_panel, replace_panel, sized_panel_spec, text_panel,
};
use crate::stores::{RegisteredWorkerEntry, RunProgress};
use serde_json::Value as JsonValue;

pub fn build_run_panel_response(
    run: &RunProgress,
    run_spec: &RunSpec,
    tasks: &[RunTask],
    workers: &[RegisteredWorkerEntry],
) -> Result<PanelResponse, EngineError> {
    let source_id = format!("run:{}:summary", run.run_id);
    let panels = panel_specs(run_spec);
    let updates = panel_states(run, run_spec, tasks, workers)?
        .into_iter()
        .map(replace_panel)
        .collect();
    Ok(PanelResponse {
        source_id,
        cursor: None,
        reset_required: false,
        panels,
        updates,
        poll_after_ms: Some(5000),
    })
}

fn panel_specs(_run_spec: &RunSpec) -> Vec<PanelSpec> {
    let panels = vec![
        sized_panel_spec(
            "run_identity",
            "Run Identity",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Half,
        ),
        sized_panel_spec(
            "run_lifecycle",
            "Lifecycle",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Half,
        ),
        sized_panel_spec(
            "run_progress",
            "Progress",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Half,
        ),
        sized_panel_spec(
            "run_queue",
            "Queue",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Half,
        ),
        sized_panel_spec(
            "run_batch",
            "Batch",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Half,
        ),
        sized_panel_spec(
            "run_engine",
            "Engine Summary",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
            PanelWidth::Half,
        ),
        sized_panel_spec(
            "run_target",
            "Target",
            PanelKind::Text,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
    ];
    panels
}

fn panel_states(
    run: &RunProgress,
    run_spec: &RunSpec,
    tasks: &[RunTask],
    workers: &[RegisteredWorkerEntry],
) -> Result<Vec<PanelState>, EngineError> {
    let current_task = tasks.iter().find(|task| task.state.as_str() == "active");
    let active_sampler = workers.iter().find(|worker| {
        worker.current_run_id == Some(run.run_id)
            && worker.current_role.as_deref() == Some("sampler_aggregator")
    });
    let active_evaluator_count = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "active_evaluator_count"));
    let target_pending_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "target_pending_batches"));
    let db_pending_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "db_pending_batches"));
    let pending_shortfall = target_pending_batches
        .zip(db_pending_batches)
        .map(|(target, pending)| target.saturating_sub(pending));
    let local_pending_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "local_pending_batches"));
    let local_inflight_insert_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "local_inflight_insert_batches"));
    let local_inflight_insert_tasks = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "local_inflight_insert_tasks"));
    let local_ready_processed_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "local_ready_processed_batches"));
    let current_batch_size = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(batch_size_current);
    let current_batch_eval_ms = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(batch_eval_ms_mean);
    let panels = vec![
        key_value_panel(
            "run_identity",
            vec![
                key_value("run_id", "Run ID", run.run_id),
                key_value("run_name", "Run Name", run.run_name.as_str()),
                key_value("state", "State", run.lifecycle_state.as_str()),
                key_value(
                    "active_task",
                    "Active Task",
                    current_task_label(current_task),
                ),
            ],
        ),
        key_value_panel(
            "run_lifecycle",
            vec![
                key_value(
                    "started_at",
                    "Started",
                    run.started_at.map(|value| value.to_rfc3339()),
                ),
                key_value(
                    "completed_at",
                    "Completed",
                    run.completed_at.map(|value| value.to_rfc3339()),
                ),
                key_value("active_workers", "Active Workers", run.active_worker_count),
                key_value(
                    "desired_assignments",
                    "Desired Assignments",
                    run.desired_assignment_count,
                ),
            ],
        ),
        key_value_panel(
            "run_progress",
            vec![
                key_value("tasks", "Run Tasks", tasks.len()),
                key_value("produced", "Produced Samples", run.nr_produced_samples),
                key_value(
                    "completed",
                    "Completed Samples",
                    run.nr_completed_samples_including_children,
                ),
                key_value("completion_rate", "Completion Rate", run.completion_rate),
            ],
        ),
        key_value_panel(
            "run_queue",
            vec![
                key_value("failed", "Failed Batches", run.failed_batches),
                key_value(
                    "queue_buffer",
                    "Target Pending Batches / Evaluator",
                    run_spec.sampler_aggregator_runner_params.queue.queue_buffer,
                ),
                key_value(
                    "active_evaluator_count",
                    "Active Evaluators",
                    active_evaluator_count,
                ),
                key_value(
                    "db_pending_batches",
                    "DB Pending Batches",
                    db_pending_batches,
                ),
                key_value(
                    "target_pending_batches",
                    "Target DB Pending Batches",
                    target_pending_batches,
                ),
                key_value(
                    "pending_shortfall",
                    "DB Pending Shortfall",
                    pending_shortfall,
                ),
                key_value(
                    "local_pending_batches",
                    "Local Pending Batches",
                    local_pending_batches,
                ),
                key_value(
                    "local_inflight_insert_tasks",
                    "Local In-Flight Insert Tasks",
                    local_inflight_insert_tasks,
                ),
                key_value(
                    "local_inflight_insert_batches",
                    "Local In-Flight Insert Batches",
                    local_inflight_insert_batches,
                ),
                key_value(
                    "local_ready_processed_batches",
                    "Completed Prefetch Buffer",
                    local_ready_processed_batches,
                ),
            ],
        ),
        key_value_panel(
            "run_batch",
            vec![
                key_value(
                    "current_batch_size",
                    "Current Batch Size",
                    current_batch_size,
                ),
                key_value(
                    "max_batch_size",
                    "Max Batch Size",
                    run_spec
                        .sampler_aggregator_runner_params
                        .queue
                        .max_batch_size,
                ),
                key_value(
                    "current_batch_eval_ms",
                    "Current Batch Eval (ms)",
                    current_batch_eval_ms,
                ),
                key_value(
                    "target_batch_eval_ms",
                    "Target Batch Eval (ms)",
                    run_spec
                        .sampler_aggregator_runner_params
                        .queue
                        .target_batch_eval_ms,
                ),
            ],
        ),
        key_value_panel(
            "run_engine",
            vec![
                key_value(
                    "evaluator",
                    "Evaluator",
                    run_spec
                        .evaluator
                        .as_ref()
                        .map(kind_of)
                        .unwrap_or_else(|| "none".to_string()),
                ),
                key_value(
                    "accumulator",
                    "Accumulator",
                    current_task
                        .map(|task| accumulator_label(&task.task))
                        .unwrap_or_else(|| "none".to_string()),
                ),
                key_value("domain", "Domain", &run_spec.domain),
                key_value(
                    "sampler",
                    "Sampler",
                    current_task
                        .map(sampler_label)
                        .unwrap_or_else(|| "none".to_string()),
                ),
            ],
        ),
        text_panel("run_target", target_summary(run.target.as_ref())),
    ];
    Ok(panels)
}

fn accumulator_label(task: &crate::core::RunTaskSpec) -> String {
    match task.new_accumulator_config() {
        Ok(Some(config)) => kind_of(&config),
        Ok(None) => "reuse_previous".to_string(),
        Err(_) => "none".to_string(),
    }
}

fn sampler_label(task: &RunTask) -> String {
    let spec = &task.task;
    if let Some(config) = spec
        .sample_sampler_config()
        .or_else(|| spec.sampler_config())
    {
        return kind_of(&config);
    }
    match spec.sample_sampler_source() {
        Some(crate::core::SourceRefSpec::Latest) => "reuse_previous".to_string(),
        Some(crate::core::SourceRefSpec::FromName(name)) => format!("from:{name}"),
        None => "none".to_string(),
    }
}

fn current_task_label(task: Option<&RunTask>) -> String {
    task.map(|task| format!("{} ({})", task.name, task.task.kind_str()))
        .unwrap_or_else(|| "none".to_string())
}

fn batch_size_current(metrics: &JsonValue) -> Option<usize> {
    metrics
        .as_object()
        .and_then(|value| value.get("batch_size_current"))
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn batch_eval_ms_mean(metrics: &JsonValue) -> Option<f64> {
    metrics
        .as_object()
        .and_then(|value| value.get("sampler"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("eval_ms_per_batch"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("mean"))
        .and_then(JsonValue::as_f64)
}

fn runner_diagnostic_i64(metrics: &JsonValue, key: &str) -> Option<i64> {
    metrics
        .as_object()
        .and_then(|value| value.get("runner"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get(key))
        .and_then(JsonValue::as_i64)
}

fn kind_of(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| match value {
            JsonValue::String(value) => Some(value),
            JsonValue::Object(value) => value
                .get("kind")
                .and_then(JsonValue::as_str)
                .map(str::to_string),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn target_summary(target: Option<&JsonValue>) -> String {
    match target {
        None => "none".to_string(),
        Some(JsonValue::Object(value))
            if value.get("kind").and_then(JsonValue::as_str) == Some("scalar") =>
        {
            value
                .get("value")
                .map(JsonValue::to_string)
                .map(|value| format!("scalar({value})"))
                .unwrap_or_else(|| "scalar".to_string())
        }
        Some(JsonValue::Object(value))
            if value.get("kind").and_then(JsonValue::as_str) == Some("vector") =>
        {
            value
                .get("components")
                .or_else(|| value.get("values"))
                .map(JsonValue::to_string)
                .map(|value| format!("vector({value})"))
                .unwrap_or_else(|| "vector".to_string())
        }
        Some(value) => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}
