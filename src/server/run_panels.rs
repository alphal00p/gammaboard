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
                "run_evaluator",
                "Evaluator Performance",
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
    let active_evaluator_count = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "active_evaluator_count"));
    let target_pending_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "target_pending_batches"));
    let pending_batches = active_sampler
        .and_then(|worker| worker.sampler_engine_diagnostics.as_ref())
        .and_then(|value| runner_diagnostic_i64(value, "pending_batches"));
    let pending_shortfall = target_pending_batches
        .zip(pending_batches)
        .map(|(target, pending)| target.saturating_sub(pending));
    let current_batch_size = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(batch_size_current);
    let current_batch_eval_ms = active_sampler
        .and_then(|worker| worker.sampler_runtime_metrics.as_ref())
        .and_then(batch_eval_ms_mean);
    let evaluator_summary = summarize_evaluator_metrics(workers, run.run_id);

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
                    "Avg Pending Queue Carryover Ratio (Diagnostic)",
                    avg_queue_remaining,
                ),
                key_value(
                    "queue_buffer",
                    "Queue Buffer",
                    run_spec.sampler_aggregator_runner_params.queue_buffer,
                ),
                key_value(
                    "active_evaluator_count",
                    "Active Evaluators",
                    active_evaluator_count,
                ),
                key_value(
                    "target_pending_batches",
                    "Target Pending Batches",
                    target_pending_batches,
                ),
                key_value("pending_shortfall", "Pending Shortfall", pending_shortfall),
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
        key_value_panel(
            "run_evaluator",
            vec![
                key_value(
                    "active_evaluators_with_metrics",
                    "Active Evaluators With Metrics",
                    evaluator_summary.evaluator_count,
                ),
                key_value(
                    "avg_fetch_time_us",
                    "Avg Fetch+Decode Per Sample (us)",
                    evaluator_summary
                        .avg_fetch_time_per_sample_ms
                        .map(|value| value * 1000.0),
                ),
                key_value(
                    "avg_fetch_stall_time_us",
                    "Avg Fetch Stall Per Sample (us)",
                    evaluator_summary
                        .avg_fetch_stall_time_per_sample_ms
                        .map(|value| value * 1000.0),
                ),
                key_value(
                    "avg_prefetch_hit_ratio",
                    "Avg Prefetch Hit Ratio",
                    evaluator_summary.avg_prefetch_hit_ratio,
                ),
                key_value(
                    "avg_fetch_stall_ratio",
                    "Avg Fetch Stall Ratio",
                    evaluator_summary.avg_fetch_stall_ratio,
                ),
                key_value(
                    "avg_queue_starvation_ratio",
                    "Avg Queue Starvation Ratio",
                    evaluator_summary.avg_queue_starvation_ratio,
                ),
                key_value(
                    "avg_materialization_time_us",
                    "Avg Materialization Per Sample (us)",
                    evaluator_summary
                        .avg_materialization_time_per_sample_ms
                        .map(|value| value * 1000.0),
                ),
                key_value(
                    "avg_evaluate_time_us",
                    "Avg Evaluate Per Sample (us)",
                    evaluator_summary
                        .avg_evaluate_time_per_sample_ms
                        .map(|value| value * 1000.0),
                ),
                key_value(
                    "avg_submit_time_us",
                    "Avg Submit Per Sample (us)",
                    evaluator_summary
                        .avg_submit_time_per_sample_ms
                        .map(|value| value * 1000.0),
                ),
                key_value(
                    "avg_submit_stall_time_us",
                    "Avg Submit Stall Per Sample (us)",
                    evaluator_summary
                        .avg_submit_stall_time_per_sample_ms
                        .map(|value| value * 1000.0),
                ),
                key_value(
                    "avg_submit_slot_hit_ratio",
                    "Avg Submit Slot Hit Ratio",
                    evaluator_summary.avg_submit_slot_hit_ratio,
                ),
                key_value(
                    "avg_submit_stall_ratio",
                    "Avg Submit Stall Ratio",
                    evaluator_summary.avg_submit_stall_ratio,
                ),
                key_value("idle_ratio", "Idle Ratio", evaluator_summary.avg_idle_ratio),
            ],
        ),
        text_panel("run_target", &target_summary(run.target.as_ref())),
    ])
}

struct EvaluatorSummary {
    evaluator_count: usize,
    avg_fetch_time_per_sample_ms: Option<f64>,
    avg_fetch_stall_time_per_sample_ms: Option<f64>,
    avg_prefetch_hit_ratio: Option<f64>,
    avg_fetch_stall_ratio: Option<f64>,
    avg_queue_starvation_ratio: Option<f64>,
    avg_materialization_time_per_sample_ms: Option<f64>,
    avg_evaluate_time_per_sample_ms: Option<f64>,
    avg_submit_time_per_sample_ms: Option<f64>,
    avg_submit_stall_time_per_sample_ms: Option<f64>,
    avg_submit_slot_hit_ratio: Option<f64>,
    avg_submit_stall_ratio: Option<f64>,
    avg_idle_ratio: Option<f64>,
}

fn summarize_evaluator_metrics(workers: &[RegisteredWorkerEntry], run_id: i32) -> EvaluatorSummary {
    let mut count = 0usize;
    let mut fetch_sum = 0.0;
    let mut fetch_stall_sum = 0.0;
    let mut prefetch_hit_sum = 0.0;
    let mut fetch_stall_ratio_sum = 0.0;
    let mut queue_starvation_ratio_sum = 0.0;
    let mut materialization_sum = 0.0;
    let mut evaluate_sum = 0.0;
    let mut submit_sum = 0.0;
    let mut submit_stall_sum = 0.0;
    let mut submit_slot_hit_sum = 0.0;
    let mut submit_stall_ratio_sum = 0.0;
    let mut idle_sum = 0.0;

    for worker in workers.iter().filter(|worker| {
        worker.current_run_id == Some(run_id)
            && worker.current_role.as_deref() == Some("evaluator")
            && worker.evaluator_metrics.is_some()
    }) {
        let metrics = worker
            .evaluator_metrics
            .as_ref()
            .expect("checked evaluator metrics");
        count += 1;
        fetch_sum += metrics.avg_fetch_time_per_sample_ms;
        fetch_stall_sum += metrics.avg_fetch_stall_time_per_sample_ms;
        prefetch_hit_sum += metrics.prefetch_hit_ratio;
        fetch_stall_ratio_sum += metrics.fetch_stall_ratio;
        queue_starvation_ratio_sum += metrics.queue_starvation_ratio;
        materialization_sum += metrics.avg_materialization_time_per_sample_ms;
        evaluate_sum += metrics.avg_evaluate_time_per_sample_ms;
        submit_sum += metrics.avg_submit_time_per_sample_ms;
        submit_stall_sum += metrics.avg_submit_stall_time_per_sample_ms;
        submit_slot_hit_sum += metrics.submit_slot_hit_ratio;
        submit_stall_ratio_sum += metrics.submit_stall_ratio;
        idle_sum += metrics
            .idle_profile
            .as_ref()
            .map(|profile| profile.idle_ratio)
            .unwrap_or(0.0);
    }

    if count == 0 {
        return EvaluatorSummary {
            evaluator_count: 0,
            avg_fetch_time_per_sample_ms: None,
            avg_fetch_stall_time_per_sample_ms: None,
            avg_prefetch_hit_ratio: None,
            avg_fetch_stall_ratio: None,
            avg_queue_starvation_ratio: None,
            avg_materialization_time_per_sample_ms: None,
            avg_evaluate_time_per_sample_ms: None,
            avg_submit_time_per_sample_ms: None,
            avg_submit_stall_time_per_sample_ms: None,
            avg_submit_slot_hit_ratio: None,
            avg_submit_stall_ratio: None,
            avg_idle_ratio: None,
        };
    }

    let count_f64 = count as f64;
    EvaluatorSummary {
        evaluator_count: count,
        avg_fetch_time_per_sample_ms: Some(fetch_sum / count_f64),
        avg_fetch_stall_time_per_sample_ms: Some(fetch_stall_sum / count_f64),
        avg_prefetch_hit_ratio: Some(prefetch_hit_sum / count_f64),
        avg_fetch_stall_ratio: Some(fetch_stall_ratio_sum / count_f64),
        avg_queue_starvation_ratio: Some(queue_starvation_ratio_sum / count_f64),
        avg_materialization_time_per_sample_ms: Some(materialization_sum / count_f64),
        avg_evaluate_time_per_sample_ms: Some(evaluate_sum / count_f64),
        avg_submit_time_per_sample_ms: Some(submit_sum / count_f64),
        avg_submit_stall_time_per_sample_ms: Some(submit_stall_sum / count_f64),
        avg_submit_slot_hit_ratio: Some(submit_slot_hit_sum / count_f64),
        avg_submit_stall_ratio: Some(submit_stall_ratio_sum / count_f64),
        avg_idle_ratio: Some(idle_sum / count_f64),
    }
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
