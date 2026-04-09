use crate::core::{EvaluatorPerformanceMetrics, SamplerRuntimeMetrics};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, PlotPoint,
    PlotSeries, TickBreakdownSegment, history_x, key_value, key_value_panel, merge_panel_state,
    multi_timeseries_panel, panel_spec, replace_panel, scalar_timeseries_panel,
    tick_breakdown_panel, with_panel_width,
};
use crate::stores::{EvaluatorPerformanceHistoryEntry, SamplerPerformanceHistoryEntry};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

const COMPLETED_THROUGHPUT_WINDOW_MS: f64 = 5_000.0;

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
    evaluator_entries: Option<Vec<EvaluatorPerformanceHistoryEntry>>,
) -> PanelResponse {
    let mut response = build_performance_response(
        scope_id.unwrap_or_else(|| "sampler".to_string()),
        entries.clone(),
        sampler_panel_specs(),
        |entry| entry.id.to_string(),
        |_entry| Vec::new(),
    );
    let throughput_panel = sampler_completed_throughput_panel(&entries);
    let latest_completed_samples_per_second = throughput_panel
        .as_ref()
        .and_then(|panel| match panel {
            PanelState::ScalarTimeseries { points, .. } => points.last().map(|point| point.y),
            _ => None,
        })
        .unwrap_or(0.0);
    if let Some(panel) = throughput_panel {
        response.updates.push(replace_panel(panel));
    }
    if let Some(panel) = sampler_utilization_history_panel(&entries, evaluator_entries.as_deref()) {
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
                "sampler_runtime_overview",
                "Sampler Overview",
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
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
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
                "Sampler Efficiency",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "sampler_queue_state",
                "Queue State",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
        ),
        with_panel_width(
            panel_spec(
                "sampler_queue_efficiency",
                "Queue Concurrent Work",
                PanelKind::KeyValue,
                PanelHistoryMode::Replace,
            ),
            PanelWidth::Half,
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
                    "Concurrent Submit Wait Per Sample (us)",
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

fn sampler_current_panels(
    entry: &SamplerPerformanceHistoryEntry,
    completed_samples_per_second: f64,
) -> Vec<PanelState> {
    let Some(runtime) = decode_sampler_runtime_metrics(entry) else {
        return Vec::new();
    };

    let target_pending_batches =
        queue_buffer_value(&entry.engine_diagnostics, "target_pending_batches");
    let target_local_pending_batches =
        queue_buffer_value(&entry.engine_diagnostics, "target_local_pending_batches");
    let pending_batches = queue_buffer_value(&entry.engine_diagnostics, "pending_batches");
    let claimed_batches = queue_buffer_value(&entry.engine_diagnostics, "claimed_batches");
    let completed_batches = queue_buffer_value(&entry.engine_diagnostics, "completed_batches");
    let open_batches = queue_buffer_value(&entry.engine_diagnostics, "open_batches");
    let pending_shortfall = match (target_pending_batches.as_ref(), pending_batches.as_ref()) {
        (Some(target), Some(pending)) => target
            .as_i64()
            .zip(pending.as_i64())
            .map(|(t, p)| t.saturating_sub(p)),
        _ => None,
    };

    vec![
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
                    "Completed Samples / Sec",
                    completed_samples_per_second,
                ),
                key_value(
                    "sampler_tick_busy_ratio",
                    "Sampler Tick Busy Ratio",
                    busy_ratio(runtime.sampler.tick_idle_ratio.mean),
                ),
                key_value(
                    "insert_task_utilization",
                    "Insert Task Utilization",
                    busy_ratio(runtime.queue.rolling.insert_task_idle_ratio.mean),
                ),
                key_value(
                    "completed_fetch_utilization",
                    "Completed Fetch Utilization",
                    busy_ratio(runtime.queue.rolling.fetch_completed_idle_ratio.mean),
                ),
                key_value(
                    "eval_ms_per_batch",
                    "Eval Ms / Batch",
                    runtime.sampler.eval_ms_per_batch.mean,
                ),
                key_value(
                    "target_batch_eval_ms",
                    "Target Eval Ms / Batch",
                    queue_buffer_value(&entry.engine_diagnostics, "target_batch_eval_ms"),
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
                key_value(
                    "eval_ms_per_sample",
                    "Eval Ms / Sample",
                    runtime.sampler.eval_ms_per_sample.mean,
                ),
            ],
        ),
        key_value_panel(
            "sampler_runtime_efficiency",
            vec![
                key_value(
                    "training_ingest_ms_per_sample",
                    "Training Ingest Ms / Sample",
                    runtime.sampler.training_ingest_ms_per_sample.mean,
                ),
                key_value(
                    "produce_ms_per_sample",
                    "Produce Ms / Sample",
                    runtime.sampler.produce_ms_per_sample.mean,
                ),
                key_value(
                    "merge_completed_batches_ms",
                    "Merge Completed Batches",
                    runtime.sampler.completed_merge_ingest_ms.mean,
                ),
                key_value(
                    "persist_observable_ms",
                    "Persist Observable (frontend sync)",
                    runtime.sampler.persist_observable_ms.mean,
                ),
                key_value(
                    "completed_delete_ms",
                    "Cleanup Consumed Batches",
                    runtime.sampler.completed_delete_ms.mean,
                ),
                key_value(
                    "reclaim_ms",
                    "Reclaim Abandoned Batches",
                    runtime.sampler.reclaim_ms.mean,
                ),
            ],
        ),
        key_value_panel(
            "sampler_queue_state",
            vec![
                key_value(
                    "queue_buffer",
                    "Queue Buffer",
                    queue_buffer_value(&entry.engine_diagnostics, "queue_buffer"),
                ),
                key_value(
                    "local_pending_buffer_multiplier",
                    "Local Pending Buffer Multiplier",
                    queue_buffer_value(
                        &entry.engine_diagnostics,
                        "local_pending_buffer_multiplier",
                    ),
                ),
                key_value(
                    "target_pending_batches",
                    "Target Pending Batches",
                    target_pending_batches,
                ),
                key_value(
                    "target_local_pending_batches",
                    "Target Local Pending Batches",
                    target_local_pending_batches,
                ),
                key_value("pending_batches", "Pending Batches", pending_batches),
                key_value("claimed_batches", "Claimed Batches", claimed_batches),
                key_value("completed_batches", "Completed Batches", completed_batches),
                key_value("open_batches", "Open Batches", open_batches),
                key_value("pending_shortfall", "Pending Shortfall", pending_shortfall),
                key_value(
                    "local_pending_batches",
                    "Local Pending Batches",
                    queue_buffer_value(&entry.engine_diagnostics, "local_pending_batches"),
                ),
                key_value(
                    "local_inflight_insert_tasks",
                    "Local Inflight Insert Tasks",
                    queue_buffer_value(&entry.engine_diagnostics, "local_inflight_insert_tasks"),
                ),
                key_value(
                    "local_inflight_insert_batches",
                    "Local Inflight Insert Batches",
                    queue_buffer_value(&entry.engine_diagnostics, "local_inflight_insert_batches"),
                ),
                key_value(
                    "local_ready_processed_batches",
                    "Local Ready Processed Batches",
                    queue_buffer_value(&entry.engine_diagnostics, "local_ready_processed_batches"),
                ),
            ],
        ),
        key_value_panel(
            "sampler_queue_efficiency",
            vec![
                key_value(
                    "completed_batch_fetch_ms",
                    "Completed Batch Fetch (concurrent)",
                    runtime.queue.rolling.fetch_completed_ms.mean,
                ),
                key_value(
                    "completed_batch_fetch_idle_ratio",
                    "Completed Batch Fetch Idle Ratio",
                    runtime.queue.rolling.fetch_completed_idle_ratio.mean,
                ),
                key_value(
                    "insert_bundle_ms",
                    "Insert Bundle (concurrent)",
                    runtime.queue.rolling.insert_bundle_ms.mean,
                ),
                key_value(
                    "insert_bundle_ms_per_batch",
                    "Insert / Batch (concurrent)",
                    runtime.queue.rolling.insert_bundle_ms_per_batch.mean,
                ),
                key_value(
                    "insert_task_idle_ratio",
                    "Insert Task Idle Ratio",
                    runtime.queue.rolling.insert_task_idle_ratio.mean,
                ),
                key_value(
                    "insert_bundle_serialize_ms",
                    "Insert Serialize",
                    runtime.queue.rolling.insert_bundle_serialize_ms.mean,
                ),
                key_value(
                    "insert_bundle_payload_bytes",
                    "Insert Payload Bytes",
                    runtime.queue.rolling.insert_bundle_payload_bytes.mean,
                ),
                key_value(
                    "insert_bundle_payload_bytes_per_batch",
                    "Insert Payload Bytes / Batch",
                    runtime
                        .queue
                        .rolling
                        .insert_bundle_payload_bytes_per_batch
                        .mean,
                ),
                key_value(
                    "insert_bundle_db_batches_ms",
                    "Insert Batches SQL",
                    runtime.queue.rolling.insert_bundle_db_batches_ms.mean,
                ),
                key_value(
                    "insert_bundle_db_inputs_ms",
                    "Insert Inputs SQL",
                    runtime.queue.rolling.insert_bundle_db_inputs_ms.mean,
                ),
                key_value(
                    "insert_bundle_commit_ms",
                    "Insert Commit",
                    runtime.queue.rolling.insert_bundle_commit_ms.mean,
                ),
                key_value(
                    "insert_bundle_local_pending_at_start",
                    "Local Pending at Insert Start",
                    runtime
                        .queue
                        .rolling
                        .insert_bundle_local_pending_at_start
                        .mean,
                ),
            ],
        ),
    ]
}

fn sampler_completed_throughput_panel(
    entries: &[SamplerPerformanceHistoryEntry],
) -> Option<PanelState> {
    let mut samples = entries
        .iter()
        .filter_map(|entry| {
            let runtime = decode_sampler_runtime_metrics(entry)?;
            let cumulative_samples = if runtime.completed_samples_total > 0 {
                runtime.completed_samples_total
            } else {
                runtime.ingested_samples_total
            };
            Some((history_x(entry.created_at), cumulative_samples))
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|left, right| left.0.total_cmp(&right.0));
    let points = throughput_points_from_cumulative(&samples, COMPLETED_THROUGHPUT_WINDOW_MS);
    Some(scalar_timeseries_panel(
        "sampler_completed_samples_per_second",
        points,
    ))
}

fn sampler_utilization_history_panel(
    entries: &[SamplerPerformanceHistoryEntry],
    evaluator_entries: Option<&[EvaluatorPerformanceHistoryEntry]>,
) -> Option<PanelState> {
    let mut sampler_tick_points = Vec::new();
    let mut insert_task_points = Vec::new();
    let mut completed_fetch_points = Vec::new();

    for entry in entries.iter().rev() {
        let runtime = decode_sampler_runtime_metrics(entry)?;
        let x = history_x(entry.created_at);
        sampler_tick_points.push(PlotPoint {
            x,
            y: busy_ratio(runtime.sampler.tick_idle_ratio.mean).unwrap_or(0.0),
            y_min: None,
            y_max: None,
        });
        insert_task_points.push(PlotPoint {
            x,
            y: busy_ratio(runtime.queue.rolling.insert_task_idle_ratio.mean).unwrap_or(0.0),
            y_min: None,
            y_max: None,
        });
        completed_fetch_points.push(PlotPoint {
            x,
            y: busy_ratio(runtime.queue.rolling.fetch_completed_idle_ratio.mean).unwrap_or(0.0),
            y_min: None,
            y_max: None,
        });
    }

    let evaluator_busy_points = evaluator_entries
        .map(aggregate_evaluator_busy_points)
        .unwrap_or_default();

    Some(multi_timeseries_panel(
        "sampler_utilization_history",
        vec![
            PlotSeries {
                id: "sampler_tick_busy_ratio".to_string(),
                label: "Sampler Tick Busy Ratio".to_string(),
                color: Some("#2563eb".to_string()),
                points: sampler_tick_points,
            },
            PlotSeries {
                id: "insert_task_utilization".to_string(),
                label: "Insert Task Utilization".to_string(),
                color: Some("#ea580c".to_string()),
                points: insert_task_points,
            },
            PlotSeries {
                id: "completed_fetch_utilization".to_string(),
                label: "Completed Fetch Utilization".to_string(),
                color: Some("#16a34a".to_string()),
                points: completed_fetch_points,
            },
            PlotSeries {
                id: "avg_evaluator_utilization".to_string(),
                label: "Avg Evaluator Utilization".to_string(),
                color: Some("#7c3aed".to_string()),
                points: evaluator_busy_points,
            },
        ],
    ))
}

fn aggregate_evaluator_busy_points(entries: &[EvaluatorPerformanceHistoryEntry]) -> Vec<PlotPoint> {
    const BUCKET_MS: f64 = 1000.0;

    let mut by_bucket = BTreeMap::<i64, (f64, usize)>::new();
    for entry in entries {
        let Some(idle_ratio) = entry
            .metrics
            .idle_profile
            .as_ref()
            .map(|profile| profile.idle_ratio)
        else {
            continue;
        };
        let x = history_x(entry.created_at);
        let bucket = (x / BUCKET_MS).round() as i64;
        let busy = (1.0 - idle_ratio).clamp(0.0, 1.0);
        let aggregate = by_bucket.entry(bucket).or_insert((0.0, 0));
        aggregate.0 += busy;
        aggregate.1 += 1;
    }

    by_bucket
        .into_iter()
        .filter_map(|(bucket, (sum, count))| {
            (count > 0).then_some(PlotPoint {
                x: bucket as f64 * BUCKET_MS,
                y: sum / count as f64,
                y_min: None,
                y_max: None,
            })
        })
        .collect()
}

fn throughput_points_from_cumulative(samples: &[(f64, i64)], window_ms: f64) -> Vec<PlotPoint> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mut baseline_idx = 0usize;
    let mut points = Vec::with_capacity(samples.len());
    for (index, (x_ms, cumulative_samples)) in samples.iter().copied().enumerate() {
        let target_x_ms = x_ms - window_ms;
        while baseline_idx + 1 < index && samples[baseline_idx + 1].0 <= target_x_ms {
            baseline_idx += 1;
        }
        let (baseline_x_ms, baseline_cumulative_samples) = if target_x_ms <= samples[0].0 {
            samples[0]
        } else {
            samples[baseline_idx]
        };
        let elapsed_ms = x_ms - baseline_x_ms;
        let delta_samples = cumulative_samples.saturating_sub(baseline_cumulative_samples);
        let elapsed_secs = elapsed_ms / 1000.0;
        let y = if elapsed_secs > 0.0 {
            (delta_samples as f64 / elapsed_secs).max(0.0)
        } else {
            0.0
        };
        points.push(PlotPoint {
            x: x_ms,
            y,
            y_min: None,
            y_max: None,
        });
    }
    points
}

fn queue_buffer_value(value: &JsonValue, key: &str) -> Option<JsonValue> {
    value.get("runner")?.get(key).cloned()
}

fn busy_ratio(idle_ratio: Option<f64>) -> Option<f64> {
    idle_ratio.map(|value| (1.0 - value).clamp(0.0, 1.0))
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
    let fetch_sync_ms = (metrics.avg_fetch_time_per_sample_ms
        - metrics.avg_fetch_stall_time_per_sample_ms)
        .max(0.0);
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
            "completed_merge",
            "Merge Completed Batches (sync)",
            runtime
                .sampler
                .completed_merge_ingest_ms
                .mean
                .unwrap_or(0.0),
            "#ca6702",
        ),
        (
            "completed_delete",
            "Cleanup Consumed Batches (sync)",
            runtime.sampler.completed_delete_ms.mean.unwrap_or(0.0),
            "#6d597a",
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
