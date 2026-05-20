use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator};
use crate::process_runtime::build_process_worker_command;
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker, pipe_process_stderr};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessEvaluatorParams {
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    pub domain: Domain,
    #[serde(default = "default_components")]
    pub components: Vec<String>,
    #[serde(default = "default_args")]
    pub args: Value,
}

pub struct ProcessEvaluator {
    domain: Domain,
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
        let domain = params.domain.clone();
        let worker = ProcessRuntimeWorker::spawn(&params, domain.clone())?;
        Ok(Self {
            domain,
            components: params.components,
            worker,
        })
    }
}

impl Evaluator for ProcessEvaluator {
    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let inputs = ragged_row_major_inputs(batch);
        let values = self.worker.eval_batch(&inputs, batch.size())?;
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

#[derive(Debug, Clone)]
struct RaggedRowMajorInputs {
    xs_discrete_row_major: Vec<i64>,
    xs_discrete_offsets: Vec<usize>,
    xs_continuous_row_major: Vec<f64>,
    xs_continuous_offsets: Vec<usize>,
}

fn ragged_row_major_inputs(batch: &Batch) -> RaggedRowMajorInputs {
    let mut xs_discrete = Vec::new();
    let mut xs_discrete_offsets = Vec::with_capacity(batch.size() + 1);
    let mut xs_continuous = Vec::new();
    let mut xs_continuous_offsets = Vec::with_capacity(batch.size() + 1);
    xs_discrete_offsets.push(0);
    xs_continuous_offsets.push(0);
    for point in batch.points() {
        xs_discrete.extend_from_slice(&point.discrete);
        xs_discrete_offsets.push(xs_discrete.len());
        xs_continuous.extend_from_slice(&point.continuous);
        xs_continuous_offsets.push(xs_continuous.len());
    }
    RaggedRowMajorInputs {
        xs_discrete_row_major: xs_discrete,
        xs_discrete_offsets,
        xs_continuous_row_major: xs_continuous,
        xs_continuous_offsets,
    }
}

struct ProcessRuntimeWorker {
    process: ProcessWorker,
    domain: Domain,
    components: Vec<String>,
}

impl ProcessRuntimeWorker {
    fn spawn(params: &ProcessEvaluatorParams, domain: Domain) -> Result<Self, BuildError> {
        let mut command =
            build_process_worker_command(&params.command, params.cwd.as_deref(), "evaluator")?;
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
        let stderr_tail = pipe_process_stderr("process evaluator", stderr);

        let mut worker = Self {
            process: ProcessWorker::new("process evaluator", child, stdin, stdout, stderr_tail),
            domain,
            components: params.components.clone(),
        };
        worker.send_init(params.args.clone())?;
        Ok(worker)
    }

    fn send_init(&mut self, args: Value) -> Result<(), BuildError> {
        let response = self
            .process
            .request(
                "initialize",
                serde_json::json!({
                    "protocol": PROCESS_PROTOCOL,
                    "role": "evaluator",
                    "components": self.components,
                    "observable": {
                        "components": self.components,
                    },
                    "domain": self.domain,
                    "args": args,
                }),
            )
            .map_err(BuildError::build)?;
        Self::expect_ack(response).map_err(BuildError::build)
    }

    fn eval_batch(
        &mut self,
        inputs: &RaggedRowMajorInputs,
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        let response = self
            .process
            .request(
                "eval_batch",
                serde_json::json!({
                    "nr_samples": nr_samples,
                    "components": self.components,
                    "xs_discrete_row_major": inputs.xs_discrete_row_major,
                    "xs_discrete_offsets": inputs.xs_discrete_offsets,
                    "xs_continuous_row_major": inputs.xs_continuous_row_major,
                    "xs_continuous_offsets": inputs.xs_continuous_offsets,
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

#[cfg(test)]
mod tests {
    use super::{ProcessEvaluatorParams, ProcessRuntimeWorker, RaggedRowMajorInputs};
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
cwd = "$resources"
domain = { rectangular = { discrete_cardinalities = [2, 3], continuous_dims = 2 } }
args = { scale = 1.0 }
"#,
        )
        .expect("process scalar config should parse");

        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.cwd.as_deref(), Some("$resources"));
        assert_eq!(params.domain.fixed_rectangular_dims(), Some((2, 2)));
        assert_eq!(
            params.domain.fixed_discrete_cardinalities(),
            Some(vec![2, 3])
        );
        assert_eq!(params.components, ["value"]);
        assert!(params.args.is_object());
    }

    #[test]
    fn process_evaluator_deserializes_inhomogeneous_domain() {
        let params = toml::from_str::<ProcessEvaluatorParams>(
            r#"
command = ["worker"]
domain = { discrete = { axis_label = "d0", branches = [
  { index = 0, domain = { continuous = { dims = 3 } } },
  { index = 1, domain = { discrete = { axis_label = "d1", branches = [
    { index = 0, domain = { continuous = { dims = 1 } } },
    { index = 1, domain = { rectangular = { discrete_cardinalities = [5], continuous_dims = 5 } } },
  ] } } },
] } }
"#,
        )
        .expect("process evaluator config should parse inhomogeneous domain");

        assert_eq!(params.domain.fixed_rectangular_dims(), None);
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
        let params = ProcessEvaluatorParams {
            command,
            cwd: None,
            domain: crate::utils::domain::Domain::continuous(2),
            components: vec!["value".to_string()],
            args: json!({}),
        };
        let components = params.components.clone();
        let continuous_dims = 2;
        let domain = params.domain.clone();
        let mut worker = ProcessRuntimeWorker::spawn(&params, domain)?;

        let cases = [(1_usize, 2_000_usize), (1_024, 500), (65_536, 20)];
        for (nr_samples, repetitions) in cases {
            let xs_discrete_row_major = Vec::new();
            let xs_continuous_row_major = vec![0.5; nr_samples * continuous_dims];
            let inputs = RaggedRowMajorInputs {
                xs_discrete_row_major,
                xs_discrete_offsets: vec![0; nr_samples + 1],
                xs_continuous_row_major,
                xs_continuous_offsets: (0..=nr_samples)
                    .map(|index| index * continuous_dims)
                    .collect(),
            };
            for _ in 0..5 {
                let values = worker.eval_batch(&inputs, nr_samples)?;
                assert_eq!(values.len(), nr_samples * components.len());
            }

            let started = Instant::now();
            for _ in 0..repetitions {
                let values = worker.eval_batch(&inputs, nr_samples)?;
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
