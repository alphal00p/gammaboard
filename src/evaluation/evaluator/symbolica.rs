use std::{collections::BTreeMap, fs};

use crate::utils::domain::Domain;
use crate::{
    Batch, BatchResult, BuildError, EngineError, EvalError,
    core::AccumulatorConfig,
    evaluation::{AccumulatorState, IngestScalar},
    evaluation::{EvalBatchOptions, Evaluator, ingest_scalar_values},
    runtime_context::evaluator_tmp_dir,
};
use serde::{Deserialize, Serialize};
use symbolica::domains::{float::Complex as SymbolicaComplex, rational::Rational};
use symbolica::evaluate::{
    BatchEvaluator, CompileOptions, CompiledComplexEvaluator, FunctionMap, OptimizationSettings,
};
use symbolica::parser::ParseSettings;
use symbolica::wrap_input;
use symbolica::{
    atom::{Atom, AtomCore},
    evaluate::ExportSettings,
};
use tempfile::TempDir;

pub struct SymbolicaEngine {
    eval: CompiledComplexEvaluator,
    _parsed_expr: Atom,
    _expr: String,
    _constants: BTreeMap<String, toml::Value>,
    args: Vec<String>,
    _artifacts_dir: TempDir,
}

impl SymbolicaEngine {
    fn new(
        eval: CompiledComplexEvaluator,
        _parsed_expr: Atom,
        _expr: String,
        _constants: BTreeMap<String, toml::Value>,
        args: Vec<String>,
        artifacts_dir: TempDir,
    ) -> Self {
        SymbolicaEngine {
            eval,
            _parsed_expr,
            _expr,
            _constants,
            args,
            _artifacts_dir: artifacts_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolicaParams {
    pub expr: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, toml::Value>,
}

impl SymbolicaEngine {
    pub fn from_params(params: SymbolicaParams) -> Result<Self, crate::BuildError> {
        let settings = ParseSettings::symbolica();
        // Keep these plain parser calls unless updating Symbolica requires
        // default-namespace parsing for generated benchmark expressions.
        let parsed_expr = Atom::parse(wrap_input!(&params.expr), settings.clone())
            .map_err(|err| BuildError::build(err.to_string()))?;

        let mut args = Vec::with_capacity(params.args.len());
        for arg in &params.args {
            let parsed = Atom::parse(wrap_input!(arg), settings.clone())
                .map_err(|err| BuildError::build(err.to_string()))?;
            args.push(parsed);
        }

        let root_artifacts_dir = evaluator_tmp_dir("symbolica").map_err(|err| {
            BuildError::build(format!("failed to resolve evaluator tmp dir: {err}"))
        })?;
        fs::create_dir_all(&root_artifacts_dir)?;

        let artifacts_dir = tempfile::Builder::new()
            .prefix("symbolica-eval-")
            .rand_bytes(8)
            .tempdir_in(&root_artifacts_dir)
            .map_err(|err| BuildError::io(err.to_string()))?;
        let function_map = build_function_map(&params.constants, &settings)?;
        let evaluator =
            compile_complex_evaluator(&parsed_expr, &args, &function_map, &artifacts_dir)?;

        Ok(SymbolicaEngine::new(
            evaluator,
            parsed_expr,
            params.expr,
            params.constants,
            params.args.clone(),
            artifacts_dir,
        ))
    }
}

fn build_function_map(
    constants: &BTreeMap<String, toml::Value>,
    settings: &ParseSettings,
) -> Result<FunctionMap, BuildError> {
    let mut function_map = FunctionMap::new();
    let imaginary_unit = SymbolicaComplex::new(Rational::from(0), Rational::from(1));
    function_map.add_constant(
        Atom::parse(wrap_input!("i"), settings.clone())
            .map_err(|err| BuildError::build(err.to_string()))?,
        imaginary_unit.clone(),
    );
    function_map.add_constant(
        Atom::parse(wrap_input!("I"), settings.clone())
            .map_err(|err| BuildError::build(err.to_string()))?,
        imaginary_unit,
    );

    for (name, value) in constants {
        let key = Atom::parse(wrap_input!(name), settings.clone()).map_err(|err| {
            BuildError::build(format!("invalid symbolica constant {name:?}: {err}"))
        })?;
        let value_expr = constant_value_expr(value)?;
        let parsed_value =
            Atom::parse(wrap_input!(&value_expr), settings.clone()).map_err(|err| {
                BuildError::build(format!(
                    "invalid value for symbolica constant {name:?}: {err}"
                ))
            })?;
        let value = SymbolicaComplex::<Rational>::try_from(&parsed_value).map_err(|err| {
            BuildError::build(format!(
                "symbolica constant {name:?} must be a numeric real or complex value, got {value_expr:?}: {err}"
            ))
        })?;
        function_map.add_constant(key, value);
    }

    Ok(function_map)
}

fn constant_value_expr(value: &toml::Value) -> Result<String, BuildError> {
    match value {
        toml::Value::String(value) => Ok(value.clone()),
        toml::Value::Integer(value) => Ok(value.to_string()),
        toml::Value::Float(_) => Err(BuildError::invalid_input(
            "symbolica constants must be exact Symbolica numeric values; use integers or strings like \"1/2\" instead of TOML floats",
        )),
        other => Err(BuildError::invalid_input(format!(
            "symbolica constants must be exact Symbolica numeric values; use integers or strings like \"1/2\", got {other}"
        ))),
    }
}

fn compile_complex_evaluator(
    expr: &Atom,
    args: &[Atom],
    function_map: &FunctionMap,
    artifacts_dir: &TempDir,
) -> Result<CompiledComplexEvaluator, BuildError> {
    let evaluator = expr
        .evaluator(function_map, args, OptimizationSettings::default())
        .map_err(|err| BuildError::build(err.to_string()))?
        .map_coeff(&|x| SymbolicaComplex::new(x.re.to_f64(), x.im.to_f64()));
    let stem = "eval";
    let path = artifacts_dir.path().join(stem);
    let exported_code = evaluator
        .export_cpp::<SymbolicaComplex<f64>>(
            path.with_extension("cpp"),
            stem,
            ExportSettings::default(),
        )
        .map_err(|err| BuildError::build(err.to_string()))?;
    let compiled_code = exported_code
        .compile(path.with_extension("so"), CompileOptions::default())
        .map_err(|err| BuildError::build(err.to_string()))?;
    compiled_code
        .load()
        .map_err(|err| BuildError::build(err.to_string()))
}

impl Evaluator for SymbolicaEngine {
    fn get_domain(&self) -> Domain {
        Domain::continuous(self.args.len())
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let width = self.args.len();
        let mut continuous = Vec::with_capacity(batch.size() * width);
        for (sample_idx, point) in batch.points().iter().enumerate() {
            if point.continuous.len() != width {
                return Err(EvalError::Engine(format!(
                    "symbolica evaluator expects {} continuous coordinates, got {} for sample {}",
                    width,
                    point.continuous.len(),
                    sample_idx
                )));
            }
            if !point.discrete.is_empty() {
                return Err(EvalError::Engine(format!(
                    "symbolica evaluator does not support discrete coordinates, got {:?} for sample {}",
                    point.discrete, sample_idx
                )));
            }
            continuous.extend(
                point
                    .continuous
                    .iter()
                    .map(|value| SymbolicaComplex::new(*value, 0.0)),
            );
        }

        let mut observable_state = AccumulatorState::from_config(accumulator);

        let mut out = vec![SymbolicaComplex::new(0.0, 0.0); batch.size()];
        self.eval
            .evaluate_batch(batch.size(), &continuous, &mut out)
            .map_err(|err| EngineError::Eval(err.to_string()))?;
        let has_imaginary_part = out.iter().any(|value| value.im != 0.0);

        if matches!(&observable_state, AccumulatorState::Vector(_))
            || matches!(
                &observable_state,
                AccumulatorState::FullVector(full_vector)
                    if full_vector.components == ["real".to_string(), "imag".to_string()]
            )
        {
            let mut training_values = options
                .require_training_values
                .then(|| Vec::with_capacity(batch.size()));
            match &mut observable_state {
                AccumulatorState::Vector(vector) => {
                    for (idx, point) in batch.points().iter().enumerate() {
                        let value = out[idx];
                        let projected = vector
                            .ingest_vector(&[value.re, value.im], point)
                            .map_err(EvalError::eval)?;
                        if let Some(training_values) = training_values.as_mut() {
                            training_values.push(projected * point.total_weight());
                        }
                    }
                }
                AccumulatorState::FullVector(full_vector) => {
                    for (idx, point) in batch.points().iter().enumerate() {
                        let value = out[idx];
                        let weight = point.total_weight().abs();
                        full_vector.push_vector(&[value.re * weight, value.im * weight]);
                    }
                    training_values = None;
                }
                _ => unreachable!(),
            }
            return Ok(BatchResult::new(training_values, observable_state));
        }

        if has_imaginary_part {
            return Err(EvalError::eval(
                "complex symbolica expression requires a vector or full_vector accumulator with components [\"real\", \"imag\"]",
            ));
        }

        let out_re = out.iter().map(|value| value.re).collect::<Vec<_>>();
        let scalar_accumulator: &mut dyn IngestScalar = match &mut observable_state {
            AccumulatorState::Empty(accumulator) => accumulator,
            AccumulatorState::Scalar(accumulator) => accumulator,
            AccumulatorState::FullVector(accumulator) => accumulator,
            other => {
                return Err(EvalError::eval(format!(
                    "symbolica evaluator does not support accumulator kind {}",
                    other.kind_str()
                )));
            }
        };
        let weighted_values = ingest_scalar_values(
            &out_re,
            batch.points(),
            options.require_training_values,
            scalar_accumulator,
        )?;
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

#[cfg(test)]
mod tests {
    use super::{SymbolicaEngine, SymbolicaParams};
    use crate::core::AccumulatorConfig;
    use crate::evaluation::{Batch, EvalBatchOptions, Evaluator, Point};
    use std::collections::BTreeMap;

    #[test]
    fn symbolica_manual_eval_has_two_expected_peaks_with_decimal_centers() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: "1/((x-0.25)^2+(y-0.25)^2+1/40) + 1/((x-0.75)^2+(y-0.75)^2+1/40) + z".to_string(),
            args: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            constants: BTreeMap::new(),
        })
        .expect("build symbolica evaluator");

        let points = [
            (0.25, 0.25, 0.0), // expected peak 1
            (0.75, 0.75, 0.0), // expected peak 2
            (0.25, 0.75, 0.0),
            (0.75, 0.25, 0.0),
            (0.50, 0.50, 0.0),
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 0.0),
        ];
        let batch = Batch::from_points(
            points
                .iter()
                .map(|(x, y, z)| Point::new(vec![*x, *y, *z], Vec::new(), 1.0)),
        )
        .expect("build batch");

        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("evaluate batch");
        let values = result.values.expect("training values present");
        assert_eq!(values.len(), points.len());

        let v_peak_1 = values[0];
        let v_peak_2 = values[1];
        for (idx, value) in values.iter().enumerate().skip(2) {
            assert!(
                v_peak_1 > *value,
                "expected first peak to dominate sample {idx}: peak={v_peak_1}, sample={value}"
            );
            assert!(
                v_peak_2 > *value,
                "expected second peak to dominate sample {idx}: peak={v_peak_2}, sample={value}"
            );
        }
    }

    #[test]
    fn symbolica_fraction_literals_match_expected_peak_locations() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: "1/((x-1/4)^2+(y-1/4)^2+1/40) + 1/((x-3/4)^2+(y-3/4)^2+1/40) + z".to_string(),
            args: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            constants: BTreeMap::new(),
        })
        .expect("build symbolica evaluator");

        let points = [(0.0, 0.0, 0.0), (0.25, 0.25, 0.0), (0.75, 0.75, 0.0)];
        let batch = Batch::from_points(
            points
                .iter()
                .map(|(x, y, z)| Point::new(vec![*x, *y, *z], Vec::new(), 1.0)),
        )
        .expect("build batch");

        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("evaluate batch");
        let values = result.values.expect("training values present");

        assert!(values[1] > values[0], "quarter peak should exceed origin");
        assert!(
            values[2] > values[0],
            "three-quarter peak should exceed origin"
        );
    }

    #[test]
    fn symbolica_complex_eval_fills_real_imag_full_vector() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: "x + i*y".to_string(),
            args: vec!["x".to_string(), "y".to_string()],
            constants: BTreeMap::new(),
        })
        .expect("build symbolica evaluator");

        let batch = Batch::from_points([
            Point::new(vec![1.0, 2.0], Vec::new(), 1.0),
            Point::new(vec![3.0, 4.0], Vec::new(), 1.0),
        ])
        .expect("build batch");
        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::FullVector {
                    components: vec!["real".to_string(), "imag".to_string()],
                },
                EvalBatchOptions {
                    require_training_values: false,
                },
            )
            .expect("evaluate batch");
        let crate::evaluation::AccumulatorState::FullVector(state) = result.accumulator else {
            panic!("expected full vector accumulator");
        };
        assert_eq!(state.values_row_major, vec![1.0, 2.0, 3.0, 4.0]);
        assert!(result.values.is_none());
    }

    #[test]
    fn symbolica_constants_are_registered_in_function_map() {
        let mut constants = BTreeMap::new();
        constants.insert("scale".to_string(), toml::Value::Integer(3));
        constants.insert("center".to_string(), toml::Value::String("1/2".to_string()));
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: "scale * (x - center)".to_string(),
            args: vec!["x".to_string()],
            constants,
        })
        .expect("build symbolica evaluator");

        let batch =
            Batch::from_points([Point::new(vec![1.0], Vec::new(), 1.0)]).expect("build batch");
        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("evaluate batch");
        assert_eq!(result.values.expect("training values"), vec![1.5]);
    }

    #[test]
    fn symbolica_constants_reject_toml_floats() {
        let mut constants = BTreeMap::new();
        constants.insert("scale".to_string(), toml::Value::Float(0.5));
        let err = match SymbolicaEngine::from_params(SymbolicaParams {
            expr: "scale * x".to_string(),
            args: vec!["x".to_string()],
            constants,
        }) {
            Ok(_) => panic!("toml float constants should be rejected"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("instead of TOML floats"));
    }

    #[test]
    fn symbolica_complex_expr_rejects_scalar_accumulator() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: "x + i*y".to_string(),
            args: vec!["x".to_string(), "y".to_string()],
            constants: BTreeMap::new(),
        })
        .expect("build symbolica evaluator");

        let batch =
            Batch::from_points([Point::new(vec![1.0, 2.0], Vec::new(), 1.0)]).expect("build batch");
        let err = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: false,
                },
            )
            .expect_err("complex output should reject scalar accumulator");
        assert!(err.to_string().contains("complex symbolica expression"));
    }
}
