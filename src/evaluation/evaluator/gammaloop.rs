use std::path::PathBuf;

use gammaloop_api::state::{ProcessRef, State};
use gammalooprs::initialisation::initialise;
use gammalooprs::integrands::process::{MomentumSpaceEvaluationInput, ProcessIntegrand};
use gammalooprs::model::Model;
use gammalooprs::settings::runtime::SamplingSettings;
use gammalooprs::utils::F;
use serde::{Deserialize, Serialize};
use symbolica::numerical_integration::Sample;

use crate::{
    Batch, BatchResult, BuildError, EvalError, PointSpec,
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
    point_spec: PointSpec,
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
    pub continuous_dims: usize,
    pub discrete_dims: usize,
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
            continuous_dims: 3,
            discrete_dims: 0,
        }
    }
}

impl GammaLoopEvaluator {
    pub fn from_params(params: GammaLoopParams) -> Result<Self, BuildError> {
        _ = initialise();
        let mut state = State::load(params.state_folder.clone(), None, None).map_err(|err| {
            BuildError::build(format!(
                "failed to load state from {}: {err}",
                params.state_folder.display()
            ))
        })?;

        let (process_id, integrand_name) = state
            .find_integrand_ref(params.process_id.as_ref(), params.integrand_name.as_ref())
            .map_err(|err| BuildError::build(format!("failed to find integrand: {err}")))?;

        let integrand = state
            .process_list
            .get_integrand_mut(process_id, integrand_name.clone())
            .map_err(|err| BuildError::build(err.to_string()))?
            .clone();
        let model = state.model.clone();

        Ok(Self {
            integrand,
            model,
            momentum_space: params.momentum_space,
            training_projection: params.training_projection,
            point_spec: PointSpec {
                continuous_dims: params.continuous_dims,
                discrete_dims: params.discrete_dims,
            },
        })
    }

    fn evaluate(&mut self, batch: &Batch) -> Result<Vec<num::complex::Complex64>, EvalError> {
        if self.momentum_space {
            let inputs = batch
                .continuous()
                .outer_iter()
                .zip(batch.discrete().outer_iter())
                .map(|(point, discrete_dim)| {
                    let point = point.to_vec();
                    if !point.len().is_multiple_of(3) {
                        return Err(EvalError::eval(format!(
                            "momentum-space evaluation expects point dimension divisible by 3, got {}",
                            point.len()
                        )));
                    }
                    let loop_momenta = point
                        .chunks_exact(3)
                        .map(|coords| gammalooprs::momentum::ThreeMomentum {
                            px: F(coords[0]),
                            py: F(coords[1]),
                            pz: F(coords[2]),
                        })
                        .collect::<Vec<_>>();
                    let discrete_dim = discrete_dim
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
                        SamplingSettings::DiscreteGraphs(_) => self
                            .integrand
                            .resolve_discrete_selection(discrete_dim.as_slice())
                            .map_err(|err| {
                                EvalError::eval(format!(
                                    "invalid momentum-space discrete selection: {err}"
                                ))
                            })?,
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

            let results = self
                .integrand
                .evaluate_momentum_configurations_raw(&self.model, inputs.as_slice(), false)
                .map_err(|err| EvalError::eval(format!("failed to evaluate integrand: {err}")))?;

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
            .continuous()
            .outer_iter()
            .zip(batch.discrete().outer_iter())
            .map(|(point, discrete_dim)| {
                let cont = point.iter().map(|&x| F(x)).collect::<Vec<_>>();
                let discrete_dim = discrete_dim
                    .iter()
                    .copied()
                    .map(|dim| {
                        usize::try_from(dim).map_err(|_| {
                            EvalError::eval(format!("batch has negative discrete index {}", dim))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let expected_dimension = self
                    .integrand
                    .expected_x_space_dimension(discrete_dim.as_slice())
                    .map_err(|err| EvalError::eval(format!("invalid x-space selection: {err}")))?;
                if cont.len() != expected_dimension {
                    return Err(EvalError::eval(format!(
                        "expected {expected_dimension} x-space coordinates for this selection, got {}",
                        cont.len()
                    )));
                }
                Ok(havana_sample(cont, discrete_dim.as_slice(), F(1.0)))
            })
            .collect::<Result<Vec<Sample<F<f64>>>, _>>()?;

        let results = self
            .integrand
            .evaluate_samples_raw(
                &self.model,
                samples.as_slice(),
                1,
                false,
                false,
                Default::default(),
            )
            .map_err(|err| EvalError::eval(format!("failed to evaluate integrand: {err}")))?;

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
    fn get_point_spec(&self) -> PointSpec {
        self.point_spec.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let vec_res = self.evaluate(batch)?;
        let mut observable_state = ObservableState::from_config(observable);
        let weighted_values = match observable.semantic_kind() {
            crate::evaluation::SemanticObservableKind::Scalar => match &mut observable_state {
                ObservableState::Scalar(observable) => self.ingest_scalar_values(
                    &vec_res
                        .iter()
                        .map(|value| self.training_projection.project(*value))
                        .collect::<Vec<_>>(),
                    weights,
                    options.require_training_values,
                    observable,
                ),
                ObservableState::FullScalar(observable) => self.ingest_scalar_values(
                    &vec_res
                        .iter()
                        .map(|value| self.training_projection.project(*value))
                        .collect::<Vec<_>>(),
                    weights,
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
                    weights,
                    options.require_training_values,
                    observable,
                    |value| self.training_projection.project(value),
                ),
                ObservableState::FullComplex(observable) => self.ingest_complex_values(
                    &vec_res,
                    weights,
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
