use crate::core::{EvaluatorPerformanceMetrics, SamplerRuntimeMetrics};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, PlotPoint,
    TickBreakdownSegment, history_x, key_value, key_value_panel, merge_panel_state, panel_spec,
    replace_panel, scalar_timeseries_panel, tick_breakdown_panel, with_panel_width,
};
use crate::stores::{EvaluatorPerformanceHistoryEntry, SamplerPerformanceHistoryEntry};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub fn build_evaluator_performance_response(
    scope_id: Option<String>,
    entries: Vec<EvaluatorPerformanceHistoryEntry>,
    include_summary: bool,
) -> PanelResponse {
    let source_id = scope_id.unwrap_or_else(|| "evaluator".to_string());
    let panels = evaluator_panel_specs(include_summary);
    let mut updates = Vec::new();
    if include_summary && !entries.is_empty() {
        updates.push(replace_panel(evaluator_summary_panel(&entries)));
    }
    if !include_summary && let Some(entry) = entries.first() {
        for panel in evaluator_current_panels(entry) {
            updates.push(replace_panel(panel));
        }
    }
    PanelResponse {
        source_id,
        cursor: entries.first().map(|entry| entry.id.to_string()),
        reset_required: false,
        panels,
        updates,
    }
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

fn evaluator_panel_specs(include_summary: bool) -> Vec<PanelSpec> {
    if include_summary {
        return vec![with_panel_width(
            panel_spec(
                "evaluator_summary",
                "Run Evaluator Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        )];
    }

    vec![
        with_panel_width(
            panel_spec(
                "evaluator_tick_breakdown",
                "Evaluator Time Breakdown",
                PanelKind::TickBreakdown,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "evaluator_overview",
                "Evaluator Overview",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "evaluator_pipeline_metrics",
                "Evaluator Pipeline Metrics",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
    ]
}

fn sampler_panel_specs() -> Vec<PanelSpec> {
    vec![
        with_panel_width(
            panel_spec(
                "sampler_tick_breakdown",
                "Sampler Tick Breakdown",
                PanelKind::TickBreakdown,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_runtime_overview",
                "Sampler Overview",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "sampler_runtime_efficiency",
                "Sampler Efficiency",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
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

fn evaluator_current_panels(entry: &EvaluatorPerformanceHistoryEntry) -> Vec<PanelState> {
    vec![
        tick_breakdown_panel(
            "evaluator_tick_breakdown",
            evaluator_tick_total_ms(&entry.metrics),
            evaluator_tick_segments(&entry.metrics),
        ),
        key_value_panel(
            "evaluator_overview",
            vec![
                key_value("worker_id", "Worker", entry.worker_id.as_str()),
                key_value(
                    "memory_usage",
                    "Memory Usage",
                    entry.rss_bytes.map(format_bytes_human),
                ),
                key_value(
                    "samples_evaluated",
                    "Samples Evaluated",
                    entry.metrics.samples_evaluated,
                ),
                key_value(
                    "avg_total_time_us",
                    "Avg Total Per Sample (us)",
                    ms_to_us(entry.metrics.avg_time_per_sample_ms),
                ),
                key_value(
                    "prefetch_hit_ratio",
                    "Prefetch Hit Ratio",
                    entry.metrics.prefetch_hit_ratio,
                ),
                key_value(
                    "fetch_stall_ratio",
                    "Fetch Stall Ratio",
                    entry.metrics.fetch_stall_ratio,
                ),
                key_value(
                    "submit_stall_ratio",
                    "Submit Stall Ratio",
                    entry.metrics.submit_stall_ratio,
                ),
                key_value(
                    "queue_starvation_ratio",
                    "Queue Starvation Ratio",
                    entry.metrics.queue_starvation_ratio,
                ),
                key_value(
                    "idle_ratio",
                    "Idle Ratio",
                    entry
                        .metrics
                        .idle_profile
                        .as_ref()
                        .map(|profile| profile.idle_ratio),
                ),
            ],
        ),
        key_value_panel(
            "evaluator_pipeline_metrics",
            vec![
                key_value(
                    "avg_fetch_decode_time_us",
                    "Fetch+Decode Per Sample (us)",
                    ms_to_us(entry.metrics.avg_fetch_time_per_sample_ms),
                ),
                key_value(
                    "avg_fetch_stall_time_us",
                    "Fetch Stall Per Sample (us)",
                    ms_to_us(entry.metrics.avg_fetch_stall_time_per_sample_ms),
                ),
                key_value(
                    "avg_materialization_time_us",
                    "Materialization Per Sample (us)",
                    ms_to_us(entry.metrics.avg_materialization_time_per_sample_ms),
                ),
                key_value(
                    "avg_evaluate_time_us",
                    "Evaluate Per Sample (us)",
                    ms_to_us(entry.metrics.avg_evaluate_time_per_sample_ms),
                ),
                key_value(
                    "avg_submit_time_us",
                    "Submit Per Sample (us)",
                    ms_to_us(entry.metrics.avg_submit_time_per_sample_ms),
                ),
                key_value(
                    "avg_submit_stall_time_us",
                    "Submit Stall Per Sample (us)",
                    ms_to_us(entry.metrics.avg_submit_stall_time_per_sample_ms),
                ),
                key_value(
                    "submit_slot_hit_ratio",
                    "Submit Slot Hit Ratio",
                    entry.metrics.submit_slot_hit_ratio,
                ),
            ],
        ),
    ]
}

fn evaluator_summary_panel(entries: &[EvaluatorPerformanceHistoryEntry]) -> PanelState {
    let summary = summarize_evaluator_metrics(entries);
    key_value_panel(
        "evaluator_summary",
        vec![
            key_value(
                "active_evaluators_with_metrics",
                "Active Evaluators With Metrics",
                summary.evaluator_count,
            ),
            key_value(
                "avg_total_time_us",
                "Avg Total Per Sample (us)",
                summary.avg_total_time_per_sample_ms.map(ms_to_us),
            ),
            key_value(
                "avg_fetch_stall_time_us",
                "Avg Fetch Stall Per Sample (us)",
                summary.avg_fetch_stall_time_per_sample_ms.map(ms_to_us),
            ),
            key_value(
                "avg_materialization_time_us",
                "Avg Materialization Per Sample (us)",
                summary.avg_materialization_time_per_sample_ms.map(ms_to_us),
            ),
            key_value(
                "avg_evaluate_time_us",
                "Avg Evaluate Per Sample (us)",
                summary.avg_evaluate_time_per_sample_ms.map(ms_to_us),
            ),
            key_value(
                "avg_submit_time_us",
                "Avg Submit Per Sample (us)",
                summary.avg_submit_time_per_sample_ms.map(ms_to_us),
            ),
            key_value(
                "avg_prefetch_hit_ratio",
                "Avg Prefetch Hit Ratio",
                summary.avg_prefetch_hit_ratio,
            ),
            key_value(
                "avg_queue_starvation_ratio",
                "Avg Queue Starvation Ratio",
                summary.avg_queue_starvation_ratio,
            ),
            key_value("avg_idle_ratio", "Avg Idle Ratio", summary.avg_idle_ratio),
        ],
    )
}

struct EvaluatorSummary {
    evaluator_count: usize,
    avg_total_time_per_sample_ms: Option<f64>,
    avg_fetch_stall_time_per_sample_ms: Option<f64>,
    avg_prefetch_hit_ratio: Option<f64>,
    avg_queue_starvation_ratio: Option<f64>,
    avg_materialization_time_per_sample_ms: Option<f64>,
    avg_evaluate_time_per_sample_ms: Option<f64>,
    avg_submit_time_per_sample_ms: Option<f64>,
    avg_idle_ratio: Option<f64>,
}

fn summarize_evaluator_metrics(entries: &[EvaluatorPerformanceHistoryEntry]) -> EvaluatorSummary {
    let mut latest_by_worker = BTreeMap::<&str, &EvaluatorPerformanceMetrics>::new();
    for entry in entries {
        latest_by_worker
            .entry(entry.worker_id.as_str())
            .or_insert(&entry.metrics);
    }

    let count = latest_by_worker.len();
    if count == 0 {
        return EvaluatorSummary {
            evaluator_count: 0,
            avg_total_time_per_sample_ms: None,
            avg_fetch_stall_time_per_sample_ms: None,
            avg_prefetch_hit_ratio: None,
            avg_queue_starvation_ratio: None,
            avg_materialization_time_per_sample_ms: None,
            avg_evaluate_time_per_sample_ms: None,
            avg_submit_time_per_sample_ms: None,
            avg_idle_ratio: None,
        };
    }

    let mut total_sum = 0.0;
    let mut fetch_stall_sum = 0.0;
    let mut prefetch_hit_sum = 0.0;
    let mut queue_starvation_ratio_sum = 0.0;
    let mut materialization_sum = 0.0;
    let mut evaluate_sum = 0.0;
    let mut submit_sum = 0.0;
    let mut idle_sum = 0.0;
    for metrics in latest_by_worker.values() {
        total_sum += metrics.avg_time_per_sample_ms;
        fetch_stall_sum += metrics.avg_fetch_stall_time_per_sample_ms;
        prefetch_hit_sum += metrics.prefetch_hit_ratio;
        queue_starvation_ratio_sum += metrics.queue_starvation_ratio;
        materialization_sum += metrics.avg_materialization_time_per_sample_ms;
        evaluate_sum += metrics.avg_evaluate_time_per_sample_ms;
        submit_sum += metrics.avg_submit_time_per_sample_ms;
        idle_sum += metrics
            .idle_profile
            .as_ref()
            .map(|profile| profile.idle_ratio)
            .unwrap_or(0.0);
    }

    let count_f64 = count as f64;
    EvaluatorSummary {
        evaluator_count: count,
        avg_total_time_per_sample_ms: Some(total_sum / count_f64),
        avg_fetch_stall_time_per_sample_ms: Some(fetch_stall_sum / count_f64),
        avg_prefetch_hit_ratio: Some(prefetch_hit_sum / count_f64),
        avg_queue_starvation_ratio: Some(queue_starvation_ratio_sum / count_f64),
        avg_materialization_time_per_sample_ms: Some(materialization_sum / count_f64),
        avg_evaluate_time_per_sample_ms: Some(evaluate_sum / count_f64),
        avg_submit_time_per_sample_ms: Some(submit_sum / count_f64),
        avg_idle_ratio: Some(idle_sum / count_f64),
    }
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
    let mut panels = vec![
        tick_breakdown_panel(
            "sampler_tick_breakdown",
            sampler_tick_total_ms(&runtime),
            sampler_tick_segments(&runtime),
        ),
        key_value_panel(
            "sampler_runtime_overview",
            vec![
                key_value(
                    "memory_usage",
                    "Memory Usage",
                    entry.rss_bytes.map(format_bytes_human),
                ),
                key_value(
                    "completed_samples_per_second",
                    "Completed Samples Per Second",
                    runtime.completed_samples_per_second,
                ),
                key_value(
                    "produced_samples_total",
                    "Produced Samples Total",
                    runtime.produced_samples_total,
                ),
                key_value(
                    "ingested_samples_total",
                    "Ingested Samples Total",
                    runtime.ingested_samples_total,
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
                    "sampler_tick_ms",
                    "Sampler Tick Ms",
                    runtime.rolling.sampler_tick_ms.mean,
                ),
            ],
        ),
        key_value_panel(
            "sampler_runtime_efficiency",
            vec![
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
                    "runnable_queue_retained_ratio",
                    "Pending Queue Carryover Ratio",
                    runtime.rolling.runnable_queue_retained_ratio.mean,
                ),
                key_value(
                    "reclaim_ms",
                    "Reclaim (ms)",
                    runtime.rolling.reclaim_ms.mean,
                ),
                key_value(
                    "queue_snapshot_ms",
                    "Queue Snapshot (ms)",
                    runtime.rolling.queue_snapshot_ms.mean,
                ),
                key_value(
                    "active_evaluator_count_ms",
                    "Active Evaluator Count (ms)",
                    runtime.rolling.active_evaluator_count_ms.mean,
                ),
            ],
        ),
    ];
    if let Some(queue_buffer) = sampler_queue_buffer_panel(&entry.engine_diagnostics) {
        panels.push(queue_buffer);
    }
    panels
}

fn sampler_queue_buffer_panel(value: &JsonValue) -> Option<PanelState> {
    let runner = runner_diagnostics(value)?;
    let target_pending_batches = runner_value_as_i64(runner, "target_pending_batches");
    let pending_batches = runner_value_as_i64(runner, "pending_batches");
    let pending_shortfall = target_pending_batches
        .zip(pending_batches)
        .map(|(target, pending)| target.saturating_sub(pending));
    let entries = vec![
        runner_value_entry(runner, "queue_buffer", "Queue Buffer"),
        runner_value_entry(runner, "active_evaluator_count", "Active Evaluators"),
        runner_value_entry(runner, "target_pending_batches", "Target Pending Batches"),
        runner_value_entry(runner, "pending_batches", "Pending Batches"),
        Some(key_value(
            "pending_shortfall",
            "Pending Shortfall",
            pending_shortfall,
        )),
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

fn segment(key: &str, label: &str, value_ms: f64, color: &str) -> TickBreakdownSegment {
    TickBreakdownSegment {
        key: key.to_string(),
        label: label.to_string(),
        value_ms,
        color: color.to_string(),
    }
}

fn evaluator_tick_segments(metrics: &EvaluatorPerformanceMetrics) -> Vec<TickBreakdownSegment> {
    [
        segment(
            "fetch_decode",
            "Fetch+Decode",
            metrics.avg_fetch_time_per_sample_ms,
            "#0a9396",
        ),
        segment(
            "fetch_stall",
            "Fetch Stall",
            metrics.avg_fetch_stall_time_per_sample_ms,
            "#94d2bd",
        ),
        segment(
            "materialize",
            "Materialize",
            metrics.avg_materialization_time_per_sample_ms,
            "#ee9b00",
        ),
        segment(
            "evaluate",
            "Evaluate",
            metrics.avg_evaluate_time_per_sample_ms,
            "#ca6702",
        ),
        segment(
            "submit",
            "Submit",
            metrics.avg_submit_time_per_sample_ms,
            "#bb3e03",
        ),
        segment(
            "submit_stall",
            "Submit Stall",
            metrics.avg_submit_stall_time_per_sample_ms,
            "#ae2012",
        ),
    ]
    .into_iter()
    .filter(|segment| segment.value_ms.is_finite() && segment.value_ms > 0.0)
    .collect()
}

fn evaluator_tick_total_ms(metrics: &EvaluatorPerformanceMetrics) -> f64 {
    evaluator_tick_segments(metrics)
        .iter()
        .map(|segment| segment.value_ms)
        .sum()
}

fn sampler_tick_segments(runtime: &SamplerRuntimeMetrics) -> Vec<TickBreakdownSegment> {
    [
        (
            "reclaim",
            "Reclaim",
            runtime.rolling.reclaim_ms.mean.unwrap_or(0.0),
            "#0a9396",
        ),
        (
            "queue_snapshot",
            "Queue Snapshot",
            runtime.rolling.queue_snapshot_ms.mean.unwrap_or(0.0),
            "#94d2bd",
        ),
        (
            "active_evaluators",
            "Active Evaluators",
            runtime
                .rolling
                .active_evaluator_count_ms
                .mean
                .unwrap_or(0.0),
            "#e9d8a6",
        ),
        (
            "completed_fetch_wait",
            "Completed Fetch Wait",
            runtime.rolling.completed_fetch_wait_ms.mean.unwrap_or(0.0),
            "#ee9b00",
        ),
        (
            "completed_fetch_ingest",
            "Completed Process",
            runtime
                .rolling
                .completed_fetch_ingest_ms
                .mean
                .unwrap_or(0.0),
            "#ca6702",
        ),
        (
            "enqueue_drain_wait",
            "Enqueue Drain Wait",
            runtime.rolling.enqueue_drain_wait_ms.mean.unwrap_or(0.0),
            "#bb3e03",
        ),
        (
            "produce_enqueue",
            "Produce+Enqueue",
            runtime.rolling.produce_enqueue_ms.mean.unwrap_or(0.0),
            "#ae2012",
        ),
        (
            "progress_sync",
            "Progress Sync",
            runtime.rolling.progress_sync_ms.mean.unwrap_or(0.0),
            "#9b2226",
        ),
        (
            "performance_sync",
            "Performance Sync",
            runtime.rolling.performance_sync_ms.mean.unwrap_or(0.0),
            "#6a040f",
        ),
    ]
    .into_iter()
    .map(|(key, label, value_ms, color)| segment(key, label, value_ms, color))
    .filter(|segment| segment.value_ms.is_finite() && segment.value_ms > 0.0)
    .collect()
}

fn sampler_tick_total_ms(runtime: &SamplerRuntimeMetrics) -> f64 {
    sampler_tick_segments(runtime)
        .iter()
        .map(|segment| segment.value_ms)
        .sum()
}

fn format_bytes_human(bytes: i64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * KIB;
    const GIB: f64 = 1024.0 * MIB;

    let bytes_f64 = bytes as f64;
    if bytes_f64 >= GIB {
        format!("{:.2} GiB", bytes_f64 / GIB)
    } else if bytes_f64 >= MIB {
        format!("{:.1} MiB", bytes_f64 / MIB)
    } else if bytes_f64 >= KIB {
        format!("{:.1} KiB", bytes_f64 / KIB)
    } else {
        format!("{bytes} B")
    }
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

fn runner_value_as_i64(runner: &serde_json::Map<String, JsonValue>, key: &str) -> Option<i64> {
    runner.get(key)?.as_i64()
}
