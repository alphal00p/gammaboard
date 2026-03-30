use std::{any::Any, panic::AssertUnwindSafe, path::PathBuf};

use gammaloop_api::state::{ProcessRef, State};
use gammalooprs::graph::GroupId;
use gammalooprs::initialisation::initialise;
use gammalooprs::integrands::HasIntegrand;
use gammalooprs::integrands::process::{MomentumSpaceEvaluationInput, ProcessIntegrand};
use gammalooprs::model::Model;
use gammalooprs::settings::runtime::{DiscreteGraphSamplingType, SamplingSettings};
use gammalooprs::utils::F;
use serde::{Deserialize, Serialize};
use symbolica::numerical_integration::Sample;

use crate::{
    Batch, BatchResult, BuildError, Domain, DomainBranch, EvalError,
    core::ObservableConfig,
    evaluation::{
        ComplexValueEvaluator, EvalBatchOptions, Evaluator, ObservableState, ScalarValueEvaluator,
    },
};

pub struct GammaLoopEvaluator {
    integrand: ProcessIntegrand,
    model: Model,
    momentum_space: bool,
    training_projection: TrainingProjection,
    domain: Domain,
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
    fn project(self, value: num::complex::Complex64) -> f64 {
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
    pub momentum_space: bool,
    pub use_f128: bool,
    pub training_projection: TrainingProjection,
}

impl Default for GammaLoopParams {
    fn default() -> Self {
        Self {
            state_folder: PathBuf::from("./gammaloop_state"),
            process_id: None,
            integrand_name: None,
            momentum_space: true,
            use_f128: false,
            training_projection: TrainingProjection::default(),
        }
    }
}

impl GammaLoopEvaluator {
    fn build_domain(
        integrand: &ProcessIntegrand,
        momentum_space: bool,
    ) -> Result<Domain, BuildError> {
        fn discrete_group_count(integrand: &ProcessIntegrand) -> usize {
            let discrete_depth = integrand.discrete_sampling_depth();
            debug_assert!(discrete_depth > 0);

            let mut group_count = 0usize;
            loop {
                let mut selection = vec![0; discrete_depth];
                selection[0] = group_count;
                if integrand
                    .resolve_discrete_selection(selection.as_slice())
                    .is_err()
                {
                    break;
                }
                group_count += 1;
            }
            group_count
        }

        fn continuous_leaf(
            integrand: &ProcessIntegrand,
            momentum_space: bool,
            discrete_selection: &[usize],
        ) -> Result<Domain, BuildError> {
            let dims = if momentum_space {
                integrand.get_n_dim()
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
                            DiscreteGraphSamplingType::DiscreteMultiChanneling(_) => {
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
                DiscreteGraphSamplingType::DiscreteMultiChanneling(_) => {
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
                continuous_leaf(integrand, momentum_space, &[])
            }
            SamplingSettings::DiscreteGraphs(_) => {
                let group_count = discrete_group_count(integrand);
                let mut group_branches = Vec::with_capacity(group_count);
                for group_idx in 0..group_count {
                    let branch = build_group_branch(integrand, momentum_space, group_idx)?;
                    group_branches.push(DomainBranch::new(group_idx, branch));
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
            Ok(Err(err)) => Err(EvalError::eval(format!("{label} failed: {err}"))),
            Err(payload) => Err(EvalError::eval(format!(
                "{label} panicked: {}",
                Self::panic_message(payload)
            ))),
        }
    }

    pub fn from_params(params: GammaLoopParams) -> Result<Self, BuildError> {
        match std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<Self, BuildError> {
            _ = initialise();
            let mut state =
                State::load(params.state_folder.clone(), None, None).map_err(|err| {
                    BuildError::build(format!(
                        "failed to load state from {}: {err}",
                        params.state_folder.display()
                    ))
                })?;

            let (process_id, integrand_name) = state
                .find_integrand_ref(params.process_id.as_ref(), params.integrand_name.as_ref())
                .map_err(|err| BuildError::build(format!("failed to find integrand: {err}")))?;

            let mut integrand = state
                .process_list
                .get_integrand_mut(process_id, integrand_name.clone())
                .map_err(|err| BuildError::build(err.to_string()))?
                .clone();
            let model = state.model.clone();
            let domain = Self::build_domain(&integrand, params.momentum_space)?;
            integrand
                .warm_up(&model)
                .map_err(|err| BuildError::build(format!("failed to warm up integrand: {err}")))?;

            Ok(Self {
                integrand,
                model,
                momentum_space: params.momentum_space,
                training_projection: params.training_projection,
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

    fn evaluate(&mut self, batch: &Batch) -> Result<Vec<num::complex::Complex64>, EvalError> {
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
                        .chunks_exact(3)
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
                        integrator_weight: F(1.0),
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

            return Ok(results
                .samples
                .into_iter()
                .map(|res| {
                    num::complex::Complex64::new(
                        res.integrand_result.re.0,
                        res.integrand_result.im.0,
                    )
                })
                .collect());
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
                Ok(havana_sample(cont, discrete_dim.as_slice(), F(1.0)))
            })
            .collect::<Result<Vec<Sample<F<f64>>>, _>>()?;

        let results = Self::call_external("evaluate_samples_raw", || {
            self.integrand.evaluate_samples_raw(
                &self.model,
                samples.as_slice(),
                1,
                false,
                false,
                Default::default(),
            )
        })?;

        Ok(results
            .samples
            .into_iter()
            .map(|res| {
                let mut value = num::complex::Complex64::new(
                    res.integrand_result.re.0,
                    res.integrand_result.im.0,
                );
                if let Some(jac) = res.parameterization_jacobian {
                    value *= jac.0;
                }
                value
            })
            .collect())
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

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch.weights();
        let vec_res = self.evaluate(batch)?;
        let mut observable_state = ObservableState::from_config(observable);
        let weighted_values = match observable.semantic_kind() {
            crate::evaluation::SemanticObservableKind::Scalar => match &mut observable_state {
                ObservableState::Scalar(observable) => self.ingest_scalar_values(
                    &vec_res
                        .iter()
                        .map(|value| self.training_projection.project(*value))
                        .collect::<Vec<_>>(),
                    weights.as_slice(),
                    options.require_training_values,
                    observable,
                ),
                ObservableState::FullScalar(observable) => self.ingest_scalar_values(
                    &vec_res
                        .iter()
                        .map(|value| self.training_projection.project(*value))
                        .collect::<Vec<_>>(),
                    weights.as_slice(),
                    options.require_training_values,
                    observable,
                ),
                other => {
                    return Err(EvalError::eval(format!(
                        "gammaloop scalar mode does not support observable kind {}",
                        other.kind_str()
                    )));
                }
            },
            crate::evaluation::SemanticObservableKind::Complex => match &mut observable_state {
                ObservableState::Complex(observable) => self.ingest_complex_values(
                    &vec_res,
                    weights.as_slice(),
                    options.require_training_values,
                    observable,
                    |value| self.training_projection.project(value),
                ),
                ObservableState::FullComplex(observable) => self.ingest_complex_values(
                    &vec_res,
                    weights.as_slice(),
                    options.require_training_values,
                    observable,
                    |value| self.training_projection.project(value),
                ),
                other => {
                    return Err(EvalError::eval(format!(
                        "gammaloop complex mode does not support observable kind {}",
                        other.kind_str()
                    )));
                }
            },
        };
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}
