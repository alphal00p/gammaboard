use crate::core::SamplerRuntimeMetrics;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, PlotPoint,
    history_x, key_value, key_value_panel, merge_panel_state, panel_spec, replace_panel,
    scalar_timeseries_panel, with_panel_width,
};
use crate::stores::{EvaluatorPerformanceHistoryEntry, SamplerPerformanceHistoryEntry};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub fn build_evaluator_performance_response(
    scope_id: Option<String>,
    entries: Vec<EvaluatorPerformanceHistoryEntry>,
) -> PanelResponse {
    build_performance_response(
        scope_id.unwrap_or_else(|| "evaluator".to_string()),
        entries,
        evaluator_panel_specs(),
        |entry| entry.id.to_string(),
        evaluator_panels,
    )
}

pub fn build_sampler_performance_response(
    scope_id: Option<String>,
    entries: Vec<SamplerPerformanceHistoryEntry>,
) -> PanelResponse {
    let mut response = build_performance_response(
        scope_id.unwrap_or_else(|| "sampler".to_string()),
        entries.clone(),
        sampler_panel_specs(),
        |entry| entry.id.to_string(),
        sampler_panels,
    );
    if let Some(latest) = entries.first() {
        for panel in sampler_current_panels(latest) {
            response.updates.push(replace_panel(panel));
        }
    }
    response
}

fn build_performance_response<T>(
    source_id: String,
    entries: Vec<T>,
    panels: Vec<PanelSpec>,
    cursor_for: impl Fn(&T) -> String,
    build_panels: impl Fn(&T) -> Vec<PanelState>,
) -> PanelResponse {
    let cursor = entries.first().map(cursor_for);
    let mut state_by_id = BTreeMap::new();
    for entry in entries.iter().rev() {
        for panel in build_panels(entry) {
            let panel_id = panel.panel_id().to_string();
            if let Some(existing) = state_by_id.get_mut(&panel_id) {
                merge_panel_state(existing, panel);
            } else {
                state_by_id.insert(panel_id, panel);
            }
        }
    }

    PanelResponse {
        source_id,
        cursor,
        reset_required: false,
        panels,
        updates: state_by_id.into_values().map(replace_panel).collect(),
    }
}

fn evaluator_panel_specs() -> Vec<PanelSpec> {
    vec![
        with_panel_width(
            panel_spec(
                "evaluator_idle_ratio",
                "Idle Ratio",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "evaluator_evaluate_time_us",
                "Evaluate Time Per Sample (us)",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "evaluator_materialization_time_us",
                "Materialization Time Per Sample (us)",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
    ]
}

fn sampler_panel_specs() -> Vec<PanelSpec> {
    vec![
        with_panel_width(
            panel_spec(
                "sampler_runtime_current",
                "Sampler Runtime",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_queue_buffer_current",
                "Sampler Queue",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_completed_samples_per_second",
                "Completed Samples Per Second",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_runnable_queue_retained_ratio",
                "Pending Queue Carryover Ratio (Diagnostic)",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
    ]
}

fn evaluator_panels(entry: &EvaluatorPerformanceHistoryEntry) -> Vec<PanelState> {
    let mut panels = vec![
        scalar_point_panel(
            "evaluator_evaluate_time_us",
            history_x(entry.created_at),
            ms_to_us(entry.metrics.avg_evaluate_time_per_sample_ms),
            Some(ms_to_us(
                entry.metrics.avg_evaluate_time_per_sample_ms
                    - entry.metrics.std_evaluate_time_per_sample_ms,
            )),
            Some(ms_to_us(
                entry.metrics.avg_evaluate_time_per_sample_ms
                    + entry.metrics.std_evaluate_time_per_sample_ms,
            )),
        ),
        scalar_point_panel(
            "evaluator_materialization_time_us",
            history_x(entry.created_at),
            ms_to_us(entry.metrics.avg_materialization_time_per_sample_ms),
            Some(ms_to_us(
                entry.metrics.avg_materialization_time_per_sample_ms
                    - entry.metrics.std_materialization_time_per_sample_ms,
            )),
            Some(ms_to_us(
                entry.metrics.avg_materialization_time_per_sample_ms
                    + entry.metrics.std_materialization_time_per_sample_ms,
            )),
        ),
    ];
    if let Some(idle_profile) = entry.metrics.idle_profile.as_ref() {
        panels.insert(
            0,
            scalar_point_panel(
                "evaluator_idle_ratio",
                history_x(entry.created_at),
                idle_profile.idle_ratio,
                None,
                None,
            ),
        );
    }
    panels
}

fn sampler_panels(entry: &SamplerPerformanceHistoryEntry) -> Vec<PanelState> {
    let Some(runtime) = decode_sampler_runtime_metrics(entry) else {
        return Vec::new();
    };
    vec![
        scalar_point_panel(
            "sampler_completed_samples_per_second",
            history_x(entry.created_at),
            runtime.completed_samples_per_second,
            None,
            None,
        ),
        scalar_point_panel(
            "sampler_runnable_queue_retained_ratio",
            history_x(entry.created_at),
            runtime
                .rolling
                .runnable_queue_retained_ratio
                .mean
                .unwrap_or(0.0),
            Some(
                runtime
                    .rolling
                    .runnable_queue_retained_ratio
                    .mean
                    .unwrap_or(0.0)
                    - runtime.rolling.runnable_queue_retained_ratio.std_dev,
            ),
            Some(
                runtime
                    .rolling
                    .runnable_queue_retained_ratio
                    .mean
                    .unwrap_or(0.0)
                    + runtime.rolling.runnable_queue_retained_ratio.std_dev,
            ),
        ),
    ]
}

fn sampler_current_panels(entry: &SamplerPerformanceHistoryEntry) -> Vec<PanelState> {
    let Some(runtime) = decode_sampler_runtime_metrics(entry) else {
        return Vec::new();
    };
    let mut panels = vec![key_value_panel(
        "sampler_runtime_current",
        vec![
            key_value(
                "completed_samples_per_second",
                "Completed Samples Per Second",
                runtime.completed_samples_per_second,
            ),
            key_value(
                "batch_size_current",
                "Batch Size Current",
                runtime.batch_size_current,
            ),
            key_value(
                "eval_ms_per_sample",
                "Eval Ms Per Sample",
                runtime.rolling.eval_ms_per_sample.mean,
            ),
            key_value(
                "eval_ms_per_batch",
                "Eval Ms Per Batch",
                runtime.rolling.eval_ms_per_batch.mean,
            ),
            key_value(
                "sampler_produce_ms_per_sample",
                "Sampler Produce Ms Per Sample",
                runtime.rolling.sampler_produce_ms_per_sample.mean,
            ),
            key_value(
                "sampler_ingest_ms_per_sample",
                "Sampler Ingest Ms Per Sample",
                runtime.rolling.sampler_ingest_ms_per_sample.mean,
            ),
            key_value(
                "sampler_tick_ms",
                "Sampler Tick Ms",
                runtime.rolling.sampler_tick_ms.mean,
            ),
            key_value(
                "batches_consumed_per_second",
                "Batches Consumed Per Second",
                runtime.rolling.batches_consumed_per_second.mean,
            ),
            key_value(
                "runnable_batches_consumed_per_tick",
                "Pending Batches Consumed Per Tick",
                runtime.rolling.runnable_batches_consumed_per_tick.mean,
            ),
            key_value(
                "runnable_queue_retained_ratio",
                "Pending Queue Carryover Ratio (Diagnostic)",
                runtime.rolling.runnable_queue_retained_ratio.mean,
            ),
        ],
    )];
    if let Some(queue_buffer) = sampler_queue_buffer_panel(&entry.engine_diagnostics) {
        panels.push(queue_buffer);
    }
    panels
}

fn sampler_queue_buffer_panel(value: &JsonValue) -> Option<PanelState> {
    let runner = runner_diagnostics(value)?;
    let entries = vec![
        runner_value_entry(runner, "queue_buffer", "Queue Buffer"),
        runner_value_entry(runner, "active_evaluator_count", "Active Evaluators"),
        runner_value_entry(runner, "target_pending_batches", "Target Pending Batches"),
        runner_value_entry(runner, "pending_batches", "Pending Batches"),
        runner_value_entry(runner, "claimed_batches", "Claimed Batches"),
        runner_value_entry(runner, "completed_batches", "Completed Batches"),
        runner_value_entry(runner, "open_batches", "Open Batches"),
        runner_value_entry(runner, "observable_checkpoint_state", "Checkpoint State"),
        runner_value_entry(
            runner,
            "training_samples_remaining",
            "Training Samples Remaining",
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    Some(key_value_panel("sampler_queue_buffer_current", entries))
}

fn scalar_point_panel(
    panel_id: &str,
    x: f64,
    y: f64,
    y_min: Option<f64>,
    y_max: Option<f64>,
) -> PanelState {
    scalar_timeseries_panel(panel_id, vec![PlotPoint { x, y, y_min, y_max }])
}

fn decode_sampler_runtime_metrics(
    entry: &SamplerPerformanceHistoryEntry,
) -> Option<SamplerRuntimeMetrics> {
    serde_json::from_value(entry.runtime_metrics.clone()).ok()
}

fn ms_to_us(value_ms: f64) -> f64 {
    value_ms * 1000.0
}

fn runner_diagnostics(value: &JsonValue) -> Option<&serde_json::Map<String, JsonValue>> {
    value.as_object()?.get("runner")?.as_object()
}

fn runner_value_entry(
    runner: &serde_json::Map<String, JsonValue>,
    key: &str,
    label: &str,
) -> Option<crate::server::panels::KeyValueEntry> {
    runner
        .get(key)
        .cloned()
        .map(|value| key_value(key, label, value))
}
