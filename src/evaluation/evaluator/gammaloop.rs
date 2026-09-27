use crate::core::EngineResultExt;
use std::{any::Any, ops::ControlFlow, panic::AssertUnwindSafe, path::PathBuf};

use gammaloop_api::{
    CLISettings,
    state::{CommandHistory, ProcessRef, RunHistory, State},
};
use gammalooprs::graph::GroupId;
use gammalooprs::initialisation::initialise;
use gammalooprs::integrands::HasIntegrand;
use gammalooprs::integrands::evaluation::EvaluationResult;
use gammalooprs::integrands::process::{
    EvaluationTarget, GaussianReferenceFunction, MomentumSpaceEvaluationInput, ProcessIntegrand,
    SamplingChannelInspection,
};
use gammalooprs::model::Model;
use gammalooprs::settings::RuntimeSettings;
use gammalooprs::settings::runtime::{
    DiscreteGraphSamplingType, Precision, SamplingSettings, SamplingSettingsParser,
    StabilityLevelSetting,
};
use gammalooprs::utils::F;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use symbolica::numerical_integration::Sample;

use crate::{
    Batch, BatchResult, BuildError, Domain, DomainBranch, EvalError,
    core::{
        AccumulatorConfig, AccumulatorMomentConfig,
        TrainingProjection as AccumulatorTrainingProjection,
    },
    evaluation::{
        AccumulatorState, EvalBatchOptions, Evaluator, GammaLoopAccumulatorState,
        VectorAccumulatorState,
    },
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrainingProjection {
    #[default]
    Real,
    Imag,
    Abs,
    AbsSq,
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GammaLoopParams {
    pub state_folder: PathBuf,
    pub process_id: Option<ProcessRef>,
    pub integrand_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_groups: Option<Vec<usize>>,
    pub momentum_space: bool,
    pub use_f128: bool,
    pub training_projection: TrainingProjection,
    /// Replace physics with known normalization and second-moment targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_gaussian: Option<GaussianReferenceFunction>,
    pub preprocessing: GammaLoopPreprocessing,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GammaLoopPreprocessing {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    pub read_only: bool,
}

impl Default for GammaLoopPreprocessing {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            read_only: true,
        }
    }
}

impl Default for GammaLoopParams {
    fn default() -> Self {
        Self {
            state_folder: PathBuf::from("./gammaloop_state"),
            process_id: None,
            integrand_name: None,
            graph_groups: None,
            momentum_space: false,
            use_f128: false,
            training_projection: TrainingProjection::default(),
            reference_gaussian: None,
            preprocessing: GammaLoopPreprocessing::default(),
        }
    }
}

impl GammaLoopEvaluator {
    fn load_integrand_and_model(
        mut params: GammaLoopParams,
    ) -> Result<(ProcessIntegrand, Model, GammaLoopMetadata), BuildError> {
        params.state_folder = resolve_resource_path(&params.state_folder).map_err(|err| {
            BuildError::build(format!(
                "failed to resolve gammaloop state_folder '{}': {err}",
                params.state_folder.display()
            ))
        })?;
        crate::activate_symbolica_oem_license().build_err()?;
        initialise()
            .map_err(|err| BuildError::build(format!("gammaloop initialization: {err}")))?;
        if params.momentum_space && params.reference_gaussian.is_some() {
            return Err(BuildError::invalid_input(
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
                    BuildError::build(format!(
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
            .map_err(|err| BuildError::build(format!("failed to find integrand: {err}")))?;

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
    ) -> Result<Vec<SamplingChannelInspection>, BuildError> {
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
                    .ok_or_else(|| BuildError::build(format!("missing master graph '{name}'")))?;
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

    pub fn resolve_domain_from_params(params: GammaLoopParams) -> Result<Domain, BuildError> {
        match std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<Domain, BuildError> {
            let graph_groups = params.graph_groups.clone();
            let (integrand, _model, metadata) = Self::load_integrand_and_model(params)?;
            Self::build_domain(&integrand, metadata.momentum_space, graph_groups.as_deref())
        })) {
            Ok(result) => result,
            Err(payload) => Err(BuildError::build(format!(
                "gammaloop domain resolution panicked: {}",
                Self::panic_message(payload)
            ))),
        }
    }

    fn build_domain(
        integrand: &ProcessIntegrand,
        momentum_space: bool,
        selected_graph_groups: Option<&[usize]>,
    ) -> Result<Domain, BuildError> {
        fn continuous_leaf(
            integrand: &ProcessIntegrand,
            momentum_space: bool,
            discrete_selection: &[usize],
        ) -> Result<Domain, BuildError> {
            let dims = if momentum_space {
                let dims = integrand.get_n_dim();
                if !dims.is_multiple_of(3) {
                    return Err(BuildError::build(format!(
                        "gammaloop momentum-space domain dimension must be divisible by 3, got {dims}"
                    )));
                }
                dims
            } else {
                integrand
                    .expected_x_space_dimension(discrete_selection)
                    .map_err(|err| {
                        BuildError::build(format!(
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
        ) -> Result<Domain, BuildError> {
            let settings = integrand.get_settings();
            let SamplingSettings::DiscreteGraphs(discrete_settings) = &settings.sampling else {
                return continuous_leaf(integrand, momentum_space, &[]);
            };

            let group_id = GroupId::from(group_idx);
            let base_selection = [group_idx];

            if discrete_settings.sample_orientations {
                let orientation_count =
                    integrand.group_orientation_count(group_id).ok_or_else(|| {
                        BuildError::build(format!(
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
                                        BuildError::build(format!(
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
                                    .collect::<Result<Vec<_>, BuildError>>()?;
                                Domain::discrete(Some("channel".to_string()), channel_branches)
                            }
                            _ => continuous_leaf(integrand, momentum_space, selection.as_slice())?,
                        };
                        Ok(DomainBranch::new(orientation_idx, domain))
                    })
                    .collect::<Result<Vec<_>, BuildError>>()?;
                return Ok(Domain::discrete(
                    Some("orientation".to_string()),
                    orientation_branches,
                ));
            }

            match &discrete_settings.sampling_type {
                DiscreteGraphSamplingType::SamplingMultiChanneling(_) => {
                    let channel_count =
                        integrand.group_channel_count(group_id).ok_or_else(|| {
                            BuildError::build(format!(
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
                        .collect::<Result<Vec<_>, BuildError>>()?;
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
                    return Err(BuildError::build(
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
                    return Err(BuildError::build(
                        "gammaloop graph_groups must select at least one graph group",
                    ));
                }
                let mut group_branches = Vec::with_capacity(group_indices.len());
                for (local_group_idx, group_idx) in group_indices.iter().copied().enumerate() {
                    if group_idx >= group_count {
                        return Err(BuildError::build(format!(
                            "gammaloop graph_groups contains {group_idx}, but the integrand has {group_count} graph groups"
                        )));
                    }
                    if group_indices[..local_group_idx].contains(&group_idx) {
                        return Err(BuildError::build(format!(
                            "gammaloop graph_groups contains duplicate graph group {group_idx}"
                        )));
                    }
                    let branch = build_group_branch(integrand, momentum_space, group_idx)?;
                    group_branches.push(DomainBranch::new(local_group_idx, branch));
                }
                if group_branches.is_empty() {
                    return Err(BuildError::build(
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
    ) -> Result<T, EvalError>
    where
        E: std::fmt::Display,
    {
        match std::panic::catch_unwind(AssertUnwindSafe(action)) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(EvalError::eval(format!("{label} failed: {err:#}"))),
            Err(payload) => Err(EvalError::eval(format!(
                "{label} panicked: {}",
                Self::panic_message(payload)
            ))),
        }
    }

    fn run_preprocessing(params: &GammaLoopParams, state: &mut State) -> Result<(), BuildError> {
        let mut run_history = RunHistory::default();
        let mut cli_settings = CLISettings::default();
        cli_settings.state.folder = params.state_folder.clone();
        cli_settings.session.read_only_state = params.preprocessing.read_only;
        let mut default_runtime_settings = RuntimeSettings::default();

        for (index, raw_command) in params.preprocessing.commands.iter().enumerate() {
            let command = CommandHistory::from_raw_string(raw_command).map_err(|err| {
                BuildError::build(format!(
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
                    BuildError::build(format!(
                        "failed to execute preprocessing.commands[{index}] '{raw_command}': {err}"
                    ))
                })?;

            if let ControlFlow::Break(_) = execution.flow {
                return Err(BuildError::build(format!(
                    "preprocessing.commands[{index}] triggered flow break and is not supported: '{}'",
                    raw_command
                )));
            }
        }

        Ok(())
    }

    pub fn from_params(params: GammaLoopParams) -> Result<Self, BuildError> {
        match std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<Self, BuildError> {
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
                        BuildError::build(format!(
                            "gammaloop warm-up panicked: {}",
                            Self::panic_message(payload)
                        ))
                    })?
                    .map_err(|err| BuildError::build(format!("failed to warm up integrand: {err}")))
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
            Err(payload) => Err(BuildError::build(format!(
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

    fn map_graph_group(&self, mut discrete: Vec<usize>) -> Result<Vec<usize>, EvalError> {
        let Some(graph_groups) = &self.graph_groups else {
            return Ok(discrete);
        };
        let Some(local_group) = discrete.first_mut() else {
            return Err(EvalError::eval(
                "gammaloop graph_groups requires a graph-group discrete coordinate",
            ));
        };
        *local_group = graph_groups.get(*local_group).copied().ok_or_else(|| {
            EvalError::eval(format!(
                "gammaloop graph-group coordinate {} is outside the configured {} groups",
                *local_group,
                graph_groups.len()
            ))
        })?;
        Ok(discrete)
    }

    fn ingest_vector_batch(
        accumulator: &mut VectorAccumulatorState,
        evaluation_results: &[EvaluationResult],
        points: &[crate::evaluation::Point],
        require_training_values: bool,
        mut values_of: impl FnMut(&EvaluationResult) -> Vec<f64>,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let mut training_values =
            require_training_values.then(|| Vec::with_capacity(evaluation_results.len()));
        for (result, point) in evaluation_results.iter().zip(points.iter()) {
            let projected = accumulator
                .ingest_vector(&values_of(result), point)
                .map_err(EvalError::eval)?;
            if let Some(training_values) = training_values.as_mut() {
                training_values.push(projected * point.total_weight());
            }
        }
        Ok(training_values)
    }

    fn batch_gammaloop_observable(
        &self,
        evaluation_results: &[EvaluationResult],
        points: &[crate::evaluation::Point],
    ) -> GammaLoopAccumulatorState {
        let mut state =
            Self::gammaloop_estimate(evaluation_results, points, self.training_projection);
        state.bundle = self
            .integrand
            .observable_snapshot_bundle()
            .unwrap_or_default();
        state
    }

    fn gammaloop_estimate(
        evaluation_results: &[EvaluationResult],
        points: &[crate::evaluation::Point],
        training_projection: TrainingProjection,
    ) -> GammaLoopAccumulatorState {
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
        for (result, point) in evaluation_results.iter().zip(points.iter()) {
            if let Some(statistics) = &mut training_norm_sqr {
                // Project after parameterization, then apply the sampling weight
                // once (squaring that weight would describe a different estimator).
                statistics.add_sample_without_discrete_projection(
                    training_projection.project(Self::project_result_value(result)),
                    point,
                );
            }
            let mut debug_point = point.clone();
            debug_point.integrand_value_re = Some(result.integrand_result.re.0);
            debug_point.integrand_value_im = Some(result.integrand_result.im.0);
            if let Some(jacobian) = result.parameterization_jacobian.map(|jac| jac.0) {
                debug_point.parameterization_jacobian = Some(jacobian);
                debug_point.add_weight_factor("gammaloop_parameterization_jacobian", jacobian);
            }
            // Keep any remaining top-level Jacobian separate. Current GammaLoop's
            // returned contribution already includes native map/partition factors;
            // its top-level Jacobian is unity. Older records may have a nonunit one.
            let value = Self::returned_result_value(result);
            estimate
                .ingest_vector(&[value.re, value.im], &debug_point)
                .expect("gammaloop estimate vector components should match");
        }
        GammaLoopAccumulatorState {
            bundle: Default::default(),
            estimate,
            training_projection: Some(training_projection),
            training_norm_sqr,
            diagnostics: GammaLoopAccumulatorState::diagnostics_from_evaluation_results(
                evaluation_results,
            ),
        }
    }

    fn evaluate(&mut self, batch: &Batch) -> Result<Vec<EvaluationResult>, EvalError> {
        if self.momentum_space {
            let inputs = batch
                .points()
                .iter()
                .map(|point| {
                    if !point.continuous.len().is_multiple_of(3) {
                        return Err(EvalError::eval(format!(
                            "momentum-space evaluation expects point dimension divisible by 3, got {}",
                            point.continuous.len()
                        )));
                    }
                    let loop_momenta = point
                        .continuous
                        .as_chunks::<3>().0.iter()
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
                                EvalError::eval(format!(
                                    "batch has negative discrete index {}",
                                    dim
                                ))
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let discrete_dim = self.map_graph_group(discrete_dim)?;

                    let (group_id, orientation, channel_id) = match &self.integrand.get_settings().sampling
                    {
                        SamplingSettings::Default(_) | SamplingSettings::MultiChanneling(_) => {
                            if !discrete_dim.is_empty() {
                                return Err(EvalError::eval(format!(
                                    "integrand does not use discrete graph sampling, but received discrete dimensions {:?}",
                                    discrete_dim
                                )));
                            }
                            (None, None, None)
                        }
                        SamplingSettings::DiscreteGraphs(_) => Self::call_external(
                            "resolve_discrete_selection",
                            || self.integrand.resolve_discrete_selection(discrete_dim.as_slice()),
                        )?,
                    };

                    Ok(MomentumSpaceEvaluationInput {
                        loop_momenta,
                        integrator_weight: F(point.total_weight()),
                        graph_id: None,
                        group_id,
                        orientation,
                        channel_id,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            let results = Self::call_external("evaluate_momentum_configurations_raw", || {
                self.integrand.evaluate_momentum_configurations_raw(
                    &self.model,
                    inputs.as_slice(),
                    false,
                )
            })?;

            return Ok(results.samples);
        }

        let samples = batch
            .points()
            .iter()
            .map(|point| {
                let cont = point.continuous.iter().map(|&x| F(x)).collect::<Vec<_>>();
                let discrete_dim = point
                    .discrete
                    .iter()
                    .copied()
                    .map(|dim| {
                        usize::try_from(dim).map_err(|_| {
                            EvalError::eval(format!("batch has negative discrete index {}", dim))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let discrete_dim = self.map_graph_group(discrete_dim)?;
                let expected_dimension = Self::call_external("expected_x_space_dimension", || {
                    self.integrand
                        .expected_x_space_dimension(discrete_dim.as_slice())
                })?;
                if cont.len() != expected_dimension {
                    return Err(EvalError::eval(format!(
                        "expected {expected_dimension} x-space coordinates for this selection, got {}",
                        cont.len()
                    )));
                }
                Ok(havana_sample(cont, discrete_dim.as_slice(), F(point.total_weight())))
            })
            .collect::<Result<Vec<Sample<F<f64>>>, _>>()?;

        let results = Self::call_external("evaluate_samples_raw", || {
            self.integrand.evaluate_samples_raw(
                self.reference_gaussian.as_ref().map_or(
                    EvaluationTarget::Physical(&self.model),
                    EvaluationTarget::Reference,
                ),
                samples.as_slice(),
                1,
                false,
                false,
                Default::default(),
            )
        })?;

        Ok(results.samples)
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
    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn metadata(&self) -> JsonValue {
        serde_json::to_value(&self.metadata).unwrap_or_else(|_| json!({}))
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        if matches!(accumulator, AccumulatorConfig::Gammaloop) {
            // Isolate this batch from prior evaluations, including failed ones.
            self.integrand = self.pristine_integrand.clone();
        }
        let points = batch.points();
        let evaluation_results = self.evaluate(batch)?;
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let weighted_values = match accumulator {
            AccumulatorConfig::Empty => {
                observable_state = AccumulatorState::empty();
                if options.require_training_values {
                    Some(
                        evaluation_results
                            .iter()
                            .map(|result| {
                                self.training_projection
                                    .project(Self::project_result_value(result))
                            })
                            .zip(points.iter())
                            .map(|(value, point)| value * point.total_weight())
                            .collect(),
                    )
                } else {
                    None
                }
            }
            AccumulatorConfig::Gammaloop => {
                observable_state = AccumulatorState::Gammaloop(
                    self.batch_gammaloop_observable(&evaluation_results, points),
                );
                if options.require_training_values {
                    Some(
                        evaluation_results
                            .iter()
                            .map(|result| {
                                self.training_projection
                                    .project(Self::project_result_value(result))
                            })
                            .zip(points.iter())
                            .map(|(value, point)| value * point.total_weight())
                            .collect(),
                    )
                } else {
                    None
                }
            }
            AccumulatorConfig::Vector { .. } => {
                let AccumulatorState::Vector(accumulator) = &mut observable_state else {
                    return Err(EvalError::eval(format!(
                        "gammaloop vector mode does not support accumulator kind {}",
                        observable_state.kind_str()
                    )));
                };
                Self::ingest_vector_batch(
                    accumulator,
                    &evaluation_results,
                    points,
                    options.require_training_values,
                    |result| {
                        let value = Self::project_result_value(result);
                        vec![value.re, value.im]
                    },
                )?
            }
            _ => match accumulator.semantic_kind() {
                crate::evaluation::SemanticAccumulatorKind::Scalar => match &mut observable_state {
                    AccumulatorState::Vector(accumulator) => Self::ingest_vector_batch(
                        accumulator,
                        &evaluation_results,
                        points,
                        options.require_training_values,
                        |result| {
                            vec![
                                self.training_projection
                                    .project(Self::project_result_value(result)),
                            ]
                        },
                    )?,
                    AccumulatorState::FullVector(accumulator) => {
                        for (result, point) in evaluation_results.iter().zip(points.iter()) {
                            let value = self
                                .training_projection
                                .project(Self::project_result_value(result));
                            accumulator.push_vector(&[value * point.total_weight().abs()]);
                        }
                        None
                    }
                    other => {
                        return Err(EvalError::eval(format!(
                            "gammaloop scalar mode does not support accumulator kind {}",
                            other.kind_str()
                        )));
                    }
                },
                crate::evaluation::SemanticAccumulatorKind::Vector => match &mut observable_state {
                    AccumulatorState::FullVector(accumulator) => {
                        for (result, point) in evaluation_results.iter().zip(points.iter()) {
                            let value = Self::project_result_value(result);
                            let weight = point.total_weight().abs();
                            accumulator.push_vector(&[value.re * weight, value.im * weight]);
                        }
                        None
                    }
                    other => {
                        return Err(EvalError::eval(format!(
                            "gammaloop vector mode does not support accumulator kind {}",
                            other.kind_str()
                        )));
                    }
                },
            },
        };
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

#[cfg(test)]
mod acceptance;
#[cfg(test)]
mod tests;
