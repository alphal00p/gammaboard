use std::fs;

use crate::utils::domain::Domain;
use crate::{
    Batch, BatchResult, BuildError, EngineError, EvalError,
    core::AccumulatorConfig,
    evaluation::{AccumulatorState, IngestScalar},
    evaluation::{EvalBatchOptions, Evaluator, ingest_scalar_values},
    runtime_context::evaluator_tmp_dir,
};
use serde::{Deserialize, Serialize};
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
    eval_re: CompiledRealEvaluator,
    eval_im: Option<CompiledRealEvaluator>,
    _parsed_expr_re: Atom,
    _parsed_expr_im: Option<Atom>,
    _expr: String,
    _imag_expr: Option<String>,
    args: Vec<String>,
    _artifacts_dir: TempDir,
}

impl SymbolicaEngine {
    fn new(
        eval_re: CompiledRealEvaluator,
        eval_im: Option<CompiledRealEvaluator>,
        _parsed_expr_re: Atom,
        _parsed_expr_im: Option<Atom>,
        _expr: String,
        _imag_expr: Option<String>,
        args: Vec<String>,
        artifacts_dir: TempDir,
    ) -> Self {
        SymbolicaEngine {
            eval_re,
            eval_im,
            _parsed_expr_re,
            _parsed_expr_im,
            _expr,
            _imag_expr,
            args,
            _artifacts_dir: artifacts_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolicaParams {
    pub expr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imag_expr: Option<String>,
    pub args: Vec<String>,
}

impl SymbolicaEngine {
    pub fn from_params(params: SymbolicaParams) -> Result<Self, crate::BuildError> {
        let settings = ParseSettings::symbolica();
        // Keep these plain parser calls unless updating Symbolica requires
        // default-namespace parsing for generated benchmark expressions.
        let parsed_expr_re = Atom::parse(wrap_input!(&params.expr), settings.clone())
            .map_err(|err| BuildError::build(err.to_string()))?;
        let parsed_expr_im = params
            .imag_expr
            .as_ref()
            .map(|expr| {
                Atom::parse(wrap_input!(expr), settings.clone())
                    .map_err(|err| BuildError::build(err.to_string()))
            })
            .transpose()?;

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
        let evaluator_re =
            compile_real_evaluator(&parsed_expr_re, &args, &artifacts_dir, "eval_re")?;
        let evaluator_im = parsed_expr_im
            .as_ref()
            .map(|expr| compile_real_evaluator(expr, &args, &artifacts_dir, "eval_im"))
            .transpose()?;

        Ok(SymbolicaEngine::new(
            evaluator_re,
            evaluator_im,
            parsed_expr_re,
            parsed_expr_im,
            params.expr,
            params.imag_expr,
            params.args.clone(),
            artifacts_dir,
        ))
    }
}

fn compile_real_evaluator(
    expr: &Atom,
    args: &[Atom],
    artifacts_dir: &TempDir,
    stem: &str,
) -> Result<CompiledRealEvaluator, BuildError> {
    let evaluator = expr
        .evaluator(
            &FunctionMap::default(),
            args,
            OptimizationSettings::default(),
        )
        .map_err(|err| BuildError::build(err.to_string()))?
        .map_coeff(&|x| x.to_real().unwrap().to_f64());
    let path = artifacts_dir.path().join(stem);
    let exported_code = evaluator
        .export_cpp::<f64>(path.with_extension("cpp"), stem, ExportSettings::default())
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
            continuous.extend_from_slice(point.continuous.as_slice());
        }

        let mut observable_state = AccumulatorState::from_config(accumulator);

        let mut out_re = vec![0.0; batch.size()];
        self.eval_re
            .evaluate_batch(batch.size(), &continuous, &mut out_re)
            .map_err(|err| EngineError::Eval(err.to_string()))?;

        if let Some(eval_im) = &mut self.eval_im {
            let mut out_im = vec![0.0; batch.size()];
            eval_im
                .evaluate_batch(batch.size(), &continuous, &mut out_im)
                .map_err(|err| EngineError::Eval(err.to_string()))?;
            let mut training_values = options
                .require_training_values
                .then(|| Vec::with_capacity(batch.size()));
            match &mut observable_state {
                AccumulatorState::Vector(vector) => {
                    for (idx, point) in batch.points().iter().enumerate() {
                        let projected = vector
                            .ingest_vector(&[out_re[idx], out_im[idx]], point)
                            .map_err(EvalError::eval)?;
                        if let Some(training_values) = training_values.as_mut() {
                            training_values.push(projected * point.total_weight());
                        }
                    }
                }
                AccumulatorState::FullVector(full_vector) => {
                    for (idx, point) in batch.points().iter().enumerate() {
                        let weight = point.total_weight().abs();
                        full_vector.push_vector(&[out_re[idx] * weight, out_im[idx] * weight]);
                    }
                    training_values = None;
                }
                other => {
                    return Err(EvalError::eval(format!(
                        "complex symbolica evaluator requires vector or full_vector accumulator, got {}",
                        other.kind_str()
                    )));
                }
            }
            return Ok(BatchResult::new(training_values, observable_state));
        }

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

    #[test]
    fn symbolica_manual_eval_has_two_expected_peaks_with_decimal_centers() {
        let mut evaluator = SymbolicaEngine::from_params(SymbolicaParams {
            expr: "1/((x-0.25)^2+(y-0.25)^2+1/40) + 1/((x-0.75)^2+(y-0.75)^2+1/40) + z".to_string(),
            imag_expr: None,
            args: vec!["x".to_string(), "y".to_string(), "z".to_string()],
        })
        .expect("build symbolica evaluator");

        let points = vec![
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
            imag_expr: None,
            args: vec!["x".to_string(), "y".to_string(), "z".to_string()],
        })
        .expect("build symbolica evaluator");

        let points = vec![(0.0, 0.0, 0.0), (0.25, 0.25, 0.0), (0.75, 0.75, 0.0)];
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
            expr: "x".to_string(),
            imag_expr: Some("y".to_string()),
            args: vec!["x".to_string(), "y".to_string()],
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
}
