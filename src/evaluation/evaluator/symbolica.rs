use std::{fs, path::Path};

use crate::{
    Batch, BatchResult, BuildError, EngineError, EvalError, PointSpec,
    core::ObservableConfig,
    evaluation::{EvalBatchOptions, Evaluator},
    evaluation::{IngestScalar, ObservableState},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use symbolica::wrap_input;
use symbolica::{
    atom::{Atom, AtomCore},
    evaluate::ExportSettings,
};
use symbolica::{
    evaluate::{
        BatchEvaluator, CompileOptions, CompiledRealEvaluator, FunctionMap, OptimizationSettings,
    },
    printer::PrintOptions,
};
use symbolica::{parser::ParseSettings, printer::PrintState};
use tempfile::TempDir;

pub struct SymbolicaEngine {
    eval: CompiledRealEvaluator,
    parsed_expr: Atom,
    expr: String,
    args: Vec<String>,
    _artifacts_dir: TempDir,
}

impl SymbolicaEngine {
    fn new(
        eval: CompiledRealEvaluator,
        parsed_expr: Atom,
        expr: String,
        args: Vec<String>,
        artifacts_dir: TempDir,
    ) -> Self {
        SymbolicaEngine {
            eval,
            parsed_expr,
            expr,
            args,
            _artifacts_dir: artifacts_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolicaParams {
    pub expr: String,
    pub args: Vec<String>,
}

impl SymbolicaEngine {
    pub fn from_params(params: SymbolicaParams) -> Result<Self, crate::BuildError> {
        let settings = ParseSettings::symbolica();
        let parsed_expr = Atom::parse(wrap_input!(&params.expr), settings.clone())
            .map_err(|err| BuildError::build(err.to_string()))?;

        let mut args = Vec::with_capacity(params.args.len());
        for arg in &params.args {
            let parsed = Atom::parse(wrap_input!(arg), settings.clone())
                .map_err(|err| BuildError::build(err.to_string()))?;
            args.push(parsed);
        }

        let evaluator = parsed_expr
            .evaluator(
                &FunctionMap::default(),
                &args,
                OptimizationSettings::default(),
            )
            .map_err(|err| BuildError::build(err.to_string()))?;

        let root_artifacts_dir = Path::new("./.evaluators");
        fs::create_dir_all(root_artifacts_dir)?;

        let artifacts_dir = tempfile::Builder::new()
            .prefix("symbolica-eval-")
            .rand_bytes(8)
            .tempdir_in(root_artifacts_dir)
            .map_err(|err| BuildError::io(err.to_string()))?;
        let stem = "eval";
        let path = artifacts_dir.path().join(stem);

        let exported_code = evaluator
            .export_cpp::<f64>(path.with_extension("cpp"), &stem, ExportSettings::default())
            .map_err(|err| BuildError::build(err.to_string()))?;

        let compiled_code = exported_code
            .compile(path.with_extension("so"), CompileOptions::default())
            .map_err(|err| BuildError::build(err.to_string()))?;

        let evaluator = compiled_code
            .load()
            .map_err(|err| BuildError::build(err.to_string()))?;

        Ok(SymbolicaEngine::new(
            evaluator,
            parsed_expr,
            params.expr,
            params.args.clone(),
            artifacts_dir,
        ))
    }
}

impl Evaluator for SymbolicaEngine {
    fn get_point_spec(&self) -> PointSpec {
        PointSpec {
            continuous_dims: self.args.len(),
            discrete_dims: 0,
        }
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let continuous = batch.continuous().as_slice().ok_or_else(|| {
            EvalError::Engine("Batch continuous array must be standard-layout".to_string())
        })?;
        let weights = batch.weights().as_slice().ok_or_else(|| {
            EvalError::Engine("Batch weights array must be standard-layout".to_string())
        })?;

        let mut observable_state = ObservableState::from_config(observable);

        let mut out = vec![0.0; batch.size()];
        self.eval
            .evaluate_batch(batch.size(), continuous, &mut out)
            .map_err(|err| EngineError::Eval(err.to_string()))?;

        match &mut observable_state {
            ObservableState::Scalar(observable) => {
                for (value, weight) in out.iter().zip(weights.iter()) {
                    observable.ingest_scalar(*value, *weight);
                }
            }
            ObservableState::FullScalar(observable) => {
                for (value, weight) in out.iter().zip(weights.iter()) {
                    observable.ingest_scalar(*value, *weight);
                }
            }
            other => {
                return Err(EvalError::eval(format!(
                    "symbolica evaluator does not support observable kind {}",
                    other.kind_str()
                )));
            }
        }
        if options.require_training_values {
            let weighted_values = out
                .into_iter()
                .zip(weights.iter().copied())
                .map(|(value, weight)| value * weight)
                .collect();
            Ok(BatchResult::new(Some(weighted_values), observable_state))
        } else {
            Ok(BatchResult::new(None, observable_state))
        }
    }

    fn get_init_metadata(&self) -> JsonValue {
        let mut str = String::new();
        _ = self
            .parsed_expr
            .format(&mut str, &PrintOptions::latex(), PrintState::new());

        json!({
            "expr": &self.expr,
            "args": &self.args,
            "expr_latex": str,
        })
    }
}
