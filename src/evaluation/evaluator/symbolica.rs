use crate::core::EngineResultExt;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::PathBuf,
};

use crate::utils::domain::Domain;
use crate::{
    Batch, BatchResult, BuildError, EngineError, EvalError,
    core::AccumulatorConfig,
    evaluation::{AccumulatorState, IngestScalar},
    evaluation::{EvalBatchOptions, Evaluator, ingest_scalar_values},
    resources::resolve_resource_path,
    runtime_context::evaluator_tmp_dir,
};
use serde::{Deserialize, Serialize};
use symbolica::evaluate::{
    BatchEvaluator, CompileOptions, CompiledComplexEvaluator, FunctionMap, OptimizationSettings,
};
use symbolica::parser::ParseSettings;
use symbolica::{
    atom::{Atom, AtomCore},
    evaluate::ExportSettings,
};
use symbolica::{
    domains::{float::Complex as SymbolicaComplex, rational::Rational},
    namespace,
};
use tempfile::TempDir;

pub struct SymbolicaEngine {
    eval: CompiledComplexEvaluator,
    _parsed_expr: Option<Atom>,
    _expr: Option<String>,
    _constants: BTreeMap<String, toml::Value>,
    args: Vec<String>,
    input_plan: Vec<SymbolicaInputSlot>,
    _artifacts_dir: Option<TempDir>,
}

impl SymbolicaEngine {
    fn new(
        eval: CompiledComplexEvaluator,
        _parsed_expr: Option<Atom>,
        _expr: Option<String>,
        _constants: BTreeMap<String, toml::Value>,
        args: Vec<String>,
        input_plan: Vec<SymbolicaInputSlot>,
        artifacts_dir: Option<TempDir>,
    ) -> Self {
        SymbolicaEngine {
            eval,
            _parsed_expr,
            _expr,
            _constants,
            args,
            input_plan,
            _artifacts_dir: artifacts_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolicaParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constants: BTreeMap<String, toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SymbolicaSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiled_args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SymbolicaSource {
    Expression {
        expr: String,
    },
    Compiled {
        path: PathBuf,
        name: String,
        #[serde(default = "default_symbolica_outputs")]
        outputs: usize,
    },
}

fn default_symbolica_outputs() -> usize {
    1
}

#[derive(Clone)]
enum SymbolicaInputSlot {
    Sample(usize),
    Fixed(SymbolicaComplex<f64>),
}

impl SymbolicaEngine {
    pub fn from_params(params: SymbolicaParams) -> Result<Self, crate::BuildError> {
        let settings = ParseSettings::symbolica();

        let mut args = Vec::with_capacity(params.args.len());
        for arg in &params.args {
            let parsed = Atom::parse(&arg, namespace!(), settings.clone()).build_err()?;
            args.push(parsed);
        }

        let source = match params.source.clone() {
            Some(source) => source,
            None => SymbolicaSource::Expression {
                expr: params.expr.clone().ok_or_else(|| {
                    BuildError::invalid_input(
                        "symbolica evaluator requires either top-level expr or source",
                    )
                })?,
            },
        };

        match source {
            SymbolicaSource::Expression { expr } => {
                if params.compiled_args.is_some() {
                    return Err(BuildError::invalid_input(
                        "symbolica compiled_args is only valid with source.kind = \"compiled\"",
                    ));
                }
                if !params.bindings.is_empty() {
                    return Err(BuildError::invalid_input(
                        "symbolica bindings are only valid with source.kind = \"compiled\"",
                    ));
                }

                // Keep these plain parser calls unless updating Symbolica requires
                // default-namespace parsing for generated benchmark expressions.
                let parsed_expr = Atom::parse(&expr, namespace!(), settings.clone()).build_err()?;
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
                let input_plan = (0..params.args.len())
                    .map(SymbolicaInputSlot::Sample)
                    .collect();

                Ok(SymbolicaEngine::new(
                    evaluator,
                    Some(parsed_expr),
                    Some(expr),
                    params.constants,
                    params.args,
                    input_plan,
                    Some(artifacts_dir),
                ))
            }
            SymbolicaSource::Compiled {
                path,
                name,
                outputs,
            } => {
                if outputs != 1 {
                    return Err(BuildError::invalid_input(format!(
                        "symbolica compiled source currently supports exactly one complex output, got {outputs}"
                    )));
                }
                if !params.constants.is_empty() {
                    return Err(BuildError::invalid_input(
                        "symbolica constants are only valid with expression sources; use bindings for compiled source fixed inputs",
                    ));
                }
                let compiled_args = params
                    .compiled_args
                    .clone()
                    .unwrap_or_else(|| params.args.clone());
                let input_plan = build_input_plan(&params.args, &compiled_args, &params.bindings)?;
                let path = resolve_resource_path(&path).map_err(|err| {
                    BuildError::build(format!(
                        "failed to resolve symbolica compiled evaluator '{}': {err}",
                        path.display()
                    ))
                })?;
                if !path.is_file() {
                    return Err(BuildError::build(format!(
                        "symbolica compiled evaluator does not exist at {}; build or copy the configured shared library into a resource root (for the bundled variable_theta example run `just symbolica-variable-theta`)",
                        path.display()
                    )));
                }
                let evaluator = CompiledComplexEvaluator::load(&path, &name).map_err(|err| {
                    BuildError::build(format!(
                        "failed to load symbolica compiled evaluator '{}' from {}: {err}",
                        name,
                        path.display()
                    ))
                })?;

                Ok(SymbolicaEngine::new(
                    evaluator,
                    None,
                    None,
                    params.constants,
                    params.args,
                    input_plan,
                    None,
                ))
            }
        }
    }
}

fn build_input_plan(
    sampled_args: &[String],
    compiled_args: &[String],
    bindings: &BTreeMap<String, toml::Value>,
) -> Result<Vec<SymbolicaInputSlot>, BuildError> {
    let sampled_index = sampled_args
        .iter()
        .enumerate()
        .map(|(idx, name)| (name.as_str(), idx))
        .collect::<HashMap<_, _>>();
    let mut plan = Vec::with_capacity(compiled_args.len());
    for name in compiled_args {
        if let Some(idx) = sampled_index.get(name.as_str()) {
            plan.push(SymbolicaInputSlot::Sample(*idx));
            continue;
        }
        let Some(value) = bindings.get(name) else {
            return Err(BuildError::invalid_input(format!(
                "symbolica compiled arg {name:?} is neither sampled nor provided in bindings"
            )));
        };
        plan.push(SymbolicaInputSlot::Fixed(fixed_input_value(name, value)?));
    }
    Ok(plan)
}

fn fixed_input_value(name: &str, value: &toml::Value) -> Result<SymbolicaComplex<f64>, BuildError> {
    let value_expr = constant_value_expr(value)?;
    match value_expr.as_str() {
        "pi" | "Pi" | "PI" => {
            return Ok(SymbolicaComplex::new(std::f64::consts::PI, 0.0));
        }
        "i" | "I" => return Ok(SymbolicaComplex::new(0.0, 1.0)),
        _ => {}
    }

    let settings = ParseSettings::symbolica();
    let parsed_value = Atom::parse(&value_expr, namespace!(), settings).map_err(|err| {
        BuildError::build(format!(
            "invalid value for symbolica compiled binding {name:?}: {err}"
        ))
    })?;
    let value = SymbolicaComplex::<Rational>::try_from(&parsed_value).map_err(|err| {
        BuildError::build(format!(
            "symbolica compiled binding {name:?} must be a numeric real or complex value, got {value_expr:?}: {err}"
        ))
    })?;
    Ok(SymbolicaComplex::new(value.re.to_f64(), value.im.to_f64()))
}

impl SymbolicaParams {
    #[cfg(test)]
    fn expression(expr: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            expr: Some(expr.into()),
            args,
            constants: BTreeMap::new(),
            source: None,
            compiled_args: None,
            bindings: BTreeMap::new(),
        }
    }
}

fn build_function_map(
    constants: &BTreeMap<String, toml::Value>,
    settings: &ParseSettings,
) -> Result<FunctionMap, BuildError> {
    let mut function_map = FunctionMap::new();
    let imaginary_unit = SymbolicaComplex::new(Rational::from(0), Rational::from(1));
    function_map
        .add_aliases([
            (
                Atom::parse("i", namespace!(), settings.clone()).build_err()?,
                Atom::num(imaginary_unit.clone()),
            ),
            (
                Atom::parse("I", namespace!(), settings.clone()).build_err()?,
                Atom::num(imaginary_unit.clone()),
            ),
        ])
        .build_err()?;

    for (name, value) in constants {
        let key = Atom::parse(&name, namespace!(), settings.clone()).map_err(|err| {
            BuildError::build(format!("invalid symbolica constant {name:?}: {err}"))
        })?;
        let value_expr = constant_value_expr(value)?;
        let parsed_value =
            Atom::parse(&value_expr, namespace!(), settings.clone()).map_err(|err| {
                BuildError::build(format!(
                    "invalid value for symbolica constant {name:?}: {err}"
                ))
            })?;
        let value = SymbolicaComplex::<Rational>::try_from(&parsed_value).map_err(|err| {
            BuildError::build(format!(
                "symbolica constant {name:?} must be a numeric real or complex value, got {value_expr:?}: {err}"
            ))
        })?;
        function_map
            .add_aliases(std::iter::once((key, Atom::num(value))))
            .build_err()?;
    }

    Ok(function_map)
}

fn constant_value_expr(value: &toml::Value) -> Result<String, BuildError> {
    match value {
        toml::Value::String(value) => {
            Ok(decimal_literal_to_rational_expr(value).unwrap_or_else(|| value.clone()))
        }
        toml::Value::Integer(value) => Ok(value.to_string()),
        toml::Value::Float(value) => {
            if !value.is_finite() {
                return Err(BuildError::invalid_input(format!(
                    "invalid Symbolica float {value}"
                )));
            }
            let value = value.to_string();
            Ok(decimal_literal_to_rational_expr(&value).unwrap_or(value))
        }
        other => Err(BuildError::invalid_input(format!(
            "symbolica constants must be numeric TOML values or numeric strings, got {other}"
        ))),
    }
}

fn decimal_literal_to_rational_expr(raw: &str) -> Option<String> {
    let raw = raw.trim().replace('_', "");
    if raw.is_empty() {
        return None;
    }
    let (negative, unsigned) = match raw.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, raw.strip_prefix('+').unwrap_or(&raw)),
    };
    let exponent_start = unsigned.find(['e', 'E']);
    let (mantissa, exponent) = match exponent_start {
        Some(index) => (
            &unsigned[..index],
            unsigned[index + 1..].parse::<i32>().ok()?,
        ),
        None => (unsigned, 0),
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    if !integer.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    if fraction.is_empty() && exponent == 0 {
        return None;
    }

    let mut digits = format!("{integer}{fraction}");
    while digits.starts_with('0') && digits.len() > 1 {
        digits.remove(0);
    }
    if digits.chars().all(|ch| ch == '0') {
        return Some("0".to_string());
    }

    let decimal_places = fraction.len() as i32 - exponent;
    let sign = if negative { "-" } else { "" };
    if decimal_places <= 0 {
        digits.extend(std::iter::repeat('0').take((-decimal_places) as usize));
        return Some(format!("{sign}{digits}"));
    }
    let denominator = format!("1{}", "0".repeat(decimal_places as usize));
    Some(format!("{sign}{digits}/{denominator}"))
}

fn compile_complex_evaluator(
    expr: &Atom,
    args: &[Atom],
    function_map: &FunctionMap,
    artifacts_dir: &TempDir,
) -> Result<CompiledComplexEvaluator, BuildError> {
    let evaluator = expr
        .evaluator(args)
        .function_map(function_map.clone())
        .optimization_settings(OptimizationSettings::default())
        .build()
        .build_err()?
        .map_coeff(&|x| SymbolicaComplex::new(x.re.to_f64(), x.im.to_f64()));
    let stem = "eval";
    let path = artifacts_dir.path().join(stem);
    let exported_code = evaluator
        .export_cpp::<SymbolicaComplex<f64>>(
            path.with_extension("cpp"),
            stem,
            ExportSettings::default(),
        )
        .build_err()?;
    let compiled_code = exported_code
        .compile(path.with_extension("so"), CompileOptions::default())
        .build_err()?;
    compiled_code.load().build_err()
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
        let mut continuous = Vec::with_capacity(batch.size() * self.input_plan.len());
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
            for slot in &self.input_plan {
                match slot {
                    SymbolicaInputSlot::Sample(idx) => {
                        continuous.push(SymbolicaComplex::new(point.continuous[*idx], 0.0));
                    }
                    SymbolicaInputSlot::Fixed(value) => continuous.push(value.clone()),
                }
            }
        }

        let mut observable_state = AccumulatorState::from_config(accumulator);

        let mut out = vec![SymbolicaComplex::new(0.0, 0.0); batch.size()];
        self.eval
            .evaluate_batch(batch.size(), &continuous, &mut out)
            .map_err(|err| EngineError::Eval(err.to_string()))?;
        let has_imaginary_part = out.iter().any(|value| value.im != 0.0);

        // Multi-component (real/imag) accumulators take the vector path; a
        // single-component vector is scalar sugar and falls through below.
        if matches!(&observable_state, AccumulatorState::Vector(vector) if vector.components.len() >= 2)
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
            AccumulatorState::Vector(accumulator) => accumulator,
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
    use super::{
        SymbolicaEngine, SymbolicaParams, SymbolicaSource, compile_complex_evaluator, namespace,
    };
    use crate::core::AccumulatorConfig;
    use crate::evaluation::{Batch, EvalBatchOptions, Evaluator, Point};
    use std::collections::BTreeMap;
    use symbolica::atom::Atom;
    use symbolica::evaluate::FunctionMap;
    use symbolica::parser::ParseSettings;
    use symbolica::wrap_input;

    #[test]
    fn symbolica_manual_eval_has_two_expected_peaks_with_decimal_centers() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams::expression(
            "1/((x-0.25)^2+(y-0.25)^2+1/40) + 1/((x-0.75)^2+(y-0.75)^2+1/40) + z",
            vec!["x".to_string(), "y".to_string(), "z".to_string()],
        ))
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
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams::expression(
            "1/((x-1/4)^2+(y-1/4)^2+1/40) + 1/((x-3/4)^2+(y-3/4)^2+1/40) + z",
            vec!["x".to_string(), "y".to_string(), "z".to_string()],
        ))
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
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams::expression(
            "x + i*y",
            vec!["x".to_string(), "y".to_string()],
        ))
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
        let mut params = SymbolicaParams::expression("scale * (x - center)", vec!["x".to_string()]);
        params.constants = constants;
        let mut evaluator =
            SymbolicaEngine::from_params(params).expect("build symbolica evaluator");

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
    fn symbolica_constants_accept_toml_floats_and_decimal_strings() {
        let mut constants = BTreeMap::new();
        constants.insert("scale".to_string(), toml::Value::Float(0.5));
        constants.insert("one".to_string(), toml::Value::Float(1.0));
        constants.insert(
            "offset".to_string(),
            toml::Value::String("1e-1".to_string()),
        );
        let mut params =
            SymbolicaParams::expression("one * (scale * x + offset)", vec!["x".to_string()]);
        params.constants = constants;
        let mut evaluator =
            SymbolicaEngine::from_params(params).expect("build symbolica evaluator");

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
        assert_eq!(result.values.expect("training values"), vec![0.6]);
    }

    #[test]
    fn symbolica_compiled_source_loads_with_bound_fixed_inputs() {
        let settings = ParseSettings::symbolica();
        let expr = Atom::parse("a + k0 + k1 + k2", namespace!(), settings.clone())
            .expect("parse expression");
        let args = ["a", "k0", "k1", "k2"]
            .iter()
            .map(|arg| Atom::parse(&arg, namespace!(), settings.clone()).expect("parse arg"))
            .collect::<Vec<_>>();
        let artifacts_dir = tempfile::tempdir().expect("create artifacts dir");
        let _compiled =
            compile_complex_evaluator(&expr, &args, &FunctionMap::new(), &artifacts_dir)
                .expect("compile source evaluator");

        let mut bindings = BTreeMap::new();
        bindings.insert("a".to_string(), toml::Value::Float(0.5));
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: None,
            args: vec!["k0".to_string(), "k1".to_string(), "k2".to_string()],
            constants: BTreeMap::new(),
            source: Some(SymbolicaSource::Compiled {
                path: artifacts_dir.path().join("eval.so"),
                name: "eval".to_string(),
                outputs: 1,
            }),
            compiled_args: Some(vec![
                "a".to_string(),
                "k0".to_string(),
                "k1".to_string(),
                "k2".to_string(),
            ]),
            bindings,
        })
        .expect("load compiled evaluator");

        let batch = Batch::from_points([Point::new(vec![1.0, 2.0, 3.0], Vec::new(), 1.0)])
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
        assert_eq!(result.values.expect("training values"), vec![6.5]);
    }

    #[test]
    fn symbolica_complex_expr_rejects_scalar_accumulator() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams::expression(
            "x + i*y",
            vec!["x".to_string(), "y".to_string()],
        ))
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
