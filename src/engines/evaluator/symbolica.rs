use std::{fs, path::Path};

use crate::{
    Batch, BatchResult, BuildError, EngineError, EvalError, PointSpec,
    engines::{BuildFromJson, Evaluator, observable::ObservableFactory},
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use symbolica::evaluate::{
    BatchEvaluator, CompileOptions, CompiledRealEvaluator, FunctionMap, OptimizationSettings,
};
use symbolica::parser::ParseSettings;
use symbolica::wrap_input;
use symbolica::{
    atom::{Atom, AtomCore},
    evaluate::ExportSettings,
};
use tempfile::TempDir;

pub struct SymbolicaEngine {
    eval: CompiledRealEvaluator,
    n_args: usize,
    expr: String,
    args: Vec<String>,
    _artifacts_dir: TempDir,
}

impl SymbolicaEngine {
    fn new(
        eval: CompiledRealEvaluator,
        n_args: usize,
        expr: String,
        args: Vec<String>,
        artifacts_dir: TempDir,
    ) -> Self {
        SymbolicaEngine {
            eval,
            n_args,
            expr,
            args,
            _artifacts_dir: artifacts_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct SymbolicaParams {
    expr: String,
    args: Vec<String>,
}

impl BuildFromJson for SymbolicaEngine {
    type Params = SymbolicaParams;

    fn from_parsed_params(params: Self::Params) -> Result<Self, crate::BuildError> {
        let expr = params.expr.clone();
        let arg_names = params.args.clone();
        let settings = ParseSettings::symbolica();

        let atom = Atom::parse(wrap_input!(&params.expr), settings.clone())
            .map_err(|err| BuildError::Build(err.to_string()))?;

        let mut args = Vec::with_capacity(params.args.len());
        for arg in &params.args {
            let parsed = Atom::parse(wrap_input!(arg), settings.clone())
                .map_err(|err| BuildError::Build(err.to_string()))?;
            args.push(parsed);
        }

        let evaluator = atom
            .evaluator(
                &FunctionMap::default(),
                &args,
                OptimizationSettings::default(),
            )
            .map_err(|err| BuildError::Build(err.to_string()))?;

        let root_artifacts_dir = Path::new("./.evaluators");
        fs::create_dir_all(root_artifacts_dir).map_err(|err| BuildError::Build(err.to_string()))?;

        let artifacts_dir = tempfile::Builder::new()
            .prefix("symbolica-eval-")
            .rand_bytes(8)
            .tempdir_in(root_artifacts_dir)
            .map_err(|err| BuildError::Build(err.to_string()))?;
        let stem = "eval";
        let path = artifacts_dir.path().join(stem);

        let exported_code = evaluator
            .export_cpp::<f64>(path.with_extension("cpp"), &stem, ExportSettings::default())
            .map_err(|err| BuildError::Build(err.to_string()))?;

        let compiled_code = exported_code
            .compile(path.with_extension("so"), CompileOptions::default())
            .map_err(|err| BuildError::Build(err.to_string()))?;

        let evaluator = compiled_code
            .load()
            .map_err(|err| BuildError::Build(err.to_string()))?;

        Ok(SymbolicaEngine::new(
            evaluator,
            args.len(),
            expr,
            arg_names,
            artifacts_dir,
        ))
    }
}

impl Evaluator for SymbolicaEngine {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.discrete_dims != 0 {
            Err(BuildError::Build(
                "Discrete dimensions are not supported".to_string(),
            ))
        } else if point_spec.continuous_dims != self.n_args {
            Err(BuildError::Build(format!(
                "Continuous dimensions need to match the number of arguments (n = {})",
                self.n_args
            )))
        } else {
            Ok(())
        }
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
    ) -> Result<BatchResult, EvalError> {
        let continuous = batch.continuous().as_slice().ok_or_else(|| {
            EvalError::Engine("Batch continuous array must be standard-layout".to_string())
        })?;
        let weights = batch.weights().as_slice().ok_or_else(|| {
            EvalError::Engine("Batch weights array must be standard-layout".to_string())
        })?;

        let mut observable = observable_factory.build()?;

        let mut out = vec![0.0; batch.size()];
        self.eval
            .evaluate_batch(batch.size(), continuous, &mut out)
            .map_err(|err| EngineError::Eval(err.to_string()))?;

        {
            let scalar_ingest = observable.as_scalar_ingest().ok_or_else(|| {
                EvalError::Engine(format!(
                    "symbolica evaluator supports only scalar-capable observables, got {}",
                    observable_factory.implementation
                ))
            })?;
            for (value, weight) in out.iter().zip(weights.iter()) {
                scalar_ingest.ingest_scalar(*value, *weight);
            }
        }
        BatchResult::from_values_weights_and_observable(out, weights, observable.as_ref())
    }

    fn supports_observable(&self, observable_factory: &ObservableFactory) -> bool {
        match observable_factory.build() {
            Ok(mut observable) => observable.as_scalar_ingest().is_some(),
            Err(_) => false,
        }
    }

    fn get_init_metadata(&self) -> JsonValue {
        json!({
            "expr": &self.expr,
            "args": &self.args,
        })
    }
}
