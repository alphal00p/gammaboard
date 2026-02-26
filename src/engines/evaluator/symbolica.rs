use crate::{
    Batch, BatchResult, BuildError, EngineError, EvalError, PointSpec,
    engines::{BuildFromJson, Evaluator, observable::ObservableFactory},
};
use serde::Deserialize;
use symbolica::evaluate::{
    BatchEvaluator, CompileOptions, CompiledRealEvaluator, FunctionMap, OptimizationSettings,
};
use symbolica::parser::ParseSettings;
use symbolica::wrap_input;
use symbolica::{
    atom::{Atom, AtomCore},
    evaluate::ExportSettings,
};

struct SymbolicaEngine {
    eval: CompiledRealEvaluator,
    n_args: usize,
}

impl SymbolicaEngine {
    fn new(eval: CompiledRealEvaluator, n_args: usize) -> Self {
        SymbolicaEngine { eval, n_args }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
struct SymbolicaParams {
    expr: String,
    args: Vec<String>,
}

impl BuildFromJson for SymbolicaEngine {
    type Params = SymbolicaParams;

    fn from_parsed_params(params: Self::Params) -> Result<Self, crate::BuildError> {
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

        let exported_code = evaluator
            .export_cpp::<f64>(".evaluators/test.cpp", "test", ExportSettings::default())
            .map_err(|err| BuildError::Build(err.to_string()))?;

        let compiled_code = exported_code
            .compile(".evaluators/test.so", CompileOptions::default())
            .map_err(|err| BuildError::Build(err.to_string()))?;

        let evaluator = compiled_code
            .load()
            .map_err(|err| BuildError::Build(err.to_string()))?;

        Ok(SymbolicaEngine::new(evaluator, args.len()))
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
        let slice = batch.continuous().as_slice().expect("standard order");

        let mut observable = observable_factory.build()?;

        let mut out = vec![0.0; batch.size()];
        self.eval
            .evaluate_batch(batch.size(), slice, &mut out)
            .map_err(|err| EngineError::Eval(err.to_string()))?;

        {
            let scalar_ingest = observable.as_scalar_ingest().ok_or_else(|| {
                EvalError::Engine(format!(
                    "symbolica evaluator supports only scalar-capable observables, got {}",
                    observable_factory.implementation
                ))
            })?;
            for (i, v) in out.iter().enumerate() {
                scalar_ingest.ingest_scalar(*v, batch.weights()[i]);
            }
        }
        BatchResult::from_values_weights_and_observable(
            out,
            batch.weights().as_slice().expect("standard order"),
            observable.as_ref(),
        )
    }

    fn supports_observable(&self, observable_factory: &ObservableFactory) -> bool {
        match observable_factory.build() {
            Ok(mut observable) => observable.as_scalar_ingest().is_some(),
            Err(_) => false,
        }
    }
}
