use std::io::BufRead;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator};
use crate::process_runtime::build_process_worker_command;
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker};
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
        let (xs_discrete, xs_continuous) = dense_rectangular_inputs(
            batch,
            self.discrete_cardinalities.len(),
            self.continuous_dims,
        )?;
        let values = self
            .worker
            .eval_batch(&xs_discrete, &xs_continuous, batch.size())?;
        let mut training_values = options
            .require_training_values
            .then(|| Vec::with_capacity(batch.size()));
        match &mut observable_state {
            AccumulatorState::Vector(vector_state) => {
                for (sample_idx, point) in batch.points().iter().enumerate() {
                    let start = sample_idx * self.components.len();
                    let end = start + self.components.len();
                    let projected = vector_state
                        .ingest_vector(&values[start..end], point)
                        .map_err(EvalError::eval)?;
                    if let Some(training_values) = training_values.as_mut() {
                        training_values.push(projected * point.total_weight());
                    }
                }
            }
            AccumulatorState::FullVector(full_state) => {
                for (sample_idx, point) in batch.points().iter().enumerate() {
                    let start = sample_idx * self.components.len();
                    let end = start + self.components.len();
                    let weight = point.total_weight().abs();
                    let weighted = values[start..end]
                        .iter()
                        .map(|value| value * weight)
                        .collect::<Vec<_>>();
                    full_state.push_vector(&weighted);
                }
                training_values = None;
            }
            other => {
                return Err(EvalError::eval(format!(
                    "process_evaluator requires vector or full_vector accumulator config, got {}",
                    other.kind_str()
                )));
            }
        }
        Ok(BatchResult::new(training_values, observable_state))
    }
}

fn dense_rectangular_inputs(
    batch: &Batch,
    discrete_dims: usize,
    continuous_dims: usize,
) -> Result<(Vec<i64>, Vec<f64>), EvalError> {
    let mut xs_discrete = Vec::with_capacity(batch.size().saturating_mul(discrete_dims));
    let mut xs_continuous = Vec::with_capacity(batch.size().saturating_mul(continuous_dims));
    for (sample_idx, point) in batch.points().iter().enumerate() {
        if point.discrete.len() != discrete_dims {
            return Err(EvalError::eval(format!(
                "process_evaluator expected {discrete_dims} discrete dimensions, sample {sample_idx} has {}",
                point.discrete.len(),
            )));
        }
        if point.continuous.len() != continuous_dims {
            return Err(EvalError::eval(format!(
                "process_evaluator expected {continuous_dims} continuous dimensions, sample {sample_idx} has {}",
                point.continuous.len(),
            )));
        }
        xs_discrete.extend_from_slice(&point.discrete);
        xs_continuous.extend_from_slice(&point.continuous);
    }
    Ok((xs_discrete, xs_continuous))
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
                    "protocol": PROCESS_PROTOCOL,
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
    use super::{ProcessEvaluatorParams, ProcessRuntimeWorker};
    use serde_json::json;
    use std::error::Error;
    use std::time::Instant;

    const ECHO_EVALUATOR_WORKER: &str = r#"
import json, sys

def read_frame():
    content_length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode("ascii", errors="replace").strip()
        if not line:
            if content_length is not None:
                break
            continue
        name, sep, value = line.partition(":")
        if sep and name.lower() == "content-length":
            content_length = int(value.strip())
    return json.loads(sys.stdin.buffer.read(content_length))

def send_result(req_id, result):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": req_id, "result": result},
        separators=(",", ":"),
    ).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

while True:
    req = read_frame()
    if req is None:
        break
    method = req.get("method")
    params = req.get("params") or {}
    if method == "initialize":
        send_result(req.get("id"), {"ok": True})
    elif method == "eval_batch":
        nr_samples = int(params["nr_samples"])
        components = params.get("components") or ["value"]
        send_result(req.get("id"), {"values_row_major": [0.0] * (nr_samples * len(components))})
    else:
        send_result(req.get("id"), {"ok": True})
"#;

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

    #[test]
    #[ignore = "manual protocol overhead benchmark; run with --ignored --nocapture"]
    fn process_evaluator_eval_batch_protocol_benchmark() -> Result<(), Box<dyn Error>> {
        let python = std::env::var("GAMMABOARD_PROTOCOL_BENCH_PYTHON")
            .unwrap_or_else(|_| "python3".to_string());
        let command = vec![
            python,
            "-u".to_string(),
            "-c".to_string(),
            ECHO_EVALUATOR_WORKER.to_string(),
        ];
        let components = vec!["value".to_string()];
        let discrete_cardinalities = Vec::new();
        let continuous_dims = 2;
        let mut worker = ProcessRuntimeWorker::spawn(
            &command,
            &discrete_cardinalities,
            continuous_dims,
            &components,
            json!({}),
        )?;

        let cases = [(1_usize, 2_000_usize), (1_024, 500), (65_536, 20)];
        for (nr_samples, repetitions) in cases {
            let xs_discrete_row_major = Vec::new();
            let xs_continuous_row_major = vec![0.5; nr_samples * continuous_dims];
            for _ in 0..5 {
                let values = worker.eval_batch(
                    &xs_discrete_row_major,
                    &xs_continuous_row_major,
                    nr_samples,
                )?;
                assert_eq!(values.len(), nr_samples * components.len());
            }

            let started = Instant::now();
            for _ in 0..repetitions {
                let values = worker.eval_batch(
                    &xs_discrete_row_major,
                    &xs_continuous_row_major,
                    nr_samples,
                )?;
                assert_eq!(values.len(), nr_samples * components.len());
            }
            let elapsed = started.elapsed();
            let total_samples = repetitions * nr_samples;
            let total_ms = elapsed.as_secs_f64() * 1_000.0;
            let mean_ms = total_ms / repetitions as f64;
            let us_per_sample = elapsed.as_secs_f64() * 1_000_000.0 / total_samples as f64;
            eprintln!(
                "process eval_batch protocol benchmark: samples_per_batch={nr_samples} reps={repetitions} total_ms={total_ms:.3} mean_batch_ms={mean_ms:.6} us_per_sample={us_per_sample:.6}",
            );
        }

        Ok(())
    }
}
