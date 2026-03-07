use std::path::PathBuf;

use gammaloop_api::state::{ProcessRef, State};
use gammalooprs::gammaloop_integrand::GLIntegrand;
use gammalooprs::initialisation::initialise;
use gammalooprs::model::Model;
use gammalooprs::settings::RuntimeSettings;
use gammalooprs::{inspect::inspect, utils::F};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Batch, BatchResult, BuildError, EvalError, PointSpec,
    engines::{
        BuildFromJson, ComplexObservableState, EvalBatchOptions, Evaluator, ObservableState,
        ScalarObservableState, SemanticObservableKind,
    },
};

pub struct GammaLoopEvaluator {
    integrand: GLIntegrand,
    model: Model,
    settings: RuntimeSettings,
    state_folder: PathBuf,
    process_id: usize,
    integrand_name: String,
    momentum_space: bool,
    use_f128: bool,
    training_projection: TrainingProjection,
    observable_kind: SemanticObservableKind,
    point_spec: PointSpec,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GammaLoopParams {
    pub state_folder: PathBuf,
    pub model_file: Option<PathBuf>,
    pub process_id: Option<ProcessRef>,
    pub integrand_name: Option<String>,
    pub momentum_space: bool,
    pub use_f128: bool,
    pub training_projection: TrainingProjection,
    pub observable_kind: SemanticObservableKind,
    pub continuous_dims: usize,
    pub discrete_dims: usize,
}

impl Default for GammaLoopParams {
    fn default() -> Self {
        Self {
            state_folder: PathBuf::from("./gammaloop_state"),
            model_file: None,
            process_id: None,
            integrand_name: None,
            momentum_space: true,
            use_f128: false,
            training_projection: TrainingProjection::default(),
            observable_kind: SemanticObservableKind::Complex,
            continuous_dims: 3,
            discrete_dims: 0,
        }
    }
}

impl BuildFromJson for GammaLoopEvaluator {
    type Params = GammaLoopParams;

    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        _ = initialise();
        let state =
            State::load(params.state_folder.clone(), params.model_file, None).map_err(|err| {
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
            .get_integrand(process_id, integrand_name.clone())
            .map_err(|err| BuildError::build(err.to_string()))?
            .clone();

        _ = integrand.warm_up(&state.model);

        let settings = integrand.get_settings().clone();
        let model = state.model.clone();

        Ok(Self {
            integrand,
            model,
            settings,
            state_folder: params.state_folder,
            process_id,
            integrand_name,
            momentum_space: params.momentum_space,
            use_f128: params.use_f128,
            training_projection: params.training_projection,
            observable_kind: params.observable_kind,
            point_spec: PointSpec {
                continuous_dims: params.continuous_dims,
                discrete_dims: params.discrete_dims,
            },
        })
    }
}

impl GammaLoopEvaluator {
    fn evaluate(
        &mut self,
        batch: &Batch,
    ) -> Result<(Vec<num::complex::Complex64>, Option<Vec<f64>>), EvalError> {
        let mut vec_res = vec![];
        let mut jac_res = vec![];

        for (point, discrete_dim) in batch
            .continuous()
            .outer_iter()
            .zip(batch.discrete().outer_iter())
        {
            let pt = point.iter().map(|&x| F(x)).collect::<Vec<F<f64>>>();
            let discrete_dim = discrete_dim
                .iter()
                .copied()
                .map(|dim| {
                    usize::try_from(dim).map_err(|_| {
                        EvalError::eval(format!("batch has negative discrete index {}", dim))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            let (inspect_res_jac, inspect_res_eval) = inspect(
                &self.settings,
                &mut self.integrand,
                &self.model,
                pt,
                &discrete_dim,
                false,
                self.momentum_space,
                self.use_f128,
            )
            .map_err(|err| EvalError::eval(err.to_string()))?;
            let res_to_return: num::complex::Complex64 = if let Some(jac) = inspect_res_jac {
                jac_res.push(jac);
                if jac == 0.0 {
                    return Err(EvalError::eval(
                        "Jacobian is zero at this point, cannot divide by zero.",
                    ));
                }
                let value = inspect_res_eval.map(|a| a.0);
                num::complex::Complex64::new(value.re, value.im)
            } else {
                let value = inspect_res_eval.map(|a| a.into());
                num::complex::Complex64::new(value.re, value.im)
            };

            vec_res.push(res_to_return);
        }

        let res_jac = if jac_res.is_empty() {
            None
        } else {
            Some(jac_res)
        };

        Ok((vec_res, res_jac))
    }
}

impl Evaluator for GammaLoopEvaluator {
    fn get_point_spec(&self) -> PointSpec {
        self.point_spec.clone()
    }

    fn empty_observable(&self) -> ObservableState {
        self.observable_kind.empty_state()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut values = if options.require_training_values {
            Some(Vec::with_capacity(batch.size()))
        } else {
            None
        };

        let (vec_res, res_jac) = self.evaluate(batch)?;
        let effective_weights: Vec<f64> = match res_jac {
            Some(jacobians) => {
                if jacobians.len() != vec_res.len() {
                    return Err(EvalError::eval(format!(
                        "jacobian length mismatch: jacobians has {}, values has {}",
                        jacobians.len(),
                        vec_res.len()
                    )));
                }
                weights
                    .iter()
                    .copied()
                    .zip(jacobians.into_iter())
                    .map(|(weight, jac)| {
                        if jac == 0.0 {
                            Err(EvalError::eval(
                                "Jacobian is zero at this point, cannot divide by zero.",
                            ))
                        } else {
                            Ok(weight / jac)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            None => weights.to_vec(),
        };

        let observable = match self.observable_kind {
            SemanticObservableKind::Scalar => {
                let mut observable = ScalarObservableState::default();
                for (sample_idx, value) in vec_res.iter().enumerate() {
                    observable.add_sample(
                        self.training_projection.project(*value),
                        effective_weights[sample_idx],
                    );
                    if let Some(values) = values.as_mut() {
                        values.push(self.training_projection.project(*value));
                    }
                }
                ObservableState::Scalar(observable)
            }
            SemanticObservableKind::Complex => {
                let mut observable = ComplexObservableState::default();
                for (sample_idx, value) in vec_res.iter().enumerate() {
                    observable.add_sample(*value, effective_weights[sample_idx]);
                    if let Some(values) = values.as_mut() {
                        values.push(self.training_projection.project(*value));
                    }
                }
                ObservableState::Complex(observable)
            }
        };

        let weighted_values = values.map(|values| {
            values
                .into_iter()
                .zip(effective_weights.iter().copied())
                .map(|(value, weight)| value * weight)
                .collect()
        });
        Ok(BatchResult::new(weighted_values, observable))
    }

    fn get_init_metadata(&self) -> serde_json::Value {
        json!({
            "state_folder": self.state_folder,
            "process_id": self.process_id,
            "integrand_name": self.integrand_name,
            "momentum_space": self.momentum_space,
            "use_f128": self.use_f128,
            "training_projection": self.training_projection,
            "observable_kind": self.observable_kind,
        })
    }
}
