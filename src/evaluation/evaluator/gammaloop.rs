use std::path::PathBuf;

use gammaloop_api::state::{ProcessRef, State};
use gammalooprs::initialisation::initialise;
use gammalooprs::integrands::{inspect::inspect, process::ProcessIntegrand};
use gammalooprs::model::Model;
use gammalooprs::settings::RuntimeSettings;
use gammalooprs::utils::F;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Batch, BatchResult, BuildError, EvalError, PointSpec,
    engines::{
        ComplexObservableState, ObservableState, ScalarObservableState, SemanticObservableKind,
    },
    engines::{EvalBatchOptions, Evaluator},
};

pub struct GammaLoopEvaluator {
    integrand: ProcessIntegrand,
    model: Model,
    settings: RuntimeSettings,
    state_folder: PathBuf,
    process_id: usize,
    integrand_name: String,
    training_projection: TrainingProjection,
    observable_kind: SemanticObservableKind,
    point_spec: PointSpec, //why?
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
    pub observable_kind: SemanticObservableKind,
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
            observable_kind: SemanticObservableKind::Complex,
            continuous_dims: 3,
            discrete_dims: 0,
        }
    }
}

impl GammaLoopEvaluator {
    pub fn from_params(params: GammaLoopParams) -> Result<Self, BuildError> {
        _ = initialise();
        let state = State::load(params.state_folder.clone(), None, None).map_err(|err| {
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
            training_projection: params.training_projection,
            observable_kind: params.observable_kind,
            point_spec: PointSpec {
                continuous_dims: params.continuous_dims,
                discrete_dims: params.discrete_dims,
            },
        })
    }

    fn evaluate(&mut self, batch: &Batch) -> Result<Vec<num::complex::Complex64>, EvalError> {
        let mut vec_res = vec![];

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

            let (_, value) = inspect(
                &self.settings,
                &mut self.integrand,
                &self.model,
                pt,
                discrete_dim.as_slice(),
                false,
                false,
                false,
            )
            .map_err(|err| EvalError::eval(format!("failed to evaluate integrand: {err}")))?;

            vec_res.push(num::complex::Complex64::new(
                value.re.into(),
                value.im.into(),
            ));
        }

        Ok(vec_res)
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

        let vec_res = self.evaluate(batch)?;

        let observable = match self.observable_kind {
            SemanticObservableKind::Scalar => {
                let mut observable = ScalarObservableState::default();
                for (sample_idx, value) in vec_res.iter().enumerate() {
                    observable.add_sample(
                        self.training_projection.project(*value),
                        weights[sample_idx],
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
                    observable.add_sample(*value, weights[sample_idx]);
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
                .zip(weights.iter().copied())
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
            "training_projection": self.training_projection,
            "observable_kind": self.observable_kind,
        })
    }
}
