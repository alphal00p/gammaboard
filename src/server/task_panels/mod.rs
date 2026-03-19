mod full_observable;
mod sample;

use crate::core::{EngineError, EvaluatorConfig, RunSpec, RunTask, RunTaskSpec};
use crate::evaluation::{ObservableState, SemanticObservableKind};
use crate::server::panels::{PanelHistoryMode, PanelKind, PanelSpec, PanelState, panel_spec};
use crate::stores::{TaskOutputSnapshot, TaskStageSnapshot};
use serde_json::Value as JsonValue;

impl RunTaskSpec {
    pub fn panel_specs(&self, run_spec: &RunSpec) -> Vec<PanelSpec> {
        match self {
            Self::Pause => vec![panel_spec(
                "pause_state",
                "Pause State",
                PanelKind::Text,
                PanelHistoryMode::None,
            )],
            Self::Sample { .. } => sample::panel_specs(run_spec),
            Self::Image { .. } => full_observable::image_panel_specs(),
            Self::PlotLine { display, .. } => full_observable::line_panel_specs(*display, run_spec),
        }
    }

    pub fn build_current_panels(
        &self,
        task: &RunTask,
        observable: Option<&ObservableState>,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        match self {
            Self::Pause => Ok(vec![PanelState::Text {
                panel_id: "pause_state".to_string(),
                text: "Task is paused".to_string(),
            }]),
            Self::Sample { .. } => sample::build_current_panels(task, observable, run_spec),
            Self::Image {
                geometry, display, ..
            } => full_observable::build_image_current_panels(task, observable, geometry, *display),
            Self::PlotLine {
                geometry, display, ..
            } => full_observable::build_line_current_panels(
                task, observable, geometry, *display, run_spec,
            ),
        }
    }

    pub fn build_history_panels(
        &self,
        snapshot: &TaskOutputSnapshot,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        match self {
            Self::Pause => Ok(Vec::new()),
            Self::Sample { nr_samples, .. } => sample::build_panels_from_persisted(
                &snapshot.persisted_output,
                nr_samples.map(|value| value as f64),
                run_spec,
            ),
            Self::Image { geometry, .. } => full_observable::build_image_panels_from_persisted(
                &snapshot.persisted_output,
                geometry,
            ),
            Self::PlotLine { geometry, .. } => full_observable::build_line_panels_from_persisted(
                &snapshot.persisted_output,
                geometry,
            ),
        }
    }

    pub fn build_current_panels_from_persisted(
        &self,
        task: &RunTask,
        persisted: &JsonValue,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        match self {
            Self::Pause => self.build_current_panels(task, None, run_spec),
            Self::Sample { nr_samples, .. } => sample::build_panels_from_persisted(
                persisted,
                nr_samples.map(|value| value as f64),
                run_spec,
            ),
            Self::Image { geometry, .. } => {
                full_observable::build_image_panels_from_persisted(persisted, geometry)
            }
            Self::PlotLine { geometry, .. } => {
                full_observable::build_line_panels_from_persisted(persisted, geometry)
            }
        }
    }

    pub fn build_current_panels_from_stage_snapshot(
        &self,
        task: &RunTask,
        snapshot: &TaskStageSnapshot,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        self.build_current_panels(task, Some(&snapshot.observable_state), run_spec)
    }
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

    #[test]
    fn complex_line_auto_uses_multi_timeseries_components_panel() {
        let run_spec = complex_run_spec();
        let task = plot_task(LineDisplayMode::Auto);
        let descriptors = task.panel_specs(&run_spec);
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

        let current = task
            .build_current_panels(
                &run_task(task.clone()),
                Some(&complex_observable()),
                &run_spec,
            )
            .unwrap();
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
        let descriptors = task.panel_specs(&run_spec);
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

        let current = task
            .build_current_panels(
                &run_task(task.clone()),
                Some(&complex_observable()),
                &run_spec,
            )
            .unwrap();
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
}
