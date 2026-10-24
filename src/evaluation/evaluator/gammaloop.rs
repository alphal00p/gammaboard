#[cfg(test)]
use super::gammaloop_config::GammaLoopPreprocessing;
pub use super::gammaloop_config::{GammaLoopParams, TrainingProjection};
#[cfg(test)]
use crate::AccumulatorState;
use crate::core::EngineResultExt;
use gammalooprs::integrands::process::EvaluationTarget;
use std::{any::Any, ops::ControlFlow, panic::AssertUnwindSafe};

use gammaloop_api::{
    CLISettings,
    state::{CommandHistory, RunHistory, State},
};
use gammalooprs::graph::GroupId;
use gammalooprs::initialisation::initialise;
use gammalooprs::integrands::HasIntegrand;
use gammalooprs::integrands::evaluation::EvaluationResult;
use gammalooprs::integrands::process::{
    GaussianReferenceFunction, MomentumSpaceEvaluationInput, ProcessIntegrand,
    SamplingChannelInspection,
};
use gammalooprs::model::Model;
use gammalooprs::settings::RuntimeSettings;
use gammalooprs::settings::runtime::{
    DiscreteGraphSamplingType, Precision, SamplingSettings, SamplingSettingsParser,
    StabilityLevelSetting,
};
use gammalooprs::utils::F;
use serde::Serialize;
use serde_json::{Value as JsonValue, json};
use symbolica::numerical_integration::Sample;

use crate::{
    Batch, BatchResult, Domain, DomainBranch, EngineError,
    core::{
        AccumulatorConfig, AccumulatorMomentConfig,
        TrainingProjection as AccumulatorTrainingProjection,
    },
    evaluation::{EvalBatchOptions, Evaluator, GammaLoopAccumulatorState, VectorAccumulatorState},
    resources::resolve_resource_path,
};

pub struct GammaLoopEvaluator {
    integrand: ProcessIntegrand,
    pristine_integrand: ProcessIntegrand,
    model: Model,
    metadata: GammaLoopMetadata,
    momentum_space: bool,
    training_projection: TrainingProjection,
    graph_groups: Option<Vec<usize>>,
    domain: Domain,
    reference_gaussian: Option<GaussianReferenceFunction>,
}

#[derive(Debug, Clone, Serialize)]
struct GammaLoopMetadata {
    kind: &'static str,
    state_folder: String,
    process_id: usize,
    integrand_name: String,
    momentum_space: bool,
    coordinate_space: &'static str,
    domain_axes: Vec<&'static str>,
    graph_groups: Option<Vec<usize>>,
    sampling_channels: Vec<SamplingChannelInspection>,
    sampling: SamplingSettingsParser,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_gaussian: Option<GaussianReferenceFunction>,
}

impl TrainingProjection {
    fn project(self, value: num::Complex<f64>) -> f64 {
        match self {
            Self::Real => value.re,
            Self::Imag => value.im,
            Self::Abs => value.norm(),
            Self::AbsSq => value.norm_sqr(),
        }
    }
}

impl GammaLoopEvaluator {
    fn load_integrand_and_model(
        mut params: GammaLoopParams,
    ) -> Result<(ProcessIntegrand, Model, GammaLoopMetadata), EngineError> {
        params.state_folder = resolve_resource_path(&params.state_folder).map_err(|err| {
            EngineError::build(format!(
                "failed to resolve gammaloop state_folder '{}': {err}",
                params.state_folder.display()
            ))
        })?;
        crate::activate_symbolica_oem_license().build_err()?;
        initialise()
            .map_err(|err| EngineError::build(format!("gammaloop initialization: {err}")))?;
        if params.momentum_space && params.reference_gaussian.is_some() {
            return Err(EngineError::invalid_input(
                "reference_gaussian requires x-space evaluation to exercise sampling maps",
            ));
        }
        let selection = if params.preprocessing.commands.is_empty() && params.process_id.is_none() {
            params.integrand_name.as_ref().map(|name| {
                gammalooprs::processes::ProcessLoadSelection {
                    integrand_selectors: vec![
                        gammalooprs::processes::ProcessLoadIntegrandSelector::Name(name.clone()),
                    ],
                    ..Default::default()
                }
            })
        } else {
            None
        };
        let mut state =
            State::load_with_selection(params.state_folder.clone(), None, None, selection.as_ref())
                .map_err(|err| {
                    EngineError::build(format!(
                        "failed to load state from {}: {err:#}",
                        params.state_folder.display()
                    ))
                })?;
        Self::run_preprocessing(&params, &mut state)?;
        state
            .activate_loaded_integrand_backends(false)
            .build_err()?;

        let (process_id, integrand_name) = state
            .find_integrand_ref(params.process_id.as_ref(), params.integrand_name.as_ref())
            .map_err(|err| EngineError::build(format!("failed to find integrand: {err}")))?;

        let model = state
            .resolve_model_for_integrand(process_id, &integrand_name)
            .build_err()?;
        let integrand = state
            .process_list
            .get_integrand_mut(process_id, integrand_name.clone())
            .build_err()?;
        let selected_names = integrand.get_settings().sampling.selected_graph_names();
        let mut integrand = integrand
            .clone_with_selected_graph_groups(selected_names)
            .build_err()?;
        if params.use_f128 {
            let levels = &mut integrand.get_mut_settings().stability.levels;
            levels.retain(|level| level.precision != Precision::Double);
            if levels.is_empty() {
                levels.push(StabilityLevelSetting::default_quad());
            }
        }
        let reference_gaussian = params
            .reference_gaussian
            .as_ref()
            .map(|reference| reference.for_integrand(&integrand))
            .transpose()
            .build_err()?;
        let sampling_channels =
            Self::sampling_channels(&integrand, params.graph_groups.as_deref())?;
        let momentum_space = params.momentum_space;
        let metadata = GammaLoopMetadata {
            kind: "gammaloop",
            state_folder: params.state_folder.to_string_lossy().into_owned(),
            process_id,
            integrand_name,
            momentum_space,
            coordinate_space: if momentum_space {
                "momentum_space"
            } else {
                "x_space"
            },
            domain_axes: Self::domain_axes(&integrand),
            graph_groups: params.graph_groups,
            sampling_channels,
            sampling: integrand.get_settings().sampling.as_parser(),
            reference_gaussian,
        };
        Ok((integrand, model, metadata))
    }

    fn sampling_channels(
        integrand: &ProcessIntegrand,
        selected_groups: Option<&[usize]>,
    ) -> Result<Vec<SamplingChannelInspection>, EngineError> {
        let Some(parameters) = integrand
            .get_settings()
            .sampling
            .get_parameterization_settings()
        else {
            return Ok(Vec::new());
        };
        integrand
            .graph_group_master_names()
            .into_iter()
            .enumerate()
            .filter(|(group, _)| selected_groups.is_none_or(|selected| selected.contains(group)))
            .map(|(_, name)| {
                let graph_id = integrand
                    .find_graph_id_by_name(name)
                    .ok_or_else(|| EngineError::build(format!("missing master graph '{name}'")))?;
                let setup = match integrand {
                    ProcessIntegrand::Amplitude(amplitude) => {
                        &amplitude.data.graph_terms[graph_id].multi_channeling_setup
                    }
                    ProcessIntegrand::CrossSection(cross_section) => {
                        &cross_section.data.graph_terms[graph_id].multi_channeling_setup
                    }
                };
                Ok(setup
                    .canonical_sampling_catalogue(name, &parameters)
                    .build_err()?
                    .inspection())
            })
            .collect()
    }

    pub fn resolve_domain_from_params(params: GammaLoopParams) -> Result<Domain, EngineError> {
        match std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<Domain, EngineError> {
            let graph_groups = params.graph_groups.clone();
            let (integrand, _model, metadata) = Self::load_integrand_and_model(params)?;
            Self::build_domain(&integrand, metadata.momentum_space, graph_groups.as_deref())
        })) {
            Ok(result) => result,
            Err(payload) => Err(EngineError::build(format!(
                "gammaloop domain resolution panicked: {}",
                Self::panic_message(payload)
            ))),
        }
    }

    fn build_domain(
        integrand: &ProcessIntegrand,
        momentum_space: bool,
        selected_graph_groups: Option<&[usize]>,
    ) -> Result<Domain, EngineError> {
        fn continuous_leaf(
            integrand: &ProcessIntegrand,
            momentum_space: bool,
            discrete_selection: &[usize],
        ) -> Result<Domain, EngineError> {
            let dims = if momentum_space {
                let dims = integrand.get_n_dim();
                if !dims.is_multiple_of(3) {
                    return Err(EngineError::build(format!(
                        "gammaloop momentum-space domain dimension must be divisible by 3, got {dims}"
                    )));
                }
                dims
            } else {
                integrand
                    .expected_x_space_dimension(discrete_selection)
                    .map_err(|err| {
                        EngineError::build(format!(
                            "failed to infer x-space dimensions for selection {:?}: {err}",
                            discrete_selection
                        ))
                    })?
            };
            Ok(Domain::continuous(dims))
        }

        fn build_group_branch(
            integrand: &ProcessIntegrand,
            momentum_space: bool,
            group_idx: usize,
        ) -> Result<Domain, EngineError> {
            let settings = integrand.get_settings();
            let SamplingSettings::DiscreteGraphs(discrete_settings) = &settings.sampling else {
                return continuous_leaf(integrand, momentum_space, &[]);
            };

            let group_id = GroupId::from(group_idx);
            let base_selection = [group_idx];

            if discrete_settings.sample_orientations {
                let orientation_count =
                    integrand.group_orientation_count(group_id).ok_or_else(|| {
                        EngineError::build(format!(
                            "failed to infer orientation count for graph group {group_idx}"
                        ))
                    })?;
                let orientation_branches = (0..orientation_count)
                    .map(|orientation_idx| {
                        let mut selection = vec![group_idx, orientation_idx];
                        let domain = match &discrete_settings.sampling_type {
                            DiscreteGraphSamplingType::SamplingMultiChanneling(_) => {
                                let channel_count =
                                    integrand.group_channel_count(group_id).ok_or_else(|| {
                                        EngineError::build(format!(
                                            "failed to infer channel count for graph group {group_idx}"
                                        ))
                                    })?;
                                let channel_branches = (0..channel_count)
                                    .map(|channel_idx| {
                                        selection.push(channel_idx);
                                        let leaf = continuous_leaf(
                                            integrand,
                                            momentum_space,
                                            selection.as_slice(),
                                        )?;
                                        selection.pop();
                                        Ok(DomainBranch::new(channel_idx, leaf))
                                    })
                                    .collect::<Result<Vec<_>, EngineError>>()?;
                                Domain::discrete(Some("channel".to_string()), channel_branches)
                            }
                            _ => continuous_leaf(integrand, momentum_space, selection.as_slice())?,
                        };
                        Ok(DomainBranch::new(orientation_idx, domain))
                    })
                    .collect::<Result<Vec<_>, EngineError>>()?;
                return Ok(Domain::discrete(
                    Some("orientation".to_string()),
                    orientation_branches,
                ));
            }

            match &discrete_settings.sampling_type {
                DiscreteGraphSamplingType::SamplingMultiChanneling(_) => {
                    let channel_count =
                        integrand.group_channel_count(group_id).ok_or_else(|| {
                            EngineError::build(format!(
                                "failed to infer channel count for graph group {group_idx}"
                            ))
                        })?;
                    let channel_branches = (0..channel_count)
                        .map(|channel_idx| {
                            let selection = [group_idx, channel_idx];
                            let leaf =
                                continuous_leaf(integrand, momentum_space, selection.as_slice())?;
                            Ok(DomainBranch::new(channel_idx, leaf))
                        })
                        .collect::<Result<Vec<_>, EngineError>>()?;
                    Ok(Domain::discrete(
                        Some("channel".to_string()),
                        channel_branches,
                    ))
                }
                _ => continuous_leaf(integrand, momentum_space, &base_selection),
            }
        }

        match integrand.get_settings().sampling.clone() {
            SamplingSettings::Default(_) | SamplingSettings::MultiChanneling(_) => {
                if selected_graph_groups.is_some() {
                    return Err(EngineError::build(
                        "gammaloop graph_groups requires discrete graph sampling",
                    ));
                }
                continuous_leaf(integrand, momentum_space, &[])
            }
            SamplingSettings::DiscreteGraphs(_) => {
                let group_count = integrand.graph_group_master_names().len();
                let group_indices = selected_graph_groups
                    .map(|indices| indices.to_vec())
                    .unwrap_or_else(|| (0..group_count).collect());
                if group_indices.is_empty() {
                    return Err(EngineError::build(
                        "gammaloop graph_groups must select at least one graph group",
                    ));
                }
                let mut group_branches = Vec::with_capacity(group_indices.len());
                for (local_group_idx, group_idx) in group_indices.iter().copied().enumerate() {
                    if group_idx >= group_count {
                        return Err(EngineError::build(format!(
                            "gammaloop graph_groups contains {group_idx}, but the integrand has {group_count} graph groups"
                        )));
                    }
                    if group_indices[..local_group_idx].contains(&group_idx) {
                        return Err(EngineError::build(format!(
                            "gammaloop graph_groups contains duplicate graph group {group_idx}"
                        )));
                    }
                    let branch = build_group_branch(integrand, momentum_space, group_idx)?;
                    group_branches.push(DomainBranch::new(local_group_idx, branch));
                }
                if group_branches.is_empty() {
                    return Err(EngineError::build(
                        "failed to infer gammaloop domain: no graph groups found",
                    ));
                }
                Ok(Domain::discrete(
                    Some("graph_group".to_string()),
                    group_branches,
                ))
            }
        }
    }

    fn domain_axes(integrand: &ProcessIntegrand) -> Vec<&'static str> {
        match integrand.get_settings().sampling.clone() {
            SamplingSettings::Default(_) | SamplingSettings::MultiChanneling(_) => Vec::new(),
            SamplingSettings::DiscreteGraphs(discrete_settings) => {
                let mut axes = vec!["graph_group"];
                if discrete_settings.sample_orientations {
                    axes.push("orientation");
                }
                if matches!(
                    discrete_settings.sampling_type,
                    DiscreteGraphSamplingType::SamplingMultiChanneling(_)
                ) {
                    axes.push("channel");
                }
                axes
            }
        }
    }

    fn panic_message(payload: Box<dyn Any + Send>) -> String {
        if let Some(message) = payload.downcast_ref::<&str>() {
            return (*message).to_string();
        }
        if let Some(message) = payload.downcast_ref::<String>() {
            return message.clone();
        }
        "unknown panic payload".to_string()
    }

    fn call_external<T, E>(
        label: &str,
        action: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, EngineError>
    where
        E: std::fmt::Display,
    {
        match std::panic::catch_unwind(AssertUnwindSafe(action)) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(EngineError::eval(format!("{label} failed: {err:#}"))),
            Err(payload) => Err(EngineError::eval(format!(
                "{label} panicked: {}",
                Self::panic_message(payload)
            ))),
        }
    }

    fn run_preprocessing(params: &GammaLoopParams, state: &mut State) -> Result<(), EngineError> {
        let mut run_history = RunHistory::default();
        let mut cli_settings = CLISettings::default();
        cli_settings.state.folder = params.state_folder.clone();
        cli_settings.session.read_only_state = params.preprocessing.read_only;
        let mut default_runtime_settings = RuntimeSettings::default();

        for (index, raw_command) in params.preprocessing.commands.iter().enumerate() {
            let command = CommandHistory::from_raw_string(raw_command).map_err(|err| {
                EngineError::build(format!(
                    "failed to parse preprocessing.commands[{index}] '{raw_command}': {err}"
                ))
            })?;

            tracing::info!(
                index,
                command = raw_command,
                read_only = params.preprocessing.read_only,
                "running gammaloop preprocessing command"
            );

            let execution = command
                .command
                .run(
                    state,
                    &mut run_history,
                    &mut cli_settings,
                    &mut default_runtime_settings,
                )
                .map_err(|err| {
                    EngineError::build(format!(
                        "failed to execute preprocessing.commands[{index}] '{raw_command}': {err}"
                    ))
                })?;

            if let ControlFlow::Break(_) = execution.flow {
                return Err(EngineError::build(format!(
                    "preprocessing.commands[{index}] triggered flow break and is not supported: '{}'",
                    raw_command
                )));
            }
        }

        Ok(())
    }

    pub fn from_params(params: GammaLoopParams) -> Result<Self, EngineError> {
        match std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<Self, EngineError> {
            let (mut integrand, model, metadata) = Self::load_integrand_and_model(params.clone())?;
            let domain = Self::build_domain(
                &integrand,
                metadata.momentum_space,
                params.graph_groups.as_deref(),
            )?;
            // Native map compilation needs a larger stack, especially in debug
            // builds. Join before returning; evaluation stays on the worker thread.
            std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .name("gammaloop-warmup".into())
                    .stack_size(64 * 1024 * 1024)
                    .spawn_scoped(scope, || integrand.warm_up(&model))
                    .build_err()?
                    .join()
                    .map_err(|payload| {
                        EngineError::build(format!(
                            "gammaloop warm-up panicked: {}",
                            Self::panic_message(payload)
                        ))
                    })?
                    .map_err(|err| {
                        EngineError::build(format!("failed to warm up integrand: {err}"))
                    })
            })?;
            Ok(Self {
                pristine_integrand: integrand.clone(),
                integrand,
                model,
                momentum_space: metadata.momentum_space,
                reference_gaussian: metadata.reference_gaussian.clone(),
                metadata,
                training_projection: params.training_projection,
                graph_groups: params.graph_groups,
                domain,
            })
        })) {
            Ok(result) => result,
            Err(payload) => Err(EngineError::build(format!(
                "gammaloop evaluator initialization panicked: {}",
                Self::panic_message(payload)
            ))),
        }
    }

    fn returned_result_value(result: &EvaluationResult) -> num::Complex<f64> {
        num::Complex::new(result.integrand_result.re.0, result.integrand_result.im.0)
    }

    fn project_result_value(result: &EvaluationResult) -> num::Complex<f64> {
        let mut value = Self::returned_result_value(result);
        if let Some(jac) = result.parameterization_jacobian {
            value *= jac.0;
        }
        value
    }

    fn map_graph_group(&self, mut discrete: Vec<usize>) -> Result<Vec<usize>, EngineError> {
        let Some(graph_groups) = &self.graph_groups else {
            return Ok(discrete);
        };
        let Some(local_group) = discrete.first_mut() else {
            return Err(EngineError::eval(
                "gammaloop graph_groups requires a graph-group discrete coordinate",
            ));
        };
        *local_group = graph_groups.get(*local_group).copied().ok_or_else(|| {
            EngineError::eval(format!(
                "gammaloop graph-group coordinate {} is outside the configured {} groups",
                *local_group,
                graph_groups.len()
            ))
        })?;
        Ok(discrete)
    }

    fn batch_gammaloop_observable(
        &self,
        evaluation_results: &[EvaluationResult],
        points: &[crate::evaluation::Point],
        options: EvalBatchOptions<'_>,
    ) -> Result<GammaLoopAccumulatorState, EngineError> {
        let mut state = Self::gammaloop_estimate(
            evaluation_results,
            points,
            self.training_projection,
            options,
        )?;
        state.bundle = self
            .integrand
            .observable_snapshot_bundle()
            .unwrap_or_default();
        Ok(state)
    }

    fn gammaloop_estimate(
        evaluation_results: &[EvaluationResult],
        points: &[crate::evaluation::Point],
        training_projection: TrainingProjection,
        options: EvalBatchOptions<'_>,
    ) -> Result<GammaLoopAccumulatorState, EngineError> {
        let mut estimate = VectorAccumulatorState::from_config(
            vec!["real".to_string(), "imag".to_string()],
            AccumulatorTrainingProjection::Norm,
            None,
            // Track 4th-order moments so each batch contributes the running sums
            // needed for the RSD metric's uncertainty (see GammaLoopAccumulatorState).
            AccumulatorMomentConfig::MaxOrder4,
        );
        let mut training_norm_sqr = (training_projection == TrainingProjection::AbsSq).then(|| {
            Box::new(crate::evaluation::ScalarAccumulatorState::from_config(
                None,
                AccumulatorMomentConfig::MaxOrder4,
            ))
        });
        for (sample, (result, point)) in evaluation_results.iter().zip(points.iter()).enumerate() {
            let raw = Self::project_result_value(result);
            let matched = options.matches(sample, &[raw.re, raw.im])?;
            if let Some(statistics) = &mut training_norm_sqr {
                // Project after parameterization, then apply the sampling weight
                // once (squaring that weight would describe a different estimator).
                statistics
                    .add_sample_without_discrete_projection(
                        if matched {
                            training_projection.project(raw)
                        } else {
                            0.
                        },
                        point,
                    )
                    .map_err(|err| {
                        EngineError::eval(format!("sample {sample}, training projection: {err}"))
                    })?;
            }
            let evaluation = crate::evaluation::batch::SampleEvaluation {
                integrand_value_re: result.integrand_result.re.0,
                integrand_value_im: result.integrand_result.im.0,
                parameterization_jacobian: result.parameterization_jacobian.map(|jac| jac.0),
            };
            // Statistics use the magnitude of sampling/parameterization weights.
            // Apply the parameterization factor here once; the diagnostic record
            // is only descriptive and cannot change accumulation.
            let value = Self::returned_result_value(result)
                * evaluation.parameterization_jacobian.unwrap_or(1.0).abs();
            if !(value.re * point.total_weight()).is_finite()
                || !(value.im * point.total_weight()).is_finite()
            {
                return Err(EngineError::eval(
                    "non-finite weighted GammaLoop contribution",
                ));
            }
            let value = if matched {
                value
            } else {
                num::Complex::new(0., 0.)
            };
            estimate
                .ingest_vector_with_details(
                    &[value.re, value.im],
                    point,
                    Some(&evaluation),
                    options.stratum_assignment(sample, &[raw.re, raw.im])?,
                )
                .map_err(|err| EngineError::eval(format!("sample {sample}: {err}")))?;
        }
        Ok(GammaLoopAccumulatorState {
            bundle: Default::default(),
            estimate,
            training_projection: Some(training_projection),
            training_norm_sqr,
            diagnostics: GammaLoopAccumulatorState::diagnostics_from_evaluation_results(
                evaluation_results,
            ),
        })
    }

    fn momentum_input(
        &self,
        point: &crate::Point,
    ) -> Result<MomentumSpaceEvaluationInput, EngineError> {
        if !point.continuous.len().is_multiple_of(3) {
            return Err(EngineError::eval(format!(
                "momentum-space evaluation expects point dimension divisible by 3, got {}",
                point.continuous.len()
            )));
        }
        let loop_momenta = point
            .continuous
            .as_chunks::<3>()
            .0
            .iter()
            .map(|coords| gammalooprs::momentum::ThreeMomentum {
                px: F(coords[0]),
                py: F(coords[1]),
                pz: F(coords[2]),
            })
            .collect::<Vec<_>>();
        let discrete_dim = point
            .discrete
            .iter()
            .copied()
            .map(|dim| {
                usize::try_from(dim).map_err(|_| {
                    EngineError::eval(format!("batch has negative discrete index {}", dim))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let discrete_dim = self.map_graph_group(discrete_dim)?;

        let (group_id, orientation, channel_id) = match &self.integrand.get_settings().sampling {
            SamplingSettings::Default(_) | SamplingSettings::MultiChanneling(_) => {
                if !discrete_dim.is_empty() {
                    return Err(EngineError::eval(format!(
                        "integrand does not use discrete graph sampling, but received discrete dimensions {:?}",
                        discrete_dim
                    )));
                }
                (None, None, None)
            }
            SamplingSettings::DiscreteGraphs(_) => {
                Self::call_external("resolve_discrete_selection", || {
                    self.integrand
                        .resolve_discrete_selection(discrete_dim.as_slice())
                })?
            }
        };

        Ok(MomentumSpaceEvaluationInput {
            loop_momenta,
            integrator_weight: F(point.total_weight()),
            graph_id: None,
            group_id,
            orientation,
            channel_id,
        })
    }

    fn sampling_point(&self, point: &crate::Point) -> Result<Sample<F<f64>>, EngineError> {
        let cont = point.continuous.iter().map(|&x| F(x)).collect::<Vec<_>>();
        let discrete_dim = point
            .discrete
            .iter()
            .copied()
            .map(|dim| {
                usize::try_from(dim).map_err(|_| {
                    EngineError::eval(format!("batch has negative discrete index {}", dim))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let discrete_dim = self.map_graph_group(discrete_dim)?;
        let expected_dimension = Self::call_external("expected_x_space_dimension", || {
            self.integrand
                .expected_x_space_dimension(discrete_dim.as_slice())
        })?;
        if cont.len() != expected_dimension {
            return Err(EngineError::eval(format!(
                "expected {expected_dimension} x-space coordinates for this selection, got {}",
                cont.len()
            )));
        }
        Ok(havana_sample(
            cont,
            discrete_dim.as_slice(),
            F(point.total_weight()),
        ))
    }

    fn evaluate(
        &mut self,
        batch: &Batch,
        components: &[String],
        options: EvalBatchOptions<'_>,
        record_observables: bool,
    ) -> Result<Vec<EvaluationResult>, EngineError> {
        if components != ["real", "imag"] && components != ["value"] {
            return Err(EngineError::invalid_input(
                "GammaLoop values require value or real/imag components",
            ));
        }
        options.validate(batch.len(), components)?;
        if record_observables {
            self.integrand = self.pristine_integrand.clone();
        }
        let mut results = Vec::with_capacity(batch.len());
        for (index, point) in batch.points().iter().enumerate() {
            let mut result = if self.momentum_space {
                let input = self.momentum_input(point)?;
                Self::call_external("evaluate_momentum_configuration", || {
                    self.integrand
                        .evaluate_momentum_configuration(&self.model, &input, false)
                })?
            } else {
                let sample = self.sampling_point(point)?;
                Self::call_external("evaluate_sample", || match &self.reference_gaussian {
                    // The integration overlay reports its normalized reference
                    // moment in Im. Its configuration forbids native observables.
                    Some(reference) => self
                        .integrand
                        .evaluate_samples_raw(
                            EvaluationTarget::Reference(reference),
                            std::slice::from_ref(&sample),
                            1,
                            false,
                            false,
                            Default::default(),
                        )
                        .map(|mut batch| batch.samples.remove(0)),
                    None => match &mut self.integrand {
                        ProcessIntegrand::Amplitude(integrand) => integrand.evaluate_sample(
                            &sample,
                            &self.model,
                            F(point.total_weight()),
                            1,
                            false,
                            Default::default(),
                        ),
                        ProcessIntegrand::CrossSection(integrand) => integrand.evaluate_sample(
                            &sample,
                            &self.model,
                            F(point.total_weight()),
                            1,
                            false,
                            Default::default(),
                        ),
                    },
                })?
            };
            let value = Self::project_result_value(&result);
            if !value.re.is_finite() || !value.im.is_finite() {
                return Err(EngineError::eval("non-finite GammaLoop integrand value"));
            }
            let projected = [self.training_projection.project(value)];
            let complex = [value.re, value.im];
            let values = if components.len() == 1 {
                projected.as_slice()
            } else {
                complex.as_slice()
            };
            let matched = options.matches(index, values)?;
            if !matched {
                result.event_groups.clear();
            }
            if record_observables {
                // One commit per original draw, even on a miss. Native processing
                // groups correlated events before accumulating squared bin sums.
                Self::call_external("process_evaluation_result", || {
                    self.integrand.process_evaluation_result(&result);
                    Ok::<_, EngineError>(())
                })?;
            }
            result.event_groups.clear();
            results.push(result);
        }
        Ok(results)
    }

    fn values(&self, results: &[EvaluationResult], components: &[String]) -> Vec<f64> {
        if components.len() == 1 {
            results
                .iter()
                .map(|r| {
                    self.training_projection
                        .project(Self::project_result_value(r))
                })
                .collect()
        } else {
            results
                .iter()
                .flat_map(|r| {
                    let v = Self::project_result_value(r);
                    [v.re, v.im]
                })
                .collect()
        }
    }
}

fn havana_sample(
    cont: Vec<F<f64>>,
    discrete_dimensions: &[usize],
    integrator_weight: F<f64>,
) -> Sample<F<f64>> {
    let mut sample = Sample::Continuous(F(1.0), cont);

    for &discrete_dimension in discrete_dimensions.iter().rev() {
        sample = Sample::Discrete(F(1.0), discrete_dimension, Some(Box::new(sample)));
    }

    set_top_level_sample_weight(&mut sample, integrator_weight);
    sample
}

fn set_top_level_sample_weight(sample: &mut Sample<F<f64>>, integrator_weight: F<f64>) {
    match sample {
        Sample::Continuous(weight, _)
        | Sample::Discrete(weight, _, _)
        | Sample::Uniform(weight, _, _) => *weight = integrator_weight,
    }
}

impl Evaluator for GammaLoopEvaluator {
    fn domain(&self) -> Domain {
        self.domain.clone()
    }

    fn metadata(&self) -> JsonValue {
        serde_json::to_value(&self.metadata).unwrap_or_else(|_| json!({}))
    }

    fn eval_values(
        &mut self,
        batch: &Batch,
        components: &[String],
    ) -> Result<Vec<f64>, EngineError> {
        let results = self.evaluate(batch, components, EvalBatchOptions::default(), false)?;
        Ok(self.values(&results, components))
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EngineError> {
        let native = matches!(accumulator, AccumulatorConfig::Gammaloop);
        let components = if native {
            vec!["real".into(), "imag".into()]
        } else {
            accumulator
                .components()
                .unwrap_or(&["value".into()])
                .to_vec()
        };
        let results = self.evaluate(batch, &components, options, native)?;
        let values = self.values(&results, &components);
        if !native {
            return crate::evaluation::stratification::accumulate_values(
                batch,
                &components,
                &values,
                accumulator,
                options,
            );
        }
        let state = self.batch_gammaloop_observable(&results, batch.points(), options)?;
        let training_values = if options.require_training_values {
            Some(
                results
                    .iter()
                    .zip(batch.points())
                    .map(|(result, point)| {
                        let value = Self::project_result_value(result);
                        let projected =
                            self.training_projection.project(value) * point.total_weight();
                        if !projected.is_finite() {
                            return Err(EngineError::eval(
                                "non-finite GammaLoop training feedback",
                            ));
                        }
                        Ok(projected)
                    })
                    .collect::<Result<Vec<_>, EngineError>>()?,
            )
        } else {
            None
        };
        crate::evaluation::stratification::finish_gammaloop(
            batch,
            Some(&values),
            state,
            training_values,
            options,
        )
    }
}

#[cfg(test)]
mod acceptance;
#[cfg(test)]
mod tests;
