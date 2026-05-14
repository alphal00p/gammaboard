mod full_accumulator;
mod pdf_adaptation;
mod sample;

use crate::core::{
    AccumulatorConfig, AccumulatorSourceSpec, BatchTransformConfig, EngineError, RunTask,
    RunTaskSpec, SampleErrorProjection, SampleStopCondition, SamplerAggregatorConfig,
    SamplerAggregatorSourceSpec,
};
use crate::evaluation::AccumulatorState;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelUpdate, PanelWidth,
    append_panel, key_value, key_value_panel, merge_panel_state, panel_spec, replace_panel,
    with_panel_width,
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
    ) -> Result<Vec<TaskPanelProjector>, EngineError> {
        let mut projectors = vec![task_summary_projector(effective_accumulator_config.clone())];
        projectors.extend(match self {
            Self::SetAccumulator { .. } => Vec::new(),
            Self::Sample { .. } => effective_accumulator_config
                .map(sample::projectors)
                .unwrap_or_default(),
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
        Ok(projectors)
    }
}

fn task_summary_projector(
    effective_accumulator_config: Option<AccumulatorConfig>,
) -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "task_summary",
                "Task Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        move |ctx| {
            Ok(Some(key_value_panel(
                "task_summary",
                build_task_summary_entries(ctx, effective_accumulator_config.clone()),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn build_task_summary_entries(
    ctx: &TaskPanelContext<'_>,
    effective_accumulator_config: Option<AccumulatorConfig>,
) -> Vec<crate::server::panels::KeyValueEntry> {
    let task = ctx.task;
    let mut entries = vec![
        key_value("state", "State", task.state.as_str()),
        key_value("kind", "Kind", task.task.kind_str()),
    ];

    if let Some(reason) = task.failure_reason.as_deref()
        && !reason.is_empty()
    {
        entries.push(key_value("failure", "Failure", reason));
    }

    match &task.task {
        RunTaskSpec::SetAccumulator { accumulator } => {
            entries.push(key_value(
                "accumulator",
                "Accumulator",
                accumulator_label(accumulator),
            ));
        }
        RunTaskSpec::Sample {
            stop_condition,
            sampler_aggregator,
            accumulator,
            batch_transforms,
            ..
        } => {
            entries.push(key_value(
                "samples",
                "Samples",
                progress_label(task.nr_completed_samples, task.task.nr_expected_samples()),
            ));
            entries.push(key_value(
                "stop_condition",
                "Stop",
                sample_stop_condition_label(stop_condition),
            ));
            entries.push(key_value(
                "sampler",
                "Sampler",
                sampler_source_label(sampler_aggregator.as_ref()),
            ));
            entries.push(key_value(
                "accumulator",
                "Accumulator",
                sample_accumulator_label(accumulator.as_ref(), effective_accumulator_config),
            ));
            if let Some(rate) = ctx.completed_samples_per_second {
                entries.push(key_value("rate", "Rate", format!("{rate:.2} samples/s")));
            }
            if let Some(eta_seconds) = ctx.smoothed_eta_seconds {
                entries.push(key_value(
                    "eta",
                    "ETA",
                    format_duration_seconds(eta_seconds),
                ));
            }
            if let Some(label) = batch_transforms_label(batch_transforms.as_deref()) {
                entries.push(key_value("batch_transforms", "Transforms", label));
            }
        }
        RunTaskSpec::Image {
            geometry,
            accumulator,
            batch_transforms,
            ..
        } => {
            entries.push(key_value(
                "points",
                "Points",
                progress_label(task.nr_completed_samples, Some(geometry.nr_points() as i64)),
            ));
            entries.push(key_value(
                "geometry",
                "Geometry",
                format!(
                    "{} x {}",
                    geometry.u_linspace.count, geometry.v_linspace.count
                ),
            ));
            entries.push(key_value(
                "accumulator",
                "Accumulator",
                plot_accumulator_label(*accumulator),
            ));
            if let Some(label) = batch_transforms_label(batch_transforms.as_deref()) {
                entries.push(key_value("batch_transforms", "Transforms", label));
            }
        }
        RunTaskSpec::PdfAdaptationImage {
            geometry,
            sampler_aggregator,
            batch_transforms,
        } => {
            entries.push(key_value(
                "points",
                "Points",
                progress_label(task.nr_completed_samples, Some(geometry.nr_points() as i64)),
            ));
            entries.push(key_value(
                "geometry",
                "Geometry",
                format!(
                    "{} x {}",
                    geometry.u_linspace.count, geometry.v_linspace.count
                ),
            ));
            entries.push(key_value(
                "sampler_source",
                "Sampler Source",
                source_ref_label(task.task.sample_sampler_source()),
            ));
            if let Some(label) = sampler_source_override_label(sampler_aggregator.as_ref()) {
                entries.push(key_value("sampler", "Sampler", label));
            }
            if let Some(label) = batch_transforms_label(batch_transforms.as_deref()) {
                entries.push(key_value("batch_transforms", "Transforms", label));
            }
        }
        RunTaskSpec::PdfAdaptationPlotLine {
            geometry,
            sampler_aggregator,
            batch_transforms,
        } => {
            entries.push(key_value(
                "points",
                "Points",
                progress_label(task.nr_completed_samples, Some(geometry.nr_points() as i64)),
            ));
            entries.push(key_value("geometry", "Geometry", geometry.linspace.count));
            entries.push(key_value(
                "sampler_source",
                "Sampler Source",
                source_ref_label(task.task.sample_sampler_source()),
            ));
            if let Some(label) = sampler_source_override_label(sampler_aggregator.as_ref()) {
                entries.push(key_value("sampler", "Sampler", label));
            }
            if let Some(label) = batch_transforms_label(batch_transforms.as_deref()) {
                entries.push(key_value("batch_transforms", "Transforms", label));
            }
        }
        RunTaskSpec::PlotLine {
            geometry,
            accumulator,
            batch_transforms,
            ..
        } => {
            entries.push(key_value(
                "points",
                "Points",
                progress_label(task.nr_completed_samples, Some(geometry.nr_points() as i64)),
            ));
            entries.push(key_value("geometry", "Geometry", geometry.linspace.count));
            entries.push(key_value(
                "accumulator",
                "Accumulator",
                plot_accumulator_label(*accumulator),
            ));
            if let Some(label) = batch_transforms_label(batch_transforms.as_deref()) {
                entries.push(key_value("batch_transforms", "Transforms", label));
            }
        }
    }

    entries
}

fn format_duration_seconds(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as i64;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn progress_label(current: i64, total: Option<i64>) -> String {
    match total {
        Some(total) => format!("{current} / {total}"),
        None => current.to_string(),
    }
}

fn sample_stop_condition_label(stop: &SampleStopCondition) -> String {
    let mut parts = Vec::new();
    if let Some(max_samples) = stop.max_samples {
        parts.push(format!("max_samples={max_samples}"));
    }
    if let Some(value) = stop.absolute_error {
        parts.push(format!(
            "abs_error<={value}{}",
            projection_suffix(stop.projection)
        ));
    }
    if let Some(value) = stop.relative_error {
        parts.push(format!(
            "rel_error<={value}{}",
            projection_suffix(stop.projection)
        ));
    }
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(", ")
    }
}

fn projection_suffix(projection: Option<SampleErrorProjection>) -> &'static str {
    match projection {
        Some(SampleErrorProjection::Real) => " (real)",
        Some(SampleErrorProjection::Imag) => " (imag)",
        Some(SampleErrorProjection::Abs) => " (abs)",
        None => "",
    }
}

fn sampler_source_label(source: Option<&SamplerAggregatorSourceSpec>) -> String {
    match source {
        Some(SamplerAggregatorSourceSpec::Config { config }) => sampler_config_label(config).into(),
        Some(SamplerAggregatorSourceSpec::FromName { from_name }) => format!("from {from_name}"),
        Some(SamplerAggregatorSourceSpec::Latest(_)) | None => "latest".to_string(),
    }
}

fn sample_accumulator_label(
    source: Option<&AccumulatorSourceSpec>,
    effective: Option<AccumulatorConfig>,
) -> String {
    match source {
        Some(AccumulatorSourceSpec::Config { config }) => accumulator_label(config).to_string(),
        Some(AccumulatorSourceSpec::FromName { from_name }) => format!("from {from_name}"),
        Some(AccumulatorSourceSpec::Latest(_)) | None => effective
            .as_ref()
            .map(accumulator_label)
            .unwrap_or("latest")
            .to_string(),
    }
}

fn batch_transforms_label(batch_transforms: Option<&[BatchTransformConfig]>) -> Option<String> {
    let batch_transforms = batch_transforms?;
    if batch_transforms.is_empty() {
        Some("cleared".to_string())
    } else {
        Some(
            batch_transforms
                .iter()
                .map(batch_transform_label)
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

fn batch_transform_label(config: &BatchTransformConfig) -> &'static str {
    match config {
        BatchTransformConfig::UnitBall { .. } => "unit_ball",
        BatchTransformConfig::Spherical { .. } => "spherical",
    }
}

fn source_ref_label(source: Option<crate::core::SourceRefSpec>) -> String {
    match source {
        Some(crate::core::SourceRefSpec::Latest) => "latest".to_string(),
        Some(crate::core::SourceRefSpec::FromName(name)) => format!("from {name}"),
        None => "-".to_string(),
    }
}

fn sampler_source_override_label(source: Option<&SamplerAggregatorSourceSpec>) -> Option<String> {
    match source {
        Some(SamplerAggregatorSourceSpec::Config { config }) => {
            Some(sampler_config_label(config).to_string())
        }
        _ => None,
    }
}

fn sampler_config_label(config: &SamplerAggregatorConfig) -> &'static str {
    match config {
        SamplerAggregatorConfig::NaiveMonteCarlo { .. } => "naive_monte_carlo",
        SamplerAggregatorConfig::RasterPlane { .. } => "raster_plane",
        SamplerAggregatorConfig::RasterLine { .. } => "raster_line",
        SamplerAggregatorConfig::PdfAdaptationRasterPlane { .. } => "pdf_adaptation_raster_plane",
        SamplerAggregatorConfig::PdfAdaptationRasterLine { .. } => "pdf_adaptation_raster_line",
        SamplerAggregatorConfig::HavanaTraining { .. } => "havana_training",
        SamplerAggregatorConfig::HavanaInference { .. } => "havana_inference",
        SamplerAggregatorConfig::ProcessSampler { .. } => "process_sampler",
    }
}

fn accumulator_label(config: &AccumulatorConfig) -> &'static str {
    config.kind_str()
}

fn plot_accumulator_label(kind: crate::core::PlotAccumulatorKind) -> &'static str {
    match kind {
        crate::core::PlotAccumulatorKind::Scalar => "scalar",
        crate::core::PlotAccumulatorKind::Complex => "complex",
    }
}

impl TaskPanelSource {
    pub fn new(
        task_spec: &RunTaskSpec,
        effective_accumulator_config: Option<AccumulatorConfig>,
    ) -> Result<Self, EngineError> {
        Ok(Self {
            projectors: task_spec.panel_projectors(effective_accumulator_config)?,
        })
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
    use crate::core::{AccumulatorConfig, RunTaskInput, RunTaskState, canonical_task_toml};
    use crate::evaluation::{AccumulatorState, FullVectorAccumulatorState};
    use crate::server::panels::{
        PanelKind, PanelUpdateMode, PlotPoint, panel_spec, scalar_timeseries_panel,
    };
    use chrono::Utc;

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
        AccumulatorState::FullVector(FullVectorAccumulatorState {
            components: vec!["real".to_string(), "imag".to_string()],
            values_row_major: vec![1.0, -1.0, 2.0, -2.0, 3.0, -3.0],
            invalid_entries: vec![],
        })
    }

    #[test]
    fn gammaloop_sample_uses_imag_panel() {
        let task = inherited_complex_sample_task();
        let descriptors = TaskPanelSource::new(&task, Some(AccumulatorConfig::Gammaloop))
            .expect("panel source")
            .panel_specs();
        assert!(
            descriptors
                .iter()
                .any(|panel| panel.panel_id == "imag_estimate_history")
        );
    }

    #[test]
    fn pending_sample_without_effective_accumulator_still_has_summary_panel() {
        let task = inherited_complex_sample_task();
        let descriptors = TaskPanelSource::new(&task, None)
            .expect("panel source")
            .panel_specs();

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].panel_id, "task_summary");
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
        let source = TaskPanelSource::new(
            &task,
            Some(AccumulatorConfig::FullVector {
                components: vec!["real".to_string(), "imag".to_string()],
            }),
        )
        .expect("panel source");

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
