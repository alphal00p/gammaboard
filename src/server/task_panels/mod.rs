mod full_observable;
mod sample;

use crate::core::{EngineError, EvaluatorConfig, RunSpec, RunTask, RunTaskSpec};
use crate::evaluation::{ObservableState, SemanticObservableKind};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelUpdate, append_panel,
    merge_panel_state, panel_spec, replace_panel,
};
use crate::stores::{TaskOutputSnapshot, TaskStageSnapshot};
use serde_json::Value as JsonValue;

const DEFAULT_HISTORY_POINT_BUDGET: usize = 256;

type CurrentProjectorFn =
    dyn for<'a> Fn(&TaskPanelContext<'a>) -> Result<Option<PanelState>, EngineError> + Send + Sync;
type HistoryProjectorFn = dyn for<'a> Fn(&TaskPanelHistoryContext<'a>) -> Result<Option<PanelState>, EngineError>
    + Send
    + Sync;

pub struct TaskPanelProjector {
    spec: PanelSpec,
    current: Box<CurrentProjectorFn>,
    history: Box<HistoryProjectorFn>,
}

pub enum TaskPanelCurrentSource<'a> {
    Runtime(&'a ObservableState),
    StageSnapshot(&'a TaskStageSnapshot),
    Persisted(&'a JsonValue),
    Empty,
}

impl TaskPanelCurrentSource<'_> {
    pub fn observable(&self) -> Option<&ObservableState> {
        match self {
            Self::Runtime(observable) => Some(observable),
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
    pub run_spec: &'a RunSpec,
    pub source: TaskPanelCurrentSource<'a>,
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
    TaskPanelProjector {
        spec,
        current: Box::new(current),
        history: Box::new(history),
    }
}

fn project_current_panels(
    projectors: &[TaskPanelProjector],
    ctx: &TaskPanelContext<'_>,
) -> Result<Vec<PanelState>, EngineError> {
    projectors
        .iter()
        .filter_map(|projector| projector.current(ctx).transpose())
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

impl RunTaskSpec {
    fn panel_projectors(&self, run_spec: &RunSpec) -> Vec<TaskPanelProjector> {
        match self {
            Self::Pause => vec![panel_projector(
                panel_spec(
                    "pause_state",
                    "Pause State",
                    PanelKind::Text,
                    PanelHistoryMode::None,
                ),
                |_ctx| {
                    Ok(Some(PanelState::Text {
                        panel_id: "pause_state".to_string(),
                        text: "Task is paused".to_string(),
                    }))
                },
                |_ctx| Ok(None),
            )],
            Self::Sample { .. } => sample::projectors(run_spec),
            Self::Image {
                geometry, display, ..
            } => full_observable::image_projectors(geometry.clone(), *display),
            Self::PlotLine {
                geometry, display, ..
            } => full_observable::line_projectors(geometry.clone(), *display, run_spec),
        }
    }
}

impl TaskPanelSource {
    pub fn new(task_spec: &RunTaskSpec, run_spec: &RunSpec) -> Self {
        Self {
            projectors: task_spec.panel_projectors(run_spec),
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

    pub fn current_panels(
        &self,
        task: &RunTask,
        run_spec: &RunSpec,
        current_observable: Option<&ObservableState>,
        latest_stage_snapshot: Option<&TaskStageSnapshot>,
        latest_persisted_snapshot: Option<&TaskOutputSnapshot>,
    ) -> Result<Vec<PanelState>, EngineError> {
        project_current_panels(
            &self.projectors,
            &TaskPanelContext {
                task,
                run_spec,
                source: resolve_current_source(
                    task,
                    current_observable,
                    latest_stage_snapshot,
                    latest_persisted_snapshot,
                ),
            },
        )
    }

    pub fn build_response(
        &self,
        source_id: String,
        requested_cursor: TaskPanelCursor,
        task: &RunTask,
        run_spec: &RunSpec,
        current_observable: Option<&ObservableState>,
        latest_stage_snapshot: Option<&TaskStageSnapshot>,
        latest_persisted_snapshot: Option<&TaskOutputSnapshot>,
        full_history_snapshots: &[TaskOutputSnapshot],
        delta_history_snapshots: &[TaskOutputSnapshot],
    ) -> Result<PanelResponse, EngineError> {
        let panels = self.panel_specs();
        let current_panels = self.current_panels(
            task,
            run_spec,
            current_observable,
            latest_stage_snapshot,
            latest_persisted_snapshot,
        )?;
        let full_history_panels = full_history_snapshots
            .iter()
            .rev()
            .map(|snapshot| {
                project_history_panels(&self.projectors, &TaskPanelHistoryContext { snapshot })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let compacted_full_updates =
            compacted_full_updates(&panels, current_panels.clone(), full_history_panels);
        let target_level = target_downsample_level(&panels, &compacted_full_updates);
        let cursor_snapshot_id = latest_persisted_snapshot
            .and_then(|snapshot| snapshot.id.parse::<i64>().ok())
            .or(requested_cursor.snapshot_id);
        let cursor = format_cursor(TaskPanelCursor {
            snapshot_id: cursor_snapshot_id,
            downsample_level: target_level,
        });
        let updates = if requested_cursor.snapshot_id.is_some()
            && requested_cursor.downsample_level == target_level
        {
            let delta_history_panels = delta_history_snapshots
                .iter()
                .rev()
                .map(|snapshot| {
                    project_history_panels(&self.projectors, &TaskPanelHistoryContext { snapshot })
                })
                .collect::<Result<Vec<_>, _>>()?;
            incremental_updates(&panels, current_panels, delta_history_panels)
        } else {
            compacted_full_updates
        };

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
    current_observable: Option<&'a ObservableState>,
    latest_stage_snapshot: Option<&'a TaskStageSnapshot>,
    latest_persisted_snapshot: Option<&'a TaskOutputSnapshot>,
) -> TaskPanelCurrentSource<'a> {
    if matches!(task.state, crate::core::RunTaskState::Active) {
        if let Some(observable) = current_observable {
            return TaskPanelCurrentSource::Runtime(observable);
        }
    } else if let Some(snapshot) = latest_stage_snapshot {
        return TaskPanelCurrentSource::StageSnapshot(snapshot);
    }
    if let Some(snapshot) = latest_persisted_snapshot {
        return TaskPanelCurrentSource::Persisted(&snapshot.persisted_output);
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
) -> Vec<PanelUpdate> {
    let mut updates = full_updates(specs, current_panels, history_panels);
    let level = target_downsample_level(specs, &updates);
    if level == 0 {
        return updates;
    }

    for update in &mut updates {
        if history_mode_for(specs, update.panel.panel_id()) == PanelHistoryMode::Append {
            downsample_panel_state(&mut update.panel, level);
        }
    }
    updates
}

fn incremental_updates(
    specs: &[PanelSpec],
    current_panels: Vec<PanelState>,
    history_panels: Vec<Vec<PanelState>>,
) -> Vec<PanelUpdate> {
    let mut updates = current_panels
        .into_iter()
        .filter(|panel| history_mode_for(specs, panel.panel_id()) == PanelHistoryMode::None)
        .map(replace_panel)
        .collect::<Vec<_>>();

    let mut delta_by_id = std::collections::BTreeMap::new();
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

impl EvaluatorConfig {
    pub fn observable_kind(&self) -> SemanticObservableKind {
        match self {
            Self::Gammaloop { params } => params.observable_kind,
            Self::SinEvaluator { .. } => SemanticObservableKind::Scalar,
            Self::SincEvaluator { .. } => SemanticObservableKind::Complex,
            Self::Unit { params } => params.observable_kind,
            Self::Symbolica { .. } => SemanticObservableKind::Scalar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        LineDisplayMode, ObservableConfig, ParametrizationConfig, RunTaskState,
        SamplerAggregatorConfig,
    };
    use crate::evaluation::{
        ComplexValue, FullComplexObservableState, PointSpec, UnitEvaluatorParams,
    };
    use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
    use crate::sampling::{IdentityParametrizationParams, RasterLineSamplerParams};
    use crate::server::panels::{PanelUpdateMode, PlotPoint, scalar_timeseries_panel};
    use chrono::Utc;

    fn complex_run_spec() -> RunSpec {
        RunSpec {
            run_id: 1,
            point_spec: PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            evaluator: EvaluatorConfig::Unit {
                params: UnitEvaluatorParams {
                    observable_kind: SemanticObservableKind::Complex,
                    ..UnitEvaluatorParams::default()
                },
            },
            observable: ObservableConfig::FullComplex,
            sampler_aggregator: SamplerAggregatorConfig::RasterLine {
                params: RasterLineSamplerParams {
                    geometry: line_geometry(),
                },
            },
            parametrization: ParametrizationConfig::Identity {
                params: IdentityParametrizationParams::default(),
            },
            evaluator_runner_params: EvaluatorRunnerParams {
                performance_snapshot_interval_ms: 1000,
            },
            sampler_aggregator_runner_params: SamplerAggregatorRunnerParams {
                performance_snapshot_interval_ms: 1000,
                target_batch_eval_ms: 100.0,
                target_queue_remaining: 0.5,
                max_batch_size: 16,
                max_queue_size: 16,
                max_batches_per_tick: 4,
                completed_batch_fetch_limit: 16,
            },
        }
    }

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
            observable: crate::core::PlotObservableKind::Complex,
            display,
            start_from: None,
        }
    }

    fn run_task(task: RunTaskSpec) -> RunTask {
        RunTask {
            id: 1,
            run_id: 1,
            sequence_nr: 1,
            task,
            spawned_from_run_id: None,
            spawned_from_task_id: None,
            state: RunTaskState::Active,
            nr_produced_samples: 3,
            nr_completed_samples: 3,
            failure_reason: None,
            started_at: None,
            completed_at: None,
            failed_at: None,
            created_at: Utc::now(),
        }
    }

    fn complex_observable() -> ObservableState {
        ObservableState::FullComplex(FullComplexObservableState {
            values: vec![
                ComplexValue { re: 1.0, im: -1.0 },
                ComplexValue { re: 2.0, im: -2.0 },
                ComplexValue { re: 3.0, im: -3.0 },
            ],
        })
    }

    fn current_panels(
        task_spec: &RunTaskSpec,
        task: &RunTask,
        observable: &ObservableState,
        run_spec: &RunSpec,
    ) -> Vec<PanelState> {
        TaskPanelSource::new(task_spec, run_spec)
            .current_panels(task, run_spec, Some(observable), None, None)
            .unwrap()
    }

    #[test]
    fn complex_line_auto_uses_multi_timeseries_components_panel() {
        let run_spec = complex_run_spec();
        let task = plot_task(LineDisplayMode::Auto);
        let descriptors = TaskPanelSource::new(&task, &run_spec).panel_specs();
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
        let observable = complex_observable();
        let current = current_panels(&task, &run_task, &observable, &run_spec);
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
        let run_spec = complex_run_spec();
        let task = plot_task(LineDisplayMode::ScalarCurve);
        let descriptors = TaskPanelSource::new(&task, &run_spec).panel_specs();
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
        let observable = complex_observable();
        let current = current_panels(&task, &run_task, &observable, &run_spec);
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
                        y_min: None,
                        y_max: None,
                    }],
                )]
            })
            .collect::<Vec<_>>();

        let updates = compacted_full_updates(&specs, Vec::new(), history);
        let [update] = updates.as_slice() else {
            panic!("expected one update");
        };
        assert!(matches!(update.mode, PanelUpdateMode::Replace));
        let PanelState::ScalarTimeseries { points, .. } = &update.panel else {
            panic!("expected scalar history panel");
        };
        assert!(points.len() <= DEFAULT_HISTORY_POINT_BUDGET);
        assert_eq!(points.last().map(|point| point.x), Some(299.0));
    }
}
