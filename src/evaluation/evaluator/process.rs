use std::io::BufRead;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator, IngestScalar,
    ScalarBatchEvaluator,
};
use crate::process_runtime::build_process_worker_command;
use crate::process_worker::ProcessWorker;
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessEvaluatorParams {
    pub command: Vec<String>,
    pub continuous_dims: usize,
    #[serde(default)]
    pub discrete_cardinalities: Vec<usize>,
    #[serde(default = "default_components")]
    pub components: Vec<String>,
    #[serde(default = "default_args")]
    pub args: Value,
}

pub struct ProcessEvaluator {
    continuous_dims: usize,
    discrete_cardinalities: Vec<usize>,
    components: Vec<String>,
    worker: ProcessRuntimeWorker,
}

impl ProcessEvaluator {
    pub fn from_params(params: ProcessEvaluatorParams) -> Result<Self, BuildError> {
        if params.command.is_empty() {
            return Err(BuildError::build(
                "process_evaluator command must not be empty",
            ));
        }
        if !params.args.is_object() {
            return Err(BuildError::build(
                "process_evaluator args must be a TOML table / JSON object",
            ));
        }
        if params.components.is_empty() {
            return Err(BuildError::build(
                "process_evaluator components must not be empty",
            ));
        }
        if params.components.len() != 1 {
            return Err(BuildError::build(
                "process_evaluator currently supports exactly one component until vector accumulators are enabled",
            ));
        }
        if params
            .discrete_cardinalities
            .iter()
            .any(|value| *value == 0)
        {
            return Err(BuildError::build(
                "process_evaluator discrete_cardinalities must contain only positive integers",
            ));
        }
        let worker = ProcessRuntimeWorker::spawn(
            &params.command,
            &params.discrete_cardinalities,
            params.continuous_dims,
            &params.components,
            params.args,
        )?;
        Ok(Self {
            continuous_dims: params.continuous_dims,
            discrete_cardinalities: params.discrete_cardinalities,
            components: params.components,
            worker,
        })
    }

    fn scalar_ingestor<'a>(
        state: &'a mut AccumulatorState,
    ) -> Result<&'a mut dyn IngestScalar, EvalError> {
        match state {
            AccumulatorState::Empty(accumulator) => Ok(accumulator),
            AccumulatorState::Scalar(accumulator) => Ok(accumulator),
            AccumulatorState::FullScalar(accumulator) => Ok(accumulator),
            other => Err(EvalError::eval(format!(
                "process_evaluator does not support accumulator kind {}",
                other.kind_str()
            ))),
        }
    }
}

impl ScalarBatchEvaluator for ProcessEvaluator {
    fn discrete_dims(&self) -> usize {
        self.discrete_cardinalities.len()
    }

    fn continuous_dims(&self) -> usize {
        self.continuous_dims
    }

    fn eval_scalar_rectangular_batch(
        &mut self,
        xs_discrete_row_major: &[i64],
        xs_continuous_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        let values =
            self.worker
                .eval_batch(xs_discrete_row_major, xs_continuous_row_major, nr_samples)?;
        if self.components.len() != 1 {
            return Err(EvalError::eval(
                "process_evaluator scalar accumulator path requires exactly one component",
            ));
        }
        Ok(values)
    }
}

impl Evaluator for ProcessEvaluator {
    fn get_domain(&self) -> Domain {
        Domain::rectangular_with_cardinalities(
            self.continuous_dims,
            self.discrete_cardinalities.clone(),
        )
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let weighted_values = self.eval_scalar_batch_into(
            batch,
            Self::scalar_ingestor(&mut observable_state)?,
            options.require_training_values,
        )?;
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

struct ProcessRuntimeWorker {
    process: ProcessWorker,
    discrete_cardinalities: Vec<usize>,
    continuous_dims: usize,
    components: Vec<String>,
}

impl ProcessRuntimeWorker {
    fn spawn(
        command: &[String],
        discrete_cardinalities: &[usize],
        continuous_dims: usize,
        components: &[String],
        args: Value,
    ) -> Result<Self, BuildError> {
        let mut command = build_process_worker_command(command, "evaluator")?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            BuildError::build(format!("failed to start process evaluator worker: {error}"))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BuildError::build("process worker stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BuildError::build("process worker stdout not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BuildError::build("process worker stderr not available"))?;
        pipe_child_stderr("process evaluator", stderr);

        let mut worker = Self {
            process: ProcessWorker::new("process evaluator", child, stdin, stdout),
            discrete_cardinalities: discrete_cardinalities.to_vec(),
            continuous_dims,
            components: components.to_vec(),
        };
        worker.send_init(discrete_cardinalities, continuous_dims, components, args)?;
        Ok(worker)
    }

    fn send_init(
        &mut self,
        discrete_cardinalities: &[usize],
        continuous_dims: usize,
        components: &[String],
        args: Value,
    ) -> Result<(), BuildError> {
        let response = self
            .process
            .request(
                "initialize",
                serde_json::json!({
                    "protocol": "gammaboard-jsonrpc-v1",
                    "role": "evaluator",
                    "components": components,
                    "observable": {
                        "components": components,
                    },
                    "domain": {
                        "continuous_dims": continuous_dims,
                        "discrete_cardinalities": discrete_cardinalities,
                    },
                    "discrete_cardinalities": discrete_cardinalities,
                    "continuous_dims": continuous_dims,
                    "args": args,
                }),
            )
            .map_err(BuildError::build)?;
        Self::expect_ack(response).map_err(BuildError::build)
    }

    fn eval_batch(
        &mut self,
        xs_discrete_row_major: &[i64],
        xs_continuous_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        let response = self
            .process
            .request(
                "eval_batch",
                serde_json::json!({
                    "nr_samples": nr_samples,
                    "discrete_cardinalities": self.discrete_cardinalities,
                    "continuous_dims": self.continuous_dims,
                    "components": self.components,
                    "xs_discrete_row_major": xs_discrete_row_major,
                    "xs_continuous_row_major": xs_continuous_row_major,
                }),
            )
            .map_err(EvalError::eval)?;
        let values = response
            .get("values_row_major")
            .and_then(Value::as_array)
            .ok_or_else(|| EvalError::eval("process worker response missing 'values_row_major'"))?;
        let expected_len = nr_samples.saturating_mul(self.components.len());
        if values.len() != expected_len {
            return Err(EvalError::eval(format!(
                "process evaluator output size mismatch: expected {}, got {}",
                expected_len,
                values.len()
            )));
        }
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_f64().ok_or_else(|| {
                    EvalError::eval(format!(
                        "process worker returned non-f64 at output index {index}"
                    ))
                })
            })
            .collect()
    }

    fn expect_ack(response: Value) -> Result<(), String> {
        if response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(());
        }
        Err("process worker initialize result missing ok=true".to_string())
    }
}

fn default_args() -> Value {
    Value::Object(serde_json::Map::new())
}

fn default_components() -> Vec<String> {
    vec!["value".to_string()]
}

fn pipe_child_stderr(label: &'static str, stderr: impl std::io::Read + Send + 'static) {
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            tracing::warn!(
                source = "worker",
                message = format!("[{label} stderr] {line}")
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::ProcessEvaluatorParams;

    #[test]
    fn process_evaluator_deserializes_command_and_args() {
        let params = toml::from_str::<ProcessEvaluatorParams>(
            r#"
command = ["python", "-u", "worker.py"]
continuous_dims = 2
discrete_cardinalities = [2, 3]
args = { module = "my_module", class = "MyEvaluator", scale = 1.0 }
"#,
        )
        .expect("process scalar config should parse");

        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.continuous_dims, 2);
        assert_eq!(params.discrete_cardinalities, [2, 3]);
        assert_eq!(params.components, ["value"]);
        assert!(params.args.is_object());
    }
}
