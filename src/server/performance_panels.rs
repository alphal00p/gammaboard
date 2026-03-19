use crate::core::SamplerRuntimeMetrics;
use crate::server::panels::{
    PanelDescriptor, PanelKind, PanelState, PerformanceHistoryResponse, PlotPoint, TaskHistoryItem,
    history_item, history_x, panel_descriptor, scalar_timeseries_panel,
};
use crate::stores::{EvaluatorPerformanceHistoryEntry, SamplerPerformanceHistoryEntry};

pub fn build_evaluator_performance_response(
    scope_id: Option<String>,
    entries: Vec<EvaluatorPerformanceHistoryEntry>,
) -> PerformanceHistoryResponse {
    build_performance_history_response(
        scope_id,
        entries,
        evaluator_panel_descriptors(),
        |entry| entry.id.to_string(),
        evaluator_current_panels,
        evaluator_history_item,
    )
}

pub fn build_sampler_performance_response(
    scope_id: Option<String>,
    entries: Vec<SamplerPerformanceHistoryEntry>,
) -> PerformanceHistoryResponse {
    build_performance_history_response(
        scope_id,
        entries,
        sampler_panel_descriptors(),
        |entry| entry.id.to_string(),
        sampler_current_panels,
        sampler_history_item,
    )
}

fn build_performance_history_response<T>(
    scope_id: Option<String>,
    mut entries: Vec<T>,
    panels: Vec<PanelDescriptor>,
    snapshot_id: impl Fn(&T) -> String,
    build_current: impl Fn(&T) -> Vec<PanelState>,
    build_item: impl Fn(&T) -> TaskHistoryItem,
) -> PerformanceHistoryResponse {
    entries.reverse();
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

fn evaluator_panel_descriptors() -> Vec<PanelDescriptor> {
    vec![
        panel_descriptor(
            "evaluator_idle_ratio",
            "Idle Ratio",
            PanelKind::ScalarTimeseries,
            true,
        ),
        panel_descriptor(
            "evaluator_evaluate_time_us",
            "Evaluate Time Per Sample (us)",
            PanelKind::ScalarTimeseries,
            true,
        ),
        panel_descriptor(
            "evaluator_parametrization_time_us",
            "Parametrization Time Per Sample (us)",
            PanelKind::ScalarTimeseries,
            true,
        ),
    ]
}

fn sampler_panel_descriptors() -> Vec<PanelDescriptor> {
    vec![
        panel_descriptor(
            "sampler_completed_samples_per_second",
            "Completed Samples Per Second",
            PanelKind::ScalarTimeseries,
            true,
        ),
        panel_descriptor(
            "sampler_queue_remaining_ratio",
            "Queue Remaining Ratio",
            PanelKind::ScalarTimeseries,
            true,
        ),
    ]
}

fn evaluator_current_panels(entry: &EvaluatorPerformanceHistoryEntry) -> Vec<PanelState> {
    evaluator_panels(entry)
}

fn evaluator_history_item(entry: &EvaluatorPerformanceHistoryEntry) -> TaskHistoryItem {
    history_item(entry.id, Some(entry.created_at), evaluator_panels(entry))
}

fn sampler_current_panels(entry: &SamplerPerformanceHistoryEntry) -> Vec<PanelState> {
    sampler_panels(entry)
}

fn sampler_history_item(entry: &SamplerPerformanceHistoryEntry) -> TaskHistoryItem {
    history_item(entry.id, Some(entry.created_at), sampler_panels(entry))
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
            "evaluator_parametrization_time_us",
            history_x(entry.created_at),
            ms_to_us(entry.metrics.avg_parametrization_time_per_sample_ms),
            Some(ms_to_us(
                entry.metrics.avg_parametrization_time_per_sample_ms
                    - entry.metrics.std_parametrization_time_per_sample_ms,
            )),
            Some(ms_to_us(
                entry.metrics.avg_parametrization_time_per_sample_ms
                    + entry.metrics.std_parametrization_time_per_sample_ms,
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
            "sampler_queue_remaining_ratio",
            history_x(entry.created_at),
            runtime.rolling.queue_remaining_ratio.mean.unwrap_or(0.0),
            Some(
                runtime.rolling.queue_remaining_ratio.mean.unwrap_or(0.0)
                    - runtime.rolling.queue_remaining_ratio.std_dev,
            ),
            Some(
                runtime.rolling.queue_remaining_ratio.mean.unwrap_or(0.0)
                    + runtime.rolling.queue_remaining_ratio.std_dev,
            ),
        ),
    ]
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
