use crate::core::{EvaluatorPerformanceMetrics, SamplerPerformanceMetrics, SamplerRuntimeMetrics};
use crate::server::panels::{
    KeyValueEntry, PanelDescriptor, PanelKind, PanelState, PerformanceHistoryResponse, PlotPoint,
    PlotSeries, TaskHistoryItem, history_item, history_x, key_value, key_value_panel,
    multi_timeseries_panel, panel_descriptor, scalar_timeseries_panel, single_point_band,
    single_point_series,
};
use crate::stores::{EvaluatorPerformanceHistoryEntry, SamplerPerformanceHistoryEntry};
use chrono::{DateTime, Utc};

pub fn build_evaluator_performance_response(
    scope_id: Option<String>,
    entries: Vec<EvaluatorPerformanceHistoryEntry>,
) -> PerformanceHistoryResponse {
    build_performance_history_response(
        scope_id,
        entries,
        evaluator_panel_descriptors,
        |entry| entry.id.to_string(),
        build_evaluator_current_panels,
        build_evaluator_history_item,
    )
}

pub fn build_sampler_performance_response(
    scope_id: Option<String>,
    entries: Vec<SamplerPerformanceHistoryEntry>,
) -> PerformanceHistoryResponse {
    build_performance_history_response(
        scope_id,
        entries,
        sampler_panel_descriptors,
        |entry| entry.id.to_string(),
        build_sampler_current_panels,
        build_sampler_history_item,
    )
}

fn build_performance_history_response<T>(
    scope_id: Option<String>,
    mut entries: Vec<T>,
    panel_descriptors: impl Fn(&[T]) -> Vec<PanelDescriptor>,
    snapshot_id: impl Fn(&T) -> String,
    build_current: impl Fn(&T) -> Vec<PanelState>,
    build_item: impl Fn(&T) -> TaskHistoryItem,
) -> PerformanceHistoryResponse {
    entries.reverse();
    let panels = panel_descriptors(&entries);
    let current = entries.last().map(build_current).unwrap_or_default();
    let latest_snapshot_id = entries.last().map(snapshot_id);
    let items = entries.iter().map(build_item).collect();

    PerformanceHistoryResponse {
        scope_id,
        latest_snapshot_id,
        reset_required: false,
        panels,
        current,
        items,
    }
}

fn evaluator_panel_descriptors(
    entries: &[EvaluatorPerformanceHistoryEntry],
) -> Vec<PanelDescriptor> {
    let mut panels = vec![
        panel_descriptor(
            "evaluator_throughput",
            "Evaluator Throughput",
            PanelKind::MultiTimeseries,
            true,
        ),
        panel_descriptor(
            "evaluator_timing",
            "Time Per Sample (ms)",
            PanelKind::ScalarTimeseries,
            true,
        ),
        panel_descriptor(
            "evaluator_summary",
            "Evaluator Summary",
            PanelKind::KeyValue,
            false,
        ),
    ];
    if entries
        .iter()
        .any(|entry| entry.metrics.idle_profile.is_some())
    {
        panels.push(panel_descriptor(
            "evaluator_idle_ratio",
            "Evaluator Idle Ratio",
            PanelKind::ScalarTimeseries,
            true,
        ));
    }
    panels
}

fn sampler_panel_descriptors(entries: &[SamplerPerformanceHistoryEntry]) -> Vec<PanelDescriptor> {
    let mut panels = vec![
        panel_descriptor(
            "sampler_counts",
            "Sampler Counts",
            PanelKind::MultiTimeseries,
            true,
        ),
        panel_descriptor(
            "sampler_produce_timing",
            "Produce Time (ms)",
            PanelKind::ScalarTimeseries,
            true,
        ),
        panel_descriptor(
            "sampler_ingest_timing",
            "Ingest Time (ms)",
            PanelKind::ScalarTimeseries,
            true,
        ),
        panel_descriptor(
            "sampler_summary",
            "Sampler Summary",
            PanelKind::KeyValue,
            false,
        ),
    ];
    if entries.iter().any(has_sampler_runtime_metrics) {
        panels.push(panel_descriptor(
            "sampler_runtime",
            "Sampler Runtime",
            PanelKind::MultiTimeseries,
            true,
        ));
    }
    panels
}

fn build_evaluator_current_panels(entry: &EvaluatorPerformanceHistoryEntry) -> Vec<PanelState> {
    evaluator_panels(entry, true)
}

fn build_evaluator_history_item(entry: &EvaluatorPerformanceHistoryEntry) -> TaskHistoryItem {
    history_item(
        entry.id,
        Some(entry.created_at),
        evaluator_panels(entry, false),
    )
}

fn build_sampler_current_panels(entry: &SamplerPerformanceHistoryEntry) -> Vec<PanelState> {
    sampler_panels(entry, true)
}

fn build_sampler_history_item(entry: &SamplerPerformanceHistoryEntry) -> TaskHistoryItem {
    history_item(
        entry.id,
        Some(entry.created_at),
        sampler_panels(entry, false),
    )
}

fn evaluator_panels(
    entry: &EvaluatorPerformanceHistoryEntry,
    include_summary: bool,
) -> Vec<PanelState> {
    let mut panels = vec![
        multi_timeseries_panel(
            "evaluator_throughput",
            evaluator_throughput_series(&entry.metrics, entry.created_at),
        ),
        single_point_band(
            "evaluator_timing",
            history_x(entry.created_at),
            entry.metrics.avg_time_per_sample_ms,
            Some(entry.metrics.avg_time_per_sample_ms - entry.metrics.std_time_per_sample_ms),
            Some(entry.metrics.avg_time_per_sample_ms + entry.metrics.std_time_per_sample_ms),
        ),
    ];
    if include_summary {
        panels.push(key_value_panel(
            "evaluator_summary",
            evaluator_summary_entries(&entry.metrics),
        ));
    }
    if let Some(idle_profile) = entry.metrics.idle_profile.as_ref() {
        panels.push(scalar_timeseries_panel(
            "evaluator_idle_ratio",
            vec![PlotPoint {
                x: history_x(entry.created_at),
                y: idle_profile.idle_ratio,
                y_min: None,
                y_max: None,
            }],
        ));
    }
    panels
}

fn sampler_panels(
    entry: &SamplerPerformanceHistoryEntry,
    include_summary: bool,
) -> Vec<PanelState> {
    let mut panels = vec![
        multi_timeseries_panel(
            "sampler_counts",
            sampler_count_series(&entry.metrics, entry.created_at),
        ),
        single_point_band(
            "sampler_produce_timing",
            history_x(entry.created_at),
            entry.metrics.avg_produce_time_per_sample_ms,
            Some(
                entry.metrics.avg_produce_time_per_sample_ms
                    - entry.metrics.std_produce_time_per_sample_ms,
            ),
            Some(
                entry.metrics.avg_produce_time_per_sample_ms
                    + entry.metrics.std_produce_time_per_sample_ms,
            ),
        ),
        single_point_band(
            "sampler_ingest_timing",
            history_x(entry.created_at),
            entry.metrics.avg_ingest_time_per_sample_ms,
            Some(
                entry.metrics.avg_ingest_time_per_sample_ms
                    - entry.metrics.std_ingest_time_per_sample_ms,
            ),
            Some(
                entry.metrics.avg_ingest_time_per_sample_ms
                    + entry.metrics.std_ingest_time_per_sample_ms,
            ),
        ),
    ];
    if include_summary {
        panels.push(key_value_panel(
            "sampler_summary",
            sampler_summary_entries(entry),
        ));
    }
    if let Some(runtime) = decode_sampler_runtime_metrics(entry) {
        panels.push(multi_timeseries_panel(
            "sampler_runtime",
            sampler_runtime_series(&runtime, entry.created_at),
        ));
    }
    panels
}

fn evaluator_throughput_series(
    metrics: &EvaluatorPerformanceMetrics,
    created_at: DateTime<Utc>,
) -> Vec<PlotSeries> {
    vec![
        metric_series(
            "batches_completed",
            "Batches Completed",
            created_at,
            metrics.batches_completed as f64,
        ),
        metric_series(
            "samples_evaluated",
            "Samples Evaluated",
            created_at,
            metrics.samples_evaluated as f64,
        ),
    ]
}

fn evaluator_summary_entries(metrics: &EvaluatorPerformanceMetrics) -> Vec<KeyValueEntry> {
    let mut entries = vec![
        key_value(
            "batches_completed",
            "Batches Completed",
            metrics.batches_completed,
        ),
        key_value(
            "samples_evaluated",
            "Samples Evaluated",
            metrics.samples_evaluated,
        ),
        key_value(
            "avg_time_per_sample_ms",
            "Avg Time Per Sample (ms)",
            metrics.avg_time_per_sample_ms,
        ),
        key_value(
            "std_time_per_sample_ms",
            "Std Time Per Sample (ms)",
            metrics.std_time_per_sample_ms,
        ),
    ];
    if let Some(idle_profile) = metrics.idle_profile.as_ref() {
        entries.push(key_value(
            "idle_ratio",
            "Idle Ratio",
            idle_profile.idle_ratio,
        ));
    }
    entries
}

fn sampler_count_series(
    metrics: &SamplerPerformanceMetrics,
    created_at: DateTime<Utc>,
) -> Vec<PlotSeries> {
    vec![
        metric_series(
            "produced_batches",
            "Produced Batches",
            created_at,
            metrics.produced_batches as f64,
        ),
        metric_series(
            "produced_samples",
            "Produced Samples",
            created_at,
            metrics.produced_samples as f64,
        ),
        metric_series(
            "ingested_batches",
            "Ingested Batches",
            created_at,
            metrics.ingested_batches as f64,
        ),
        metric_series(
            "ingested_samples",
            "Ingested Samples",
            created_at,
            metrics.ingested_samples as f64,
        ),
    ]
}

fn sampler_runtime_series(
    runtime: &SamplerRuntimeMetrics,
    created_at: DateTime<Utc>,
) -> Vec<PlotSeries> {
    vec![
        metric_series(
            "completed_samples_per_second",
            "Completed Samples / s",
            created_at,
            runtime.completed_samples_per_second,
        ),
        metric_series(
            "batch_size_current",
            "Batch Size",
            created_at,
            runtime.batch_size_current as f64,
        ),
    ]
}

fn sampler_summary_entries(entry: &SamplerPerformanceHistoryEntry) -> Vec<KeyValueEntry> {
    let mut entries = vec![
        key_value(
            "produced_batches",
            "Produced Batches",
            entry.metrics.produced_batches,
        ),
        key_value(
            "produced_samples",
            "Produced Samples",
            entry.metrics.produced_samples,
        ),
        key_value(
            "ingested_batches",
            "Ingested Batches",
            entry.metrics.ingested_batches,
        ),
        key_value(
            "ingested_samples",
            "Ingested Samples",
            entry.metrics.ingested_samples,
        ),
    ];
    if let Some(runtime) = decode_sampler_runtime_metrics(entry) {
        entries.push(key_value(
            "completed_samples_per_second",
            "Completed Samples / s",
            runtime.completed_samples_per_second,
        ));
        entries.push(key_value(
            "batch_size_current",
            "Batch Size",
            runtime.batch_size_current,
        ));
    }
    entries
}

fn decode_sampler_runtime_metrics(
    entry: &SamplerPerformanceHistoryEntry,
) -> Option<SamplerRuntimeMetrics> {
    serde_json::from_value(entry.runtime_metrics.clone()).ok()
}

fn has_sampler_runtime_metrics(entry: &SamplerPerformanceHistoryEntry) -> bool {
    decode_sampler_runtime_metrics(entry).is_some()
}

fn metric_series(id: &str, label: &str, created_at: DateTime<Utc>, y: f64) -> PlotSeries {
    single_point_series(id, label, history_x(created_at), y)
}
