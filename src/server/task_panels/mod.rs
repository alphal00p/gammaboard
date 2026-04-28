mod full_accumulator;
mod pdf_adaptation;
mod sample;

use crate::core::{AccumulatorConfig, EngineError, RunTask, RunTaskSpec};
use crate::evaluation::AccumulatorState;
use crate::server::panels::{
    PanelHistoryMode, PanelResponse, PanelSpec, PanelState, PanelUpdate, append_panel,
    merge_panel_state, replace_panel,
};
use crate::stores::{TaskOutputSnapshot, TaskStageSnapshot};
use serde_json::Value as JsonValue;

const DEFAULT_HISTORY_POINT_BUDGET: usize = 256;
const EMPTY_HISTORY_CURSOR_SNAPSHOT_ID: i64 = 0;

type CurrentProjectorFn =
    dyn for<'a> Fn(&TaskPanelContext<'a>) -> Result<Option<PanelState>, EngineError> + Send + Sync;
type HistoryProjectorFn = dyn for<'a> Fn(&TaskPanelHistoryContext<'a>) -> Result<Option<PanelState>, EngineError>
    + Send
    + Sync;

pub struct TaskPanelProjector {
    spec: PanelSpec,
    current_source_policy: TaskPanelCurrentSourcePolicy,
    current: Box<CurrentProjectorFn>,
    history: Box<HistoryProjectorFn>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskPanelCurrentSourcePolicy {
    #[default]
    StageFirst,
    PersistedFirst,
}

#[derive(Clone, Copy)]
pub enum TaskPanelCurrentSource<'a> {
    Runtime(&'a AccumulatorState),
    StageSnapshot(&'a TaskStageSnapshot),
    Persisted(&'a JsonValue),
    Empty,
}

impl TaskPanelCurrentSource<'_> {
    pub fn accumulator(&self) -> Option<&AccumulatorState> {
        match self {
            Self::Runtime(accumulator) => Some(accumulator),
            Self::StageSnapshot(snapshot) => Some(&snapshot.observable_state),
            Self::Persisted(_) | Self::Empty => None,
        }
    }

    pub fn persisted(&self) -> Option<&JsonValue> {
        match self {
            Self::Persisted(value) => Some(value),
            Self::Runtime(_) | Self::StageSnapshot(_) | Self::Empty => None,
        }
    }
}

pub struct TaskPanelContext<'a> {
    pub task: &'a RunTask,
    pub source: TaskPanelCurrentSource<'a>,
    pub panel_state: &'a JsonValue,
    pub run_target: Option<&'a JsonValue>,
    pub completed_samples_per_second: Option<f64>,
    pub smoothed_eta_seconds: Option<f64>,
}

impl TaskPanelContext<'_> {
    pub fn selected_value(&self, panel_id: &str) -> Option<&str> {
        self.panel_state
            .as_object()
            .and_then(|state| state.get(panel_id))
            .and_then(JsonValue::as_str)
    }
}

pub struct TaskPanelHistoryContext<'a> {
    pub snapshot: &'a TaskOutputSnapshot,
}

pub struct TaskPanelSource {
    projectors: Vec<TaskPanelProjector>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskPanelCursor {
    pub snapshot_id: Option<i64>,
    pub downsample_level: u8,
}

impl TaskPanelProjector {
    pub fn spec(&self) -> &PanelSpec {
        &self.spec
    }

    pub fn current(&self, ctx: &TaskPanelContext<'_>) -> Result<Option<PanelState>, EngineError> {
        (self.current)(ctx)
    }

    fn current_source_policy(&self) -> TaskPanelCurrentSourcePolicy {
        self.current_source_policy
    }

    pub fn history(
        &self,
        ctx: &TaskPanelHistoryContext<'_>,
    ) -> Result<Option<PanelState>, EngineError> {
        (self.history)(ctx)
    }
}

pub fn panel_projector(
    spec: PanelSpec,
    current: impl for<'a> Fn(&TaskPanelContext<'a>) -> Result<Option<PanelState>, EngineError>
    + Send
    + Sync
    + 'static,
    history: impl for<'a> Fn(&TaskPanelHistoryContext<'a>) -> Result<Option<PanelState>, EngineError>
    + Send
    + Sync
    + 'static,
) -> TaskPanelProjector {
    panel_projector_with_source(
        spec,
        TaskPanelCurrentSourcePolicy::StageFirst,
        current,
        history,
    )
}

pub fn panel_projector_with_source(
    spec: PanelSpec,
    current_source_policy: TaskPanelCurrentSourcePolicy,
    current: impl for<'a> Fn(&TaskPanelContext<'a>) -> Result<Option<PanelState>, EngineError>
    + Send
    + Sync
    + 'static,
    history: impl for<'a> Fn(&TaskPanelHistoryContext<'a>) -> Result<Option<PanelState>, EngineError>
    + Send
    + Sync
    + 'static,
) -> TaskPanelProjector {
    TaskPanelProjector {
        spec,
        current_source_policy,
        current: Box::new(current),
        history: Box::new(history),
    }
}

fn project_current_panels(
    projectors: &[TaskPanelProjector],
    task: &RunTask,
    panel_state: &JsonValue,
    run_target: Option<&JsonValue>,
    completed_samples_per_second: Option<f64>,
    smoothed_eta_seconds: Option<f64>,
    current_accumulator: Option<&AccumulatorState>,
    latest_stage_snapshot: Option<&TaskStageSnapshot>,
    latest_persisted_snapshot: Option<&TaskOutputSnapshot>,
) -> Result<Vec<PanelState>, EngineError> {
    projectors
        .iter()
        .filter_map(|projector| {
            let source = resolve_current_source(
                task,
                current_accumulator,
                latest_stage_snapshot,
                latest_persisted_snapshot,
                projector.current_source_policy(),
            );
            projector
                .current(&TaskPanelContext {
                    task,
                    source,
                    panel_state,
                    run_target,
                    completed_samples_per_second,
                    smoothed_eta_seconds,
                })
                .transpose()
        })
        .collect()
}

fn project_history_panels(
    projectors: &[TaskPanelProjector],
    ctx: &TaskPanelHistoryContext<'_>,
) -> Result<Vec<PanelState>, EngineError> {
    projectors
        .iter()
        .filter_map(|projector| projector.history(ctx).transpose())
        .collect()
}

fn project_snapshot_history_panels(
    projectors: &[TaskPanelProjector],
    snapshots: &[TaskOutputSnapshot],
) -> Result<Vec<Vec<PanelState>>, EngineError> {
    snapshots
        .iter()
        .rev()
        .map(|snapshot| project_history_panels(projectors, &TaskPanelHistoryContext { snapshot }))
        .collect()
}

impl RunTaskSpec {
    fn panel_projectors(
        &self,
        effective_accumulator_config: Option<AccumulatorConfig>,
    ) -> Vec<TaskPanelProjector> {
        let mut projectors = Vec::new();
        projectors.extend(match self {
            Self::Sample { .. } => sample::projectors(self, effective_accumulator_config),
            Self::Image {
                geometry, display, ..
            } => full_accumulator::image_projectors(geometry.clone(), *display),
            Self::PdfAdaptationImage { geometry, .. } => {
                pdf_adaptation::projectors(geometry.clone(), crate::core::ImageDisplayMode::Auto)
            }
            Self::PdfAdaptationPlotLine { geometry, .. } => pdf_adaptation::line_projectors(
                geometry.clone(),
                crate::core::LineDisplayMode::Auto,
            ),
            Self::PlotLine {
                geometry,
                display,
                accumulator,
                ..
            } => full_accumulator::line_projectors(geometry.clone(), *display, *accumulator),
        });
        projectors
    }
}

impl TaskPanelSource {
    pub fn new(
        task_spec: &RunTaskSpec,
        effective_accumulator_config: Option<AccumulatorConfig>,
    ) -> Self {
        Self {
            projectors: task_spec.panel_projectors(effective_accumulator_config),
        }
    }

    pub fn panel_specs(&self) -> Vec<PanelSpec> {
        self.projectors
            .iter()
            .map(|projector| projector.spec().clone())
            .collect()
    }

    pub fn needs_history(&self) -> bool {
        self.projectors
            .iter()
            .any(|projector| projector.spec().history != PanelHistoryMode::None)
    }

    pub fn build_response(
        &self,
        source_id: String,
        requested_cursor: TaskPanelCursor,
        task: &RunTask,
        panel_state: &JsonValue,
        run_target: Option<&JsonValue>,
        completed_samples_per_second: Option<f64>,
        smoothed_eta_seconds: Option<f64>,
        current_accumulator: Option<&AccumulatorState>,
        latest_stage_snapshot: Option<&TaskStageSnapshot>,
        latest_persisted_snapshot: Option<&TaskOutputSnapshot>,
        full_history_snapshots: &[TaskOutputSnapshot],
        delta_history_snapshots: &[TaskOutputSnapshot],
    ) -> Result<PanelResponse, EngineError> {
        let current_panels = project_current_panels(
            &self.projectors,
            task,
            panel_state,
            run_target,
            completed_samples_per_second,
            smoothed_eta_seconds,
            current_accumulator,
            latest_stage_snapshot,
            latest_persisted_snapshot,
        )?;
        let panels = self.panel_specs();
        let (updates, target_level) = if requested_cursor.snapshot_id.is_some() {
            let delta_history_panels =
                project_snapshot_history_panels(&self.projectors, delta_history_snapshots)?;
            let mut updates = incremental_updates(&panels, current_panels, delta_history_panels);
            downsample_append_updates(&panels, &mut updates, requested_cursor.downsample_level);
            (updates, requested_cursor.downsample_level)
        } else {
            let full_history_panels =
                project_snapshot_history_panels(&self.projectors, full_history_snapshots)?;
            compacted_full_updates(&panels, current_panels, full_history_panels)
        };
        let cursor_snapshot_id = latest_persisted_snapshot
            .and_then(|snapshot| snapshot.id.parse::<i64>().ok())
            .or(requested_cursor.snapshot_id)
            .or_else(|| {
                self.needs_history()
                    .then_some(EMPTY_HISTORY_CURSOR_SNAPSHOT_ID)
            });
        let cursor = format_cursor(TaskPanelCursor {
            snapshot_id: cursor_snapshot_id,
            downsample_level: target_level,
        });

        Ok(PanelResponse {
            source_id,
            cursor,
            reset_required: false,
            panels,
            updates,
        })
    }
}

fn resolve_current_source<'a>(
    task: &RunTask,
    current_accumulator: Option<&'a AccumulatorState>,
    latest_stage_snapshot: Option<&'a TaskStageSnapshot>,
    latest_persisted_snapshot: Option<&'a TaskOutputSnapshot>,
    policy: TaskPanelCurrentSourcePolicy,
) -> TaskPanelCurrentSource<'a> {
    if matches!(task.state, crate::core::RunTaskState::Active) {
        if let Some(accumulator) = current_accumulator {
            return TaskPanelCurrentSource::Runtime(accumulator);
        }
    }
    match policy {
        TaskPanelCurrentSourcePolicy::StageFirst => {
            if let Some(snapshot) = latest_stage_snapshot {
                return TaskPanelCurrentSource::StageSnapshot(snapshot);
            }
            if let Some(snapshot) = latest_persisted_snapshot {
                return TaskPanelCurrentSource::Persisted(&snapshot.persisted_output);
            }
        }
        TaskPanelCurrentSourcePolicy::PersistedFirst => {
            if let Some(snapshot) = latest_persisted_snapshot {
                return TaskPanelCurrentSource::Persisted(&snapshot.persisted_output);
            }
            if let Some(snapshot) = latest_stage_snapshot {
                return TaskPanelCurrentSource::StageSnapshot(snapshot);
            }
        }
    }
    TaskPanelCurrentSource::Empty
}

fn full_updates(
    specs: &[PanelSpec],
    current_panels: Vec<PanelState>,
    history_panels: Vec<Vec<PanelState>>,
) -> Vec<PanelUpdate> {
    let mut state_by_id = panel_state_map(current_panels);
    for panels in history_panels {
        for panel in panels {
            let panel_id = panel.panel_id().to_string();
            if history_mode_for(specs, &panel_id) != PanelHistoryMode::Append {
                continue;
            }
            if let Some(existing) = state_by_id.get_mut(&panel_id) {
                merge_panel_state(existing, panel);
            } else {
                state_by_id.insert(panel_id, panel);
            }
        }
    }
    state_by_id.into_values().map(replace_panel).collect()
}

fn compacted_full_updates(
    specs: &[PanelSpec],
    current_panels: Vec<PanelState>,
    history_panels: Vec<Vec<PanelState>>,
) -> (Vec<PanelUpdate>, u8) {
    let mut updates = full_updates(specs, current_panels, history_panels);
    let level = target_downsample_level(specs, &updates);
    downsample_append_updates(specs, &mut updates, level);
    (updates, level)
}

fn incremental_updates(
    specs: &[PanelSpec],
    current_panels: Vec<PanelState>,
    history_panels: Vec<Vec<PanelState>>,
) -> Vec<PanelUpdate> {
    let mut updates = current_panels
        .iter()
        .filter(|panel| history_mode_for(specs, panel.panel_id()) == PanelHistoryMode::None)
        .cloned()
        .map(replace_panel)
        .collect::<Vec<_>>();

    let mut delta_by_id = std::collections::BTreeMap::new();
    for panel in current_panels {
        let panel_id = panel.panel_id().to_string();
        if history_mode_for(specs, &panel_id) != PanelHistoryMode::Append {
            continue;
        }
        if let Some(existing) = delta_by_id.get_mut(&panel_id) {
            merge_panel_state(existing, panel);
        } else {
            delta_by_id.insert(panel_id, panel);
        }
    }
    for panels in history_panels {
        for panel in panels {
            let panel_id = panel.panel_id().to_string();
            if history_mode_for(specs, &panel_id) != PanelHistoryMode::Append {
                continue;
            }
            if let Some(existing) = delta_by_id.get_mut(&panel_id) {
                merge_panel_state(existing, panel);
            } else {
                delta_by_id.insert(panel_id, panel);
            }
        }
    }
    updates.extend(delta_by_id.into_values().map(append_panel));
    updates
}

fn downsample_append_updates(specs: &[PanelSpec], updates: &mut [PanelUpdate], level: u8) {
    if level == 0 {
        return;
    }
    for update in updates {
        if history_mode_for(specs, update.panel.panel_id()) == PanelHistoryMode::Append {
            downsample_panel_state(&mut update.panel, level);
        }
    }
}

fn panel_state_map(panels: Vec<PanelState>) -> std::collections::BTreeMap<String, PanelState> {
    panels
        .into_iter()
        .map(|panel| (panel.panel_id().to_string(), panel))
        .collect()
}

fn history_mode_for(specs: &[PanelSpec], panel_id: &str) -> PanelHistoryMode {
    specs
        .iter()
        .find(|spec| spec.panel_id == panel_id)
        .map(|spec| spec.history.clone())
        .unwrap_or(PanelHistoryMode::None)
}

fn target_downsample_level(specs: &[PanelSpec], updates: &[PanelUpdate]) -> u8 {
    updates
        .iter()
        .filter(|update| {
            history_mode_for(specs, update.panel.panel_id()) == PanelHistoryMode::Append
        })
        .filter_map(|update| history_point_count(&update.panel))
        .map(required_downsample_level)
        .max()
        .unwrap_or(0)
}

fn required_downsample_level(point_count: usize) -> u8 {
    let mut level = 0u8;
    let mut visible_points = point_count;
    while visible_points > DEFAULT_HISTORY_POINT_BUDGET {
        level = level.saturating_add(1);
        visible_points = visible_points.div_ceil(2);
    }
    level
}

fn history_point_count(panel: &PanelState) -> Option<usize> {
    match panel {
        PanelState::ScalarTimeseries { points, .. } => Some(points.len()),
        PanelState::MultiTimeseries { series, .. } => Some(
            series
                .iter()
                .map(|item| item.points.len())
                .max()
                .unwrap_or(0),
        ),
        _ => None,
    }
}

fn downsample_panel_state(panel: &mut PanelState, level: u8) {
    if level == 0 {
        return;
    }
    let stride = 1usize << level;
    match panel {
        PanelState::ScalarTimeseries { points, .. } => {
            *points = downsample_points(points, stride);
        }
        PanelState::MultiTimeseries { series, .. } => {
            for item in series {
                item.points = downsample_points(&item.points, stride);
            }
        }
        _ => {}
    }
}

fn downsample_points(
    points: &[crate::server::panels::PlotPoint],
    stride: usize,
) -> Vec<crate::server::panels::PlotPoint> {
    if stride <= 1 || points.len() <= DEFAULT_HISTORY_POINT_BUDGET {
        return points.to_vec();
    }

    let mut compacted = points
        .iter()
        .enumerate()
        .filter(|(index, _)| index % stride == 0)
        .map(|(_, point)| point.clone())
        .collect::<Vec<_>>();

    if let Some(last) = points.last() {
        let needs_last = compacted
            .last()
            .is_none_or(|point| point.x != last.x || point.y != last.y);
        if needs_last {
            compacted.push(last.clone());
        }
    }
    compacted
}

pub fn parse_cursor(cursor: Option<&str>) -> Result<TaskPanelCursor, String> {
    let Some(cursor) = cursor else {
        return Ok(TaskPanelCursor::default());
    };
    let mut parts = cursor.split(':');
    let snapshot_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("invalid after_cursor={cursor:?}"))?
        .parse::<i64>()
        .map_err(|_| format!("invalid after_cursor={cursor:?}"))?;
    let downsample_level = match parts.next() {
        Some(value) if !value.is_empty() => value
            .parse::<u8>()
            .map_err(|_| format!("invalid after_cursor={cursor:?}"))?,
        _ => 0,
    };
    if parts.next().is_some() {
        return Err(format!("invalid after_cursor={cursor:?}"));
    }
    Ok(TaskPanelCursor {
        snapshot_id: Some(snapshot_id),
        downsample_level,
    })
}

fn format_cursor(cursor: TaskPanelCursor) -> Option<String> {
    cursor
        .snapshot_id
        .map(|snapshot_id| format!("{snapshot_id}:{}", cursor.downsample_level))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        AccumulatorConfig, LineDisplayMode, RunTaskInput, RunTaskState, canonical_task_toml,
    };
    use crate::evaluation::{
        AccumulatorState, ComplexAccumulatorState, ComplexValue, FullComplexAccumulatorState,
        GammaLoopAccumulatorState, GammaLoopDiagnostics,
    };
    use crate::server::panels::{
        PanelKind, PanelUpdateMode, PlotPoint, panel_spec, scalar_timeseries_panel,
    };
    use chrono::Utc;
    use gammalooprs::observables::{
        HistogramBinSnapshot, HistogramSnapshot, HistogramSnapshotKind,
        HistogramStatisticsSnapshot, ObservablePhase, ObservableSnapshotBundle,
        ObservableValueTransform,
    };

    fn line_geometry() -> crate::core::LineRasterGeometry {
        crate::core::LineRasterGeometry {
            offset: vec![0.0],
            direction: vec![1.0],
            linspace: crate::core::Linspace {
                start: -1.0,
                stop: 1.0,
                count: 3,
            },
            discrete: Vec::new(),
        }
    }

    fn plot_task(display: LineDisplayMode) -> RunTaskSpec {
        RunTaskSpec::PlotLine {
            geometry: line_geometry(),
            accumulator: crate::core::PlotAccumulatorKind::Complex,
            display,
            batch_transforms: None,
        }
    }

    fn inherited_complex_sample_task() -> RunTaskSpec {
        RunTaskSpec::Sample {
            stop_condition: crate::core::SampleStopCondition {
                max_samples: Some(10),
                ..crate::core::SampleStopCondition::default()
            },
            sampler_aggregator: None,
            accumulator: None,
            queue_tuning: None,
            batch_transforms: None,
        }
    }

    fn run_task(task: RunTaskSpec) -> RunTask {
        let name = "plot_line-1".to_string();
        RunTask {
            id: 1,
            run_id: 1,
            name: name.clone(),
            sequence_nr: 1,
            task: task.clone(),
            spawned_from_snapshot_id: None,
            state: RunTaskState::Active,
            nr_produced_samples: 3,
            nr_completed_samples: 3,
            failure_reason: None,
            started_at: None,
            completed_at: None,
            failed_at: None,
            created_at: Utc::now(),
            task_toml: canonical_task_toml(&RunTaskInput {
                name: Some(name),
                task,
            })
            .expect("task toml"),
        }
    }

    fn complex_observable() -> AccumulatorState {
        AccumulatorState::FullComplex(FullComplexAccumulatorState {
            values: vec![
                ComplexValue { re: 1.0, im: -1.0 },
                ComplexValue { re: 2.0, im: -2.0 },
                ComplexValue { re: 3.0, im: -3.0 },
            ],
            nan_entries: vec![],
        })
    }

    fn gammaloop_accumulator() -> AccumulatorState {
        AccumulatorState::Gammaloop(GammaLoopAccumulatorState {
            bundle: ObservableSnapshotBundle {
                histograms: std::collections::BTreeMap::from([
                    (
                        "pt".to_string(),
                        HistogramSnapshot {
                            kind: HistogramSnapshotKind::Continuous,
                            title: "pt".to_string(),
                            type_description: "HwU".to_string(),
                            phase: ObservablePhase::Real,
                            value_transform: ObservableValueTransform::Identity,
                            supports_misbinning_mitigation: false,
                            discrete_min_bin_id: None,
                            discrete_ordering: None,
                            x_min: Some(0.0),
                            x_max: Some(10.0),
                            sample_count: 2,
                            log_x_axis: false,
                            log_y_axis: false,
                            bins: vec![HistogramBinSnapshot {
                                x_min: Some(0.0),
                                x_max: Some(10.0),
                                bin_id: None,
                                label: None,
                                entry_count: 2,
                                sum_weights: 4.0,
                                sum_weights_squared: 10.0,
                                mitigated_fill_count: 0,
                            }],
                            underflow_bin: HistogramBinSnapshot {
                                x_min: None,
                                x_max: None,
                                bin_id: None,
                                label: None,
                                entry_count: 0,
                                sum_weights: 0.0,
                                sum_weights_squared: 0.0,
                                mitigated_fill_count: 0,
                            },
                            overflow_bin: HistogramBinSnapshot {
                                x_min: None,
                                x_max: None,
                                bin_id: None,
                                label: None,
                                entry_count: 0,
                                sum_weights: 0.0,
                                sum_weights_squared: 0.0,
                                mitigated_fill_count: 0,
                            },
                            statistics: HistogramStatisticsSnapshot {
                                in_range_entry_count: 2,
                                nan_value_count: 0,
                                mitigated_pair_count: 0,
                            },
                        },
                    ),
                    (
                        "eta".to_string(),
                        HistogramSnapshot {
                            kind: HistogramSnapshotKind::Continuous,
                            discrete_min_bin_id: None,
                            discrete_ordering: None,
                            title: "eta".to_string(),
                            type_description: "HwU".to_string(),
                            phase: ObservablePhase::Imag,
                            value_transform: ObservableValueTransform::Log10,
                            supports_misbinning_mitigation: true,
                            x_min: Some(-1.0),
                            x_max: Some(1.0),
                            sample_count: 1,
                            log_x_axis: true,
                            log_y_axis: true,
                            bins: vec![HistogramBinSnapshot {
                                x_min: Some(-1.0),
                                x_max: Some(1.0),
                                bin_id: None,
                                label: None,
                                entry_count: 1,
                                sum_weights: 2.0,
                                sum_weights_squared: 4.0,
                                mitigated_fill_count: 1,
                            }],
                            underflow_bin: HistogramBinSnapshot {
                                x_min: None,
                                x_max: None,
                                bin_id: None,
                                label: None,
                                entry_count: 0,
                                sum_weights: 0.0,
                                sum_weights_squared: 0.0,
                                mitigated_fill_count: 0,
                            },
                            overflow_bin: HistogramBinSnapshot {
                                x_min: None,
                                x_max: None,
                                bin_id: None,
                                label: None,
                                entry_count: 0,
                                sum_weights: 0.0,
                                sum_weights_squared: 0.0,
                                mitigated_fill_count: 0,
                            },
                            statistics: HistogramStatisticsSnapshot {
                                in_range_entry_count: 1,
                                nan_value_count: 0,
                                mitigated_pair_count: 0,
                            },
                        },
                    ),
                ]),
            },
            estimate: ComplexAccumulatorState {
                count: 3,
                real_sum: 7.0,
                imag_sum: -1.0,
                abs_sum: 8.0,
                abs_sq_sum: 20.0,
                real_sq_sum: 17.0,
                imag_sq_sum: 5.0,
                weight_sum: 3.0,
                nan_count: 0,
                ..Default::default()
            },
            diagnostics: GammaLoopDiagnostics {
                count_total: 3,
                count_double_precision: 2,
                count_quad_precision: 1,
                count_arb_precision: 0,
                count_nan: 0,
                count_nan_or_unstable: 1,
                count_loop_momenta_escalated: 1,
                total_eval_time_ms: 12.0,
                total_integrand_eval_time_ms: 7.0,
                total_evaluator_eval_time_ms: 3.0,
                total_parameterization_time_ms: 1.0,
                total_event_processing_time_ms: 1.0,
                total_generated_events: 10,
                total_accepted_events: 7,
            },
        })
    }

    fn current_panels(
        task_spec: &RunTaskSpec,
        task: &RunTask,
        accumulator: &AccumulatorState,
    ) -> Vec<PanelState> {
        TaskPanelSource::new(task_spec, None)
            .current_panels(
                task,
                &JsonValue::Object(Default::default()),
                None,
                None,
                None,
                Some(accumulator),
                None,
                None,
            )
            .unwrap()
    }

    #[test]
    fn complex_line_auto_uses_multi_timeseries_components_panel() {
        let task = plot_task(LineDisplayMode::Auto);
        let descriptors = TaskPanelSource::new(&task, None).panel_specs();
        assert!(
            descriptors
                .iter()
                .any(|panel| panel.panel_id == "line_components")
        );
        assert!(
            !descriptors
                .iter()
                .any(|panel| panel.panel_id == "line_imag")
        );

        let run_task = run_task(task.clone());
        let accumulator = complex_observable();
        let current = current_panels(&task, &run_task, &accumulator);
        let panel = current
            .into_iter()
            .find(|panel| matches!(panel, PanelState::MultiTimeseries { panel_id, .. } if panel_id == "line_components"))
            .expect("missing line_components panel");
        let PanelState::MultiTimeseries { series, .. } = panel else {
            panic!("expected multi_timeseries panel");
        };
        assert_eq!(series.len(), 2);
    }

    #[test]
    fn complex_line_scalar_curve_uses_single_real_panel() {
        let task = plot_task(LineDisplayMode::ScalarCurve);
        let descriptors = TaskPanelSource::new(&task, None).panel_specs();
        assert!(
            descriptors
                .iter()
                .any(|panel| panel.panel_id == "line_real")
        );
        assert!(
            !descriptors
                .iter()
                .any(|panel| panel.panel_id == "line_components")
        );

        let run_task = run_task(task.clone());
        let accumulator = complex_observable();
        let current = current_panels(&task, &run_task, &accumulator);
        assert!(
            current
                .iter()
                .any(|panel| matches!(panel, PanelState::ScalarTimeseries { panel_id, .. } if panel_id == "line_real"))
        );
        assert!(
            !current
                .iter()
                .any(|panel| matches!(panel, PanelState::MultiTimeseries { .. }))
        );
    }

    #[test]
    fn inherited_complex_sample_uses_imag_panel() {
        let task = inherited_complex_sample_task();
        let descriptors =
            TaskPanelSource::new(&task, Some(AccumulatorConfig::Complex)).panel_specs();
        assert!(
            descriptors
                .iter()
                .any(|panel| panel.panel_id == "imag_estimate_history")
        );
    }

    #[test]
    fn gammaloop_sample_exposes_histogram_bundle_panels() {
        let task = inherited_complex_sample_task();
        let descriptors =
            TaskPanelSource::new(&task, Some(AccumulatorConfig::Gammaloop)).panel_specs();
        assert!(
            descriptors
                .iter()
                .any(|panel| panel.panel_id == "gammaloop_histogram_bundle")
        );

        let run_task = run_task(task.clone());
        let accumulator = gammaloop_accumulator();
        let current = TaskPanelSource::new(&task, Some(AccumulatorConfig::Gammaloop))
            .current_panels(
                &run_task,
                &JsonValue::Object(Default::default()),
                None,
                None,
                None,
                Some(&accumulator),
                None,
                None,
            )
            .unwrap();
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::Table { panel_id, columns, rows, .. }
                if panel_id == "gammaloop_histogram_bundle"
                    && columns.len() >= 5
                    && rows.len() == 2
        )));
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::TickBreakdown { panel_id, total_ms, segments }
                if panel_id == "gammaloop_evaluation_timing"
                    && *total_ms > 0.0
                    && !segments.is_empty()
        )));
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::KeyValue { panel_id, entries }
                if panel_id == "gammaloop_evaluation_diagnostics"
                    && entries.iter().any(|entry| entry.key == "count_arb_precision")
        )));
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::KeyValue { panel_id, entries }
                if panel_id == "max_weight_summary"
                    && entries.iter().any(|entry| entry.key == "max_weight_impact")
        )));
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::Table { panel_id, columns, .. }
                if panel_id == "max_weight_points" && columns.len() == 7
        )));
    }

    #[test]
    fn gammaloop_sample_uses_complex_target_for_lines_and_summary_metrics() {
        let task = inherited_complex_sample_task();
        let run_task = run_task(task.clone());
        let accumulator = gammaloop_accumulator();
        let run_target = serde_json::json!({
            "kind": "complex",
            "re": 1.5,
            "im": -0.25
        });
        let current = TaskPanelSource::new(&task, Some(AccumulatorConfig::Gammaloop))
            .current_panels(
                &run_task,
                &JsonValue::Object(Default::default()),
                Some(&run_target),
                None,
                None,
                Some(&accumulator),
                None,
                None,
            )
            .unwrap();

        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::ScalarTimeseries { panel_id, target: Some(target), .. }
                if panel_id == "real_estimate_history" && (*target - 1.5).abs() < 1.0e-12
        )));
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::ScalarTimeseries { panel_id, target: Some(target), .. }
                if panel_id == "imag_estimate_history" && (*target + 0.25).abs() < 1.0e-12
        )));
        assert!(current.iter().any(|panel| matches!(
            panel,
            PanelState::KeyValue { panel_id, entries }
                if panel_id == "estimate_summary"
                    && entries.iter().any(|entry| {
                        entry.key == "target_comparison_real"
                            && entry
                                .value
                                .as_object()
                                .and_then(|value| value.get("kind"))
                                .and_then(JsonValue::as_str)
                                == Some("target_comparison")
                    })
                    && entries.iter().any(|entry| {
                        entry.key == "target_comparison_imag"
                            && entry
                                .value
                                .as_object()
                                .and_then(|value| value.get("kind"))
                                .and_then(JsonValue::as_str)
                                == Some("target_comparison")
                    })
        )));
    }

    #[test]
    fn task_panel_cursor_round_trips_downsample_level() {
        let cursor = parse_cursor(Some("42:3")).expect("cursor should parse");
        assert_eq!(
            cursor,
            TaskPanelCursor {
                snapshot_id: Some(42),
                downsample_level: 3,
            }
        );
    }

    #[test]
    fn compacted_full_updates_replace_large_append_history_with_downsampled_series() {
        let specs = vec![panel_spec(
            "history",
            "History",
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::Append,
        )];
        let history = (0..300)
            .map(|index| {
                vec![scalar_timeseries_panel(
                    "history",
                    vec![PlotPoint {
                        x: index as f64,
                        y: index as f64,
                        x_sampler_uptime_ms: None,
                        x_completed_samples_total: None,
                        y_min: None,
                        y_max: None,
                    }],
                )]
            })
            .collect::<Vec<_>>();

        let (updates, level) = compacted_full_updates(&specs, Vec::new(), history);
        let [update] = updates.as_slice() else {
            panic!("expected one update");
        };
        assert!(level > 0);
        assert!(matches!(update.mode, PanelUpdateMode::Replace));
        let PanelState::ScalarTimeseries { points, .. } = &update.panel else {
            panic!("expected scalar history panel");
        };
        assert!(points.len() <= DEFAULT_HISTORY_POINT_BUDGET);
        assert_eq!(points.last().map(|point| point.x), Some(299.0));
    }

    #[test]
    fn build_response_emits_non_null_cursor_for_append_history_without_persisted_snapshots() {
        let task = inherited_complex_sample_task();
        let run_task = run_task(task.clone());
        let accumulator = complex_observable();
        let source = TaskPanelSource::new(&task, Some(AccumulatorConfig::Complex));

        let response = source
            .build_response(
                "run:1:task:1".to_string(),
                TaskPanelCursor::default(),
                &run_task,
                &JsonValue::Object(Default::default()),
                None,
                None,
                None,
                Some(&accumulator),
                None,
                None,
                &[],
                &[],
            )
            .expect("build response");

        assert_eq!(response.cursor.as_deref(), Some("0:0"));
    }
}
