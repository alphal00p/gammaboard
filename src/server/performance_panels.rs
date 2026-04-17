use crate::core::{EvaluatorPerformanceMetrics, SamplerRuntimeMetrics};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, PlotPoint,
    PlotSeries, TickBreakdownSegment, history_x, key_value, key_value_panel, key_value_with_tone,
    merge_panel_state, multi_timeseries_panel, panel_spec, replace_panel, tick_breakdown_panel,
    with_panel_width,
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
        |_entry| Vec::new(),
    );
    let (throughput_panel, latest_completed_samples_per_second) =
        sampler_completed_throughput_panel(&entries);
    if let Some(panel) = throughput_panel {
        response.updates.push(replace_panel(panel));
    }
    if let Some(panel) = sampler_utilization_history_panel(&entries) {
        response.updates.push(replace_panel(panel));
    }
    if let Some(latest) = entries.first() {
        for panel in sampler_current_panels(latest, latest_completed_samples_per_second) {
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
                "Evaluator Tick (Synchronous)",
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
                "sampler_priority_overview",
                "Sampler Priority Metrics",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_utilization_history",
                "Sampler Utilization History",
                PanelKind::MultiTimeseries,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_completed_samples_per_second",
                "Completed Samples / Sec",
                PanelKind::MultiTimeseries,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_tick_breakdown",
                "Sampler Tick (Synchronous)",
                PanelKind::TickBreakdown,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_runtime_efficiency",
                "Sampler Work Metrics",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "sampler_runtime_details",
                "Sampler Runner State",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "sampler_queue_state",
                "Queue Buffers",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "sampler_queue_efficiency",
                "Queue Hand-Off And DB",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
    ]
}

fn evaluator_current_panels(entry: &EvaluatorPerformanceHistoryEntry) -> Vec<PanelState> {
    let fetch_sync_ms = evaluator_fetch_sync_ms(&entry.metrics);
    let runner_sync_overhead_ms = evaluator_runner_sync_overhead_ms(&entry.metrics);
    let runner_wait_overhead_ms = evaluator_runner_wait_overhead_ms(&entry.metrics);
    let runner_total_overhead_ms = runner_sync_overhead_ms + runner_wait_overhead_ms;
    let pipeline_total_ms = evaluator_pipeline_total_ms(&entry.metrics);

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
                    "Avg Eval+Materialize Per Sample (us)",
                    ms_to_us(entry.metrics.avg_time_per_sample_ms),
                ),
                key_value(
                    "avg_pipeline_total_time_us",
                    "Avg Pipeline End-To-End Per Sample (us)",
                    ms_to_us(pipeline_total_ms),
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
                    "Fetch+Decode Total Per Sample (us)",
                    ms_to_us(entry.metrics.avg_fetch_time_per_sample_ms),
                ),
                key_value(
                    "avg_fetch_decode_sync_time_us",
                    "Fetch+Decode (sync) Per Sample (us)",
                    ms_to_us(fetch_sync_ms),
                ),
                key_value(
                    "avg_fetch_stall_time_us",
                    "Concurrent Fetch Wait Per Sample (us)",
                    ms_to_us(entry.metrics.avg_fetch_stall_time_per_sample_ms),
                ),
                key_value(
                    "avg_materialization_time_us",
                    "Materialization Per Sample (us)",
                    ms_to_us(entry.metrics.avg_materialization_time_per_sample_ms),
                ),
                key_value(
                    "avg_evaluate_time_us",
                    "Evaluate Engine Per Sample (us)",
                    ms_to_us(entry.metrics.avg_evaluate_time_per_sample_ms),
                ),
                key_value(
                    "std_evaluate_time_us",
                    "Evaluate Engine StdDev Per Sample (us)",
                    ms_to_us(entry.metrics.std_evaluate_time_per_sample_ms),
                ),
                key_value(
                    "avg_submit_time_us",
                    "Submit Per Sample (us)",
                    ms_to_us(entry.metrics.avg_submit_time_per_sample_ms),
                ),
                key_value(
                    "std_fetch_decode_time_us",
                    "Fetch+Decode StdDev Per Sample (us)",
                    ms_to_us(entry.metrics.std_fetch_time_per_sample_ms),
                ),
                key_value(
                    "std_materialization_time_us",
                    "Materialization StdDev Per Sample (us)",
                    ms_to_us(entry.metrics.std_materialization_time_per_sample_ms),
                ),
                key_value(
                    "std_submit_time_us",
                    "Submit StdDev Per Sample (us)",
                    ms_to_us(entry.metrics.std_submit_time_per_sample_ms),
                ),
                key_value(
                    "avg_submit_stall_time_us",
                    "Concurrent Submit Wait Per Sample (us)",
                    ms_to_us(entry.metrics.avg_submit_stall_time_per_sample_ms),
                ),
                key_value(
                    "submit_slot_hit_ratio",
                    "Submit Slot Hit Ratio",
                    entry.metrics.submit_slot_hit_ratio,
                ),
                key_value(
                    "avg_runner_sync_overhead_time_us",
                    "Runner Sync Overhead Per Sample (us)",
                    ms_to_us(runner_sync_overhead_ms),
                ),
                key_value(
                    "avg_runner_wait_overhead_time_us",
                    "Runner Wait Overhead Per Sample (us)",
                    ms_to_us(runner_wait_overhead_ms),
                ),
                key_value(
                    "avg_runner_total_overhead_time_us",
                    "Runner Total Overhead Per Sample (us)",
                    ms_to_us(runner_total_overhead_ms),
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
                "Avg Eval+Materialize Per Sample (us)",
                summary.avg_total_time_per_sample_ms.map(ms_to_us),
            ),
            key_value(
                "avg_pipeline_total_time_us",
                "Avg Pipeline End-To-End Per Sample (us)",
                summary.avg_pipeline_total_time_per_sample_ms.map(ms_to_us),
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
                "avg_runner_overhead_time_us",
                "Avg Runner Total Overhead Per Sample (us)",
                summary
                    .avg_runner_total_overhead_per_sample_ms
                    .map(ms_to_us),
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
            key_value(
                "avg_evaluator_utilization",
                "Avg Evaluator Utilization",
                summary.avg_evaluator_utilization,
            ),
        ],
    )
}

struct EvaluatorSummary {
    evaluator_count: usize,
    avg_total_time_per_sample_ms: Option<f64>,
    avg_pipeline_total_time_per_sample_ms: Option<f64>,
    avg_fetch_stall_time_per_sample_ms: Option<f64>,
    avg_prefetch_hit_ratio: Option<f64>,
    avg_queue_starvation_ratio: Option<f64>,
    avg_materialization_time_per_sample_ms: Option<f64>,
    avg_evaluate_time_per_sample_ms: Option<f64>,
    avg_submit_time_per_sample_ms: Option<f64>,
    avg_runner_total_overhead_per_sample_ms: Option<f64>,
    avg_evaluator_utilization: Option<f64>,
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
            avg_pipeline_total_time_per_sample_ms: None,
            avg_fetch_stall_time_per_sample_ms: None,
            avg_prefetch_hit_ratio: None,
            avg_queue_starvation_ratio: None,
            avg_materialization_time_per_sample_ms: None,
            avg_evaluate_time_per_sample_ms: None,
            avg_submit_time_per_sample_ms: None,
            avg_runner_total_overhead_per_sample_ms: None,
            avg_evaluator_utilization: None,
        };
    }

    let mut total_sum = 0.0;
    let mut pipeline_total_sum = 0.0;
    let mut fetch_stall_sum = 0.0;
    let mut prefetch_hit_sum = 0.0;
    let mut queue_starvation_ratio_sum = 0.0;
    let mut materialization_sum = 0.0;
    let mut evaluate_sum = 0.0;
    let mut submit_sum = 0.0;
    let mut runner_overhead_sum = 0.0;
    let mut utilization_sum = 0.0;
    let mut utilization_count = 0usize;
    for metrics in latest_by_worker.values() {
        total_sum += metrics.avg_time_per_sample_ms;
        pipeline_total_sum += evaluator_pipeline_total_ms(metrics);
        fetch_stall_sum += metrics.avg_fetch_stall_time_per_sample_ms;
        prefetch_hit_sum += metrics.prefetch_hit_ratio;
        queue_starvation_ratio_sum += metrics.queue_starvation_ratio;
        materialization_sum += metrics.avg_materialization_time_per_sample_ms;
        evaluate_sum += metrics.avg_evaluate_time_per_sample_ms;
        submit_sum += metrics.avg_submit_time_per_sample_ms;
        runner_overhead_sum += evaluator_runner_total_overhead_ms(metrics);
        if let Some(idle_ratio) = metrics
            .idle_profile
            .as_ref()
            .map(|profile| profile.idle_ratio)
        {
            utilization_sum += (1.0 - idle_ratio).clamp(0.0, 1.0);
            utilization_count += 1;
        }
    }

    let count_f64 = count as f64;
    EvaluatorSummary {
        evaluator_count: count,
        avg_total_time_per_sample_ms: Some(total_sum / count_f64),
        avg_pipeline_total_time_per_sample_ms: Some(pipeline_total_sum / count_f64),
        avg_fetch_stall_time_per_sample_ms: Some(fetch_stall_sum / count_f64),
        avg_prefetch_hit_ratio: Some(prefetch_hit_sum / count_f64),
        avg_queue_starvation_ratio: Some(queue_starvation_ratio_sum / count_f64),
        avg_materialization_time_per_sample_ms: Some(materialization_sum / count_f64),
        avg_evaluate_time_per_sample_ms: Some(evaluate_sum / count_f64),
        avg_submit_time_per_sample_ms: Some(submit_sum / count_f64),
        avg_runner_total_overhead_per_sample_ms: Some(runner_overhead_sum / count_f64),
        avg_evaluator_utilization: (utilization_count > 0)
            .then_some(utilization_sum / utilization_count as f64),
    }
}

fn sampler_current_panels(
    entry: &SamplerPerformanceHistoryEntry,
    completed_samples_per_second: f64,
) -> Vec<PanelState> {
    let Some(runtime) = decode_sampler_runtime_metrics(entry) else {
        return Vec::new();
    };

    let target_pending_batches =
        queue_buffer_value(&entry.engine_diagnostics, "target_pending_batches");
    let target_batch_eval_ms =
        queue_buffer_value(&entry.engine_diagnostics, "target_batch_eval_ms");
    let pending_shortfall = match (
        target_pending_batches.as_ref(),
        runtime.queue.db_pending_batches,
    ) {
        (Some(target), Some(pending)) => {
            target.as_i64().map(|target| target.saturating_sub(pending))
        }
        _ => None,
    };
    let total_memory_bytes = match (entry.rss_bytes, runtime.total_evaluator_rss_bytes) {
        (Some(sampler), Some(evaluator)) => Some(sampler.saturating_add(evaluator)),
        (Some(sampler), None) => Some(sampler),
        (None, Some(evaluator)) => Some(evaluator),
        (None, None) => None,
    };
    let evaluator_busy_ratio = runtime
        .avg_evaluator_utilization
        .map(|value| value.clamp(0.0, 1.0));
    let evaluator_busy_tone = evaluator_busy_ratio.and_then(evaluator_busy_ratio_tone);

    vec![
        key_value_panel(
            "sampler_priority_overview",
            vec![
                key_value(
                    "completed_samples_per_second",
                    "Completed Samples / Sec",
                    completed_samples_per_second,
                ),
                key_value(
                    "total_memory_usage",
                    "Total Memory Usage (Sampler + Evaluators)",
                    total_memory_bytes.map(format_bytes_human),
                ),
                key_value(
                    "sampler_memory_usage",
                    "Sampler Memory Usage",
                    entry.rss_bytes.map(format_bytes_human),
                ),
                key_value(
                    "avg_evaluator_memory",
                    "Avg Evaluator Memory Usage",
                    runtime.avg_evaluator_rss_bytes.map(format_bytes_human),
                ),
                key_value(
                    "active_evaluators",
                    "Active Evaluators",
                    runtime.active_evaluator_count,
                ),
                key_value_with_tone(
                    "evaluator_busy_ratio",
                    "Evaluator Busy Ratio",
                    evaluator_busy_ratio.map(|value| format!("{:.1}%", value * 100.0)),
                    evaluator_busy_tone,
                ),
                key_value(
                    "eval_ms_per_batch",
                    "Eval Ms / Batch",
                    runtime.sampler.eval_ms_per_batch.mean,
                ),
                key_value(
                    "target_batch_eval_ms",
                    "Target Eval Ms / Batch",
                    target_batch_eval_ms,
                ),
                key_value(
                    "eval_us_per_sample",
                    "Eval Us / Sample",
                    runtime.sampler.eval_ms_per_sample.mean.map(ms_to_us),
                ),
                key_value(
                    "batch_size_current",
                    "Current Batch Size",
                    runtime.batch_size_current,
                ),
            ],
        ),
        tick_breakdown_panel(
            "sampler_tick_breakdown",
            sampler_tick_total_ms(&runtime),
            sampler_tick_segments(&runtime),
        ),
        key_value_panel(
            "sampler_runtime_details",
            vec![
                key_value(
                    "sampler_tick_busy_ratio",
                    "Sampler Tick Busy Ratio",
                    runtime.sampler_tick_busy_ratio,
                ),
                key_value(
                    "sampler_uptime_seconds",
                    "Sampler Uptime (s)",
                    (runtime.sampler_uptime_ms / 1000.0).max(0.0),
                ),
                key_value(
                    "insert_task_utilization",
                    "Insert Task Utilization",
                    runtime.queue.insert_task_utilization,
                ),
                key_value(
                    "completed_fetch_utilization",
                    "Completed Fetch Utilization",
                    runtime.queue.completed_fetch_utilization,
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
                    "completed_samples_total",
                    "Completed Samples Total",
                    runtime.completed_samples_total,
                ),
                key_value(
                    "batch_size_current",
                    "Batch Size Current",
                    runtime.batch_size_current,
                ),
            ],
        ),
        key_value_panel(
            "sampler_runtime_efficiency",
            vec![
                key_value(
                    "produce_events",
                    "Produce Events",
                    runtime.sampler.produce_ms_per_sample.count,
                ),
                key_value(
                    "training_ingest_batches",
                    "Training Ingest Batches",
                    runtime.sampler.training_ingest_ms_per_sample.count,
                ),
                key_value(
                    "training_ingest_passes",
                    "Training Ingest Passes",
                    runtime.sampler.completed_training_ingest_ms.count,
                ),
                key_value(
                    "merge_passes",
                    "Merge Passes",
                    runtime.sampler.completed_merge_ingest_ms.count,
                ),
                key_value(
                    "frontend_sync_flushes",
                    "Frontend Sync Flushes",
                    runtime.sampler.persist_accumulator_ms.count,
                ),
                key_value(
                    "cleanup_passes",
                    "Queue Cleanup Passes",
                    runtime.sampler.completed_delete_ms.count,
                ),
                key_value(
                    "reclaim_passes",
                    "Reclaim Passes",
                    runtime.sampler.reclaim_ms.count,
                ),
                key_value(
                    "training_ingest_ms_per_sample",
                    "Avg Training Ingest Ms / Sample",
                    window_mean_value(
                        &runtime.sampler.training_ingest_ms_per_sample,
                        "no training ingest",
                    ),
                ),
                key_value(
                    "produce_ms_per_sample",
                    "Avg Produce Ms / Sample",
                    window_mean_value(&runtime.sampler.produce_ms_per_sample, "no batches"),
                ),
                key_value(
                    "completed_training_ingest_ms",
                    "Avg Ingest Training Weights Ms",
                    window_mean_value(
                        &runtime.sampler.completed_training_ingest_ms,
                        "no training ingest",
                    ),
                ),
                key_value(
                    "merge_completed_batches_ms",
                    "Avg Merge Completed Accumulators Ms",
                    window_mean_value(
                        &runtime.sampler.completed_merge_ingest_ms,
                        "no completed merges",
                    ),
                ),
                key_value(
                    "persist_accumulator_ms",
                    "Avg Persist Accumulator Ms",
                    window_mean_value(&runtime.sampler.persist_accumulator_ms, "no frontend sync"),
                ),
                key_value(
                    "completed_delete_ms",
                    "Avg Queue Cleanup Ms",
                    window_mean_value(&runtime.sampler.completed_delete_ms, "no cleanup"),
                ),
                key_value(
                    "reclaim_ms",
                    "Avg Reclaim Abandoned Batches Ms",
                    window_mean_value(&runtime.sampler.reclaim_ms, "no reclaim"),
                ),
            ],
        ),
        key_value_panel(
            "sampler_queue_state",
            vec![
                key_value(
                    "db_pending_batches",
                    "DB Pending Batches",
                    runtime.queue.db_pending_batches,
                ),
                key_value(
                    "local_pending_batches",
                    "Local Pending Batches",
                    runtime.queue.local_pending_batches,
                ),
                key_value(
                    "local_inflight_insert_batches",
                    "Local In-Flight Insert Batches",
                    runtime.queue.local_inflight_insert_batches,
                ),
                key_value(
                    "local_inflight_insert_tasks",
                    "Local In-Flight Insert Tasks",
                    runtime.queue.local_inflight_insert_tasks,
                ),
                key_value(
                    "completed_prefetch_buffer",
                    "Completed Prefetch Buffer",
                    runtime.queue.local_ready_processed_batches,
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
                    "queue_buffer",
                    "Target Pending Batches / Evaluator",
                    queue_buffer_value(&entry.engine_diagnostics, "queue_buffer"),
                ),
            ],
        ),
        key_value_panel(
            "sampler_queue_efficiency",
            vec![
                key_value(
                    "completed_fetches",
                    "Completed Fetches",
                    runtime.queue.rolling.fetch_completed_ms.count,
                ),
                key_value(
                    "completed_batches_fetched",
                    "Completed Batches Fetched",
                    window_total_value(&runtime.queue.rolling.fetch_completed_batches),
                ),
                key_value(
                    "completed_batch_fetch_ms",
                    "Avg Completed Batch Fetch Ms",
                    window_mean_value(&runtime.queue.rolling.fetch_completed_ms, "no fetches"),
                ),
                key_value(
                    "fetch_completed_batches",
                    "Avg Completed Batches / Fetch",
                    window_mean_value(&runtime.queue.rolling.fetch_completed_batches, "no fetches"),
                ),
                key_value(
                    "fetch_completed_prefetch_fill_ratio",
                    "Avg Completed Prefetch Fill Ratio",
                    window_mean_value(
                        &runtime.queue.rolling.fetch_completed_prefetch_fill_ratio,
                        "no fetches",
                    ),
                ),
                key_value(
                    "inserted_bundles",
                    "Inserted Bundles",
                    runtime.queue.rolling.insert_bundle_ms.count,
                ),
                key_value(
                    "inserted_batches",
                    "Inserted Batches",
                    window_total_value(&runtime.queue.rolling.insert_bundle_batches),
                ),
                key_value(
                    "inserted_payload_bytes",
                    "Inserted Payload Bytes",
                    window_total_value(&runtime.queue.rolling.insert_bundle_payload_bytes),
                ),
                key_value(
                    "insert_bundle_ms",
                    "Avg Insert Bundle Ms",
                    window_mean_value(&runtime.queue.rolling.insert_bundle_ms, "no inserts"),
                ),
                key_value(
                    "insert_bundle_ms_per_batch",
                    "Avg Insert Ms / Batch",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_ms_per_batch,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_local_pending_at_start",
                    "Avg Local Pending At Insert Start",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_local_pending_at_start,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_db_pending_at_start",
                    "Avg DB Pending At Insert Start",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_db_pending_at_start,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_serialize_ms",
                    "Avg Insert Serialize Ms",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_serialize_ms,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_payload_bytes_per_batch",
                    "Avg Insert Payload Bytes / Batch",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_payload_bytes_per_batch,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_db_batches_ms",
                    "Avg Insert Batches SQL Ms",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_db_batches_ms,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_db_inputs_ms",
                    "Avg Insert Inputs SQL Ms",
                    window_mean_value(
                        &runtime.queue.rolling.insert_bundle_db_inputs_ms,
                        "no inserts",
                    ),
                ),
                key_value(
                    "insert_bundle_commit_ms",
                    "Avg Insert Commit Ms",
                    window_mean_value(&runtime.queue.rolling.insert_bundle_commit_ms, "no inserts"),
                ),
            ],
        ),
    ]
}

fn sampler_completed_throughput_panel(
    entries: &[SamplerPerformanceHistoryEntry],
) -> (Option<PanelState>, f64) {
    let mut samples = entries
        .iter()
        .filter_map(|entry| {
            let runtime = decode_sampler_runtime_metrics(entry)?;
            let cumulative_samples = if runtime.completed_samples_total > 0 {
                runtime.completed_samples_total
            } else {
                runtime.ingested_samples_total
            };
            Some(SamplerProgressPoint {
                x_wall_time_ms: history_x(entry.created_at),
                x_sampler_uptime_ms: sampler_uptime_ms(&runtime),
                x_completed_samples_total: cumulative_samples as f64,
                cumulative_samples,
            })
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return (None, 0.0);
    }
    samples.sort_by(|left, right| left.x_wall_time_ms.total_cmp(&right.x_wall_time_ms));
    let instant_points = instant_throughput_points_from_cumulative(&samples);
    let latest_instant = instant_points.last().map(|point| point.y).unwrap_or(0.0);
    (
        Some(multi_timeseries_panel(
            "sampler_completed_samples_per_second",
            vec![PlotSeries {
                id: "completed_samples_per_second".to_string(),
                label: "Completed Samples / Sec".to_string(),
                color: Some("#2563eb".to_string()),
                smooth: Some(true),
                points: instant_points,
            }],
        )),
        latest_instant,
    )
}

fn sampler_utilization_history_panel(
    entries: &[SamplerPerformanceHistoryEntry],
) -> Option<PanelState> {
    let mut sampler_tick_points = Vec::new();
    let mut insert_task_points = Vec::new();
    let mut completed_fetch_points = Vec::new();
    let mut evaluator_utilization_points = Vec::new();

    for entry in entries.iter().rev() {
        let runtime = decode_sampler_runtime_metrics(entry)?;
        let x_wall_time_ms = history_x(entry.created_at);
        let x_sampler_uptime_ms = sampler_uptime_ms(&runtime);
        let x_completed_samples_total = Some(runtime.completed_samples_total as f64);
        sampler_tick_points.push(PlotPoint {
            x: x_wall_time_ms,
            y: runtime.sampler_tick_busy_ratio.unwrap_or(0.0),
            x_sampler_uptime_ms,
            x_completed_samples_total,
            y_min: None,
            y_max: None,
        });
        insert_task_points.push(PlotPoint {
            x: x_wall_time_ms,
            y: runtime.queue.insert_task_utilization.unwrap_or(0.0),
            x_sampler_uptime_ms,
            x_completed_samples_total,
            y_min: None,
            y_max: None,
        });
        completed_fetch_points.push(PlotPoint {
            x: x_wall_time_ms,
            y: runtime.queue.completed_fetch_utilization.unwrap_or(0.0),
            x_sampler_uptime_ms,
            x_completed_samples_total,
            y_min: None,
            y_max: None,
        });
        if let Some(utilization) = runtime.avg_evaluator_utilization {
            evaluator_utilization_points.push(PlotPoint {
                x: x_wall_time_ms,
                y: utilization.clamp(0.0, 1.0),
                x_sampler_uptime_ms,
                x_completed_samples_total,
                y_min: None,
                y_max: None,
            });
        }
    }

    Some(multi_timeseries_panel(
        "sampler_utilization_history",
        vec![
            PlotSeries {
                id: "sampler_tick_busy_ratio".to_string(),
                label: "Sampler Tick Busy Ratio".to_string(),
                color: Some("#2563eb".to_string()),
                smooth: Some(true),
                points: sampler_tick_points,
            },
            PlotSeries {
                id: "insert_task_utilization".to_string(),
                label: "Insert Task Utilization".to_string(),
                color: Some("#ea580c".to_string()),
                smooth: Some(true),
                points: insert_task_points,
            },
            PlotSeries {
                id: "completed_fetch_utilization".to_string(),
                label: "Completed Fetch Utilization".to_string(),
                color: Some("#16a34a".to_string()),
                smooth: Some(true),
                points: completed_fetch_points,
            },
            PlotSeries {
                id: "avg_evaluator_utilization".to_string(),
                label: "Avg Evaluator Utilization".to_string(),
                color: Some("#7c3aed".to_string()),
                smooth: Some(true),
                points: evaluator_utilization_points,
            },
        ],
    ))
}

#[derive(Debug, Clone, Copy)]
struct SamplerProgressPoint {
    x_wall_time_ms: f64,
    x_sampler_uptime_ms: Option<f64>,
    x_completed_samples_total: f64,
    cumulative_samples: i64,
}

fn instant_throughput_points_from_cumulative(samples: &[SamplerProgressPoint]) -> Vec<PlotPoint> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mut points = Vec::with_capacity(samples.len());
    for (index, point) in samples.iter().enumerate() {
        let y = if index == 0 {
            0.0
        } else {
            let previous = samples[index - 1];
            let elapsed_ms = match (point.x_sampler_uptime_ms, previous.x_sampler_uptime_ms) {
                (Some(current), Some(prev)) if current > prev => current - prev,
                _ => point.x_wall_time_ms - previous.x_wall_time_ms,
            };
            let elapsed_secs = elapsed_ms / 1000.0;
            let delta_samples = point
                .cumulative_samples
                .saturating_sub(previous.cumulative_samples);
            if elapsed_secs > 0.0 {
                (delta_samples as f64 / elapsed_secs).max(0.0)
            } else {
                0.0
            }
        };
        points.push(PlotPoint {
            x: point.x_wall_time_ms,
            y,
            x_sampler_uptime_ms: point.x_sampler_uptime_ms,
            x_completed_samples_total: Some(point.x_completed_samples_total),
            y_min: None,
            y_max: None,
        });
    }
    points
}

fn sampler_uptime_ms(runtime: &SamplerRuntimeMetrics) -> Option<f64> {
    (runtime.sampler_uptime_ms.is_finite() && runtime.sampler_uptime_ms >= 0.0)
        .then_some(runtime.sampler_uptime_ms)
}

fn queue_buffer_value(value: &JsonValue, key: &str) -> Option<JsonValue> {
    value.get("runner")?.get(key).cloned()
}

fn evaluator_busy_ratio_tone(utilization: f64) -> Option<&'static str> {
    if !utilization.is_finite() {
        return None;
    }
    if utilization < 0.5 {
        Some("critical")
    } else if utilization < 0.75 {
        Some("warning")
    } else {
        Some("good")
    }
}

fn decode_sampler_runtime_metrics(
    entry: &SamplerPerformanceHistoryEntry,
) -> Option<SamplerRuntimeMetrics> {
    serde_json::from_value(entry.runtime_metrics.clone()).ok()
}

fn window_mean_value(
    snapshot: &crate::core::RollingMetricSnapshot,
    empty_label: &str,
) -> JsonValue {
    if snapshot.count == 0 {
        JsonValue::String(empty_label.to_string())
    } else {
        serde_json::to_value(snapshot.mean).unwrap_or(JsonValue::Null)
    }
}

fn window_total_value(snapshot: &crate::core::RollingMetricSnapshot) -> JsonValue {
    JsonValue::from(snapshot.total.unwrap_or(0.0))
}

fn ms_to_us(value_ms: f64) -> f64 {
    value_ms * 1000.0
}

fn evaluator_fetch_sync_ms(metrics: &EvaluatorPerformanceMetrics) -> f64 {
    (metrics.avg_fetch_time_per_sample_ms - metrics.avg_fetch_stall_time_per_sample_ms).max(0.0)
}

fn evaluator_runner_sync_overhead_ms(metrics: &EvaluatorPerformanceMetrics) -> f64 {
    evaluator_fetch_sync_ms(metrics)
        + metrics.avg_materialization_time_per_sample_ms.max(0.0)
        + metrics.avg_submit_time_per_sample_ms.max(0.0)
}

fn evaluator_runner_wait_overhead_ms(metrics: &EvaluatorPerformanceMetrics) -> f64 {
    metrics.avg_fetch_stall_time_per_sample_ms.max(0.0)
        + metrics.avg_submit_stall_time_per_sample_ms.max(0.0)
}

fn evaluator_runner_total_overhead_ms(metrics: &EvaluatorPerformanceMetrics) -> f64 {
    evaluator_runner_sync_overhead_ms(metrics) + evaluator_runner_wait_overhead_ms(metrics)
}

fn evaluator_pipeline_total_ms(metrics: &EvaluatorPerformanceMetrics) -> f64 {
    metrics.avg_evaluate_time_per_sample_ms.max(0.0) + evaluator_runner_total_overhead_ms(metrics)
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
    let fetch_sync_ms = evaluator_fetch_sync_ms(metrics);
    [
        segment(
            "fetch_decode",
            "Fetch+Decode (sync)",
            fetch_sync_ms,
            "#0a9396",
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
            "Submit (sync)",
            metrics.avg_submit_time_per_sample_ms,
            "#bb3e03",
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
            "completed_training_ingest",
            "Ingest Training Weights (sync)",
            runtime
                .sampler
                .completed_training_ingest_ms
                .mean
                .unwrap_or(0.0),
            "#6d597a",
        ),
        (
            "completed_merge",
            "Merge Completed Accumulators (sync)",
            runtime
                .sampler
                .completed_merge_ingest_ms
                .mean
                .unwrap_or(0.0),
            "#ca6702",
        ),
        (
            "produce",
            "Produce Batches (sync)",
            runtime.sampler.produce_ms.mean.unwrap_or(0.0),
            "#ae2012",
        ),
        (
            "progress_sync",
            "Write Progress (sync)",
            runtime.sampler.progress_sync_ms.mean.unwrap_or(0.0),
            "#9b2226",
        ),
        (
            "performance_sync",
            "Write Performance Snapshot (sync)",
            runtime.sampler.performance_sync_ms.mean.unwrap_or(0.0),
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
