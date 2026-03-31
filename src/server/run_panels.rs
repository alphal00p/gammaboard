use crate::core::{EngineError, RunSpec, RunTask};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, key_value,
    key_value_panel, panel_spec, replace_panel, text_panel, with_panel_width,
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
    let panels = panel_specs();
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
    })
}

fn panel_specs() -> Vec<PanelSpec> {
    vec![
        with_panel_width(
            panel_spec(
                "run_identity",
                "Run Identity",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "run_lifecycle",
                "Lifecycle",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "run_progress",
                "Progress",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "run_queue",
                "Queue",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "run_batch",
                "Batch",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "run_engine",
                "Engine Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "run_target",
                "Target",
                PanelKind::Text,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
    ]
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
    let avg_queue_remaining = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(queue_remaining_mean);
    let queue_target_multiplier = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_f64(value, "queue_target_multiplier"));
    let target_runnable_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "target_runnable_batches_final"));
    let runnable_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "runnable_batches"));
    let current_batch_size = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(batch_size_current);
    let current_batch_eval_ms = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(batch_eval_ms_mean);

    Ok(vec![
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
                key_value("completed", "Completed Samples", run.nr_completed_samples),
                key_value("completion_rate", "Completion Rate", run.completion_rate),
            ],
        ),
        key_value_panel(
            "run_queue",
            vec![
                key_value("pending", "Pending Batches", run.pending_batches),
                key_value("claimed", "Claimed Batches", run.claimed_batches),
                key_value("failed", "Failed Batches", run.failed_batches),
                key_value("completed", "Completed Batches", run.completed_batches),
                key_value(
                    "avg_queue_remaining",
                    "Avg Runnable Queue Retained Per Tick (Diagnostic)",
                    avg_queue_remaining,
                ),
                key_value(
                    "queue_buffer",
                    "Queue Buffer",
                    run_spec.sampler_aggregator_runner_params.queue_buffer,
                ),
                key_value(
                    "queue_target_multiplier",
                    "Queue Target Multiplier",
                    queue_target_multiplier,
                ),
                key_value(
                    "target_runnable_batches",
                    "Target Runnable Batches",
                    target_runnable_batches,
                ),
                key_value(
                    "runnable_batches",
                    "Current Runnable Batches",
                    runnable_batches,
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
                    run_spec.sampler_aggregator_runner_params.max_batch_size,
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
                        .target_batch_eval_ms,
                ),
            ],
        ),
        key_value_panel(
            "run_engine",
            vec![
                key_value("evaluator", "Evaluator", kind_of(&run_spec.evaluator)),
                key_value(
                    "observable",
                    "Observable",
                    current_task
                        .map(|task| observable_label(&task.task))
                        .unwrap_or_else(|| "none".to_string()),
                ),
                key_value(
                    "domain",
                    "Domain",
                    serde_json::to_string(&run_spec.domain)
                        .unwrap_or_else(|_| "<invalid domain>".to_string()),
                ),
                key_value(
                    "sampler",
                    "Sampler",
                    current_task
                        .and_then(|task| task.task.sampler_config())
                        .map(|config| kind_of(&config))
                        .unwrap_or_else(|| "none".to_string()),
                ),
            ],
        ),
        text_panel("run_target", &target_summary(run.target.as_ref())),
    ])
}

fn observable_label(task: &crate::core::RunTaskSpec) -> String {
    match task.new_observable_config() {
        Ok(Some(config)) => kind_of(&config),
        Ok(None) => "reuse_previous".to_string(),
        Err(_) => "none".to_string(),
    }
}

fn current_task_label(task: Option<&RunTask>) -> String {
    task.map(|task| format!("{} ({})", task.name, task.task.kind_str()))
        .unwrap_or_else(|| "none".to_string())
}

fn queue_remaining_mean(metrics: &JsonValue) -> Option<f64> {
    metrics
        .as_object()
        .and_then(|value| value.get("rolling"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("runnable_queue_retained_ratio"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("mean"))
        .and_then(JsonValue::as_f64)
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
        .and_then(|value| value.get("rolling"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("eval_ms_per_batch"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("mean"))
        .and_then(JsonValue::as_f64)
}

fn runner_diagnostic_f64(metrics: &JsonValue, key: &str) -> Option<f64> {
    metrics
        .as_object()
        .and_then(|value| value.get("runner"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get(key))
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
        Some(value) => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}
