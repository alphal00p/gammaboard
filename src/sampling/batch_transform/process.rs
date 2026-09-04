use crate::core::EngineResultExt;
use std::process::Stdio;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, BatchTransform, Point};
use crate::process_runtime::{
    build_process_worker_command, default_process_args, parse_process_offsets,
};
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker, pipe_process_stderr};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessBatchTransformParams {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default = "default_process_args")]
    pub args: Value,
}

pub struct ProcessBatchTransform {
    params: ProcessBatchTransformParams,
    worker: Mutex<Option<ProcessBatchTransformWorker>>,
}

impl ProcessBatchTransform {
    pub fn from_params(params: ProcessBatchTransformParams) -> Result<Self, BuildError> {
        validate_params(&params)?;
        Ok(Self {
            params,
            worker: Mutex::new(None),
        })
    }
}

impl BatchTransform for ProcessBatchTransform {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| BuildError::build("process batch transform worker mutex poisoned"))?;
        if worker.is_none() {
            *worker = Some(ProcessBatchTransformWorker::spawn(
                &self.params,
                domain.clone(),
            )?);
        }
        Ok(())
    }

    fn apply(&self, batch: Batch) -> Result<Batch, EngineError> {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| EngineError::engine("process batch transform worker mutex poisoned"))?;
        let worker = worker.as_mut().ok_or_else(|| {
            EngineError::engine("process batch transform used before domain validation")
        })?;
        worker.transform_batch(&batch)
    }
}

fn validate_params(params: &ProcessBatchTransformParams) -> Result<(), BuildError> {
    if params.command.is_empty() {
        return Err(BuildError::build(
            "process_batch_transform command must not be empty",
        ));
    }
    if !params.args.is_object() {
        return Err(BuildError::build(
            "process_batch_transform args must be a TOML table / JSON object",
        ));
    }
    Ok(())
}

struct ProcessBatchTransformWorker {
    process: ProcessWorker,
    domain: Domain,
}

impl ProcessBatchTransformWorker {
    fn spawn(params: &ProcessBatchTransformParams, domain: Domain) -> Result<Self, BuildError> {
        let mut command = build_process_worker_command(
            &params.command,
            params.cwd.as_deref(),
            "batch_transform",
        )?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            BuildError::build(format!(
                "failed to start process batch transform worker: {error}"
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            BuildError::build("process batch transform worker stdin not available")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BuildError::build("process batch transform worker stdout not available")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            BuildError::build("process batch transform worker stderr not available")
        })?;
        let stderr_tail = pipe_process_stderr("process batch transform", stderr);

        let mut worker = Self {
            process: ProcessWorker::new(
                "process batch transform",
                child,
                stdin,
                stdout,
                stderr_tail,
            ),
            domain,
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
                    "role": "batch_transform",
                    "domain": self.domain,
                    "args": args,
                }),
            )
            .map_err(BuildError::build)?;
        expect_ack(response).map_err(BuildError::build)
    }

    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError> {
        let inputs = row_major_batch(batch);
        let response = self
            .process
            .request(
                "transform_batch",
                serde_json::json!({
                    "nr_samples": batch.size(),
                    "xs_discrete_row_major": inputs.xs_discrete_row_major,
                    "xs_discrete_offsets": inputs.xs_discrete_offsets,
                    "xs_continuous_row_major": inputs.xs_continuous_row_major,
                    "xs_continuous_offsets": inputs.xs_continuous_offsets,
                    "weights": inputs.weights,
                }),
            )
            .map_err(EngineError::engine)?;
        decode_batch(&response, batch.size(), &self.domain)
    }
}

struct RowMajorBatch {
    xs_discrete_row_major: Vec<i64>,
    xs_discrete_offsets: Vec<usize>,
    xs_continuous_row_major: Vec<f64>,
    xs_continuous_offsets: Vec<usize>,
    weights: Vec<f64>,
}

fn row_major_batch(batch: &Batch) -> RowMajorBatch {
    let mut xs_discrete_row_major = Vec::new();
    let mut xs_discrete_offsets = Vec::with_capacity(batch.size() + 1);
    let mut xs_continuous_row_major = Vec::new();
    let mut xs_continuous_offsets = Vec::with_capacity(batch.size() + 1);
    let mut weights = Vec::with_capacity(batch.size());
    xs_discrete_offsets.push(0);
    xs_continuous_offsets.push(0);
    for point in batch.points() {
        xs_discrete_row_major.extend_from_slice(&point.discrete);
        xs_discrete_offsets.push(xs_discrete_row_major.len());
        xs_continuous_row_major.extend_from_slice(&point.continuous);
        xs_continuous_offsets.push(xs_continuous_row_major.len());
        weights.push(point.total_weight());
    }
    RowMajorBatch {
        xs_discrete_row_major,
        xs_discrete_offsets,
        xs_continuous_row_major,
        xs_continuous_offsets,
        weights,
    }
}

fn decode_batch(
    response: &Value,
    nr_samples: usize,
    domain: &Domain,
) -> Result<Batch, EngineError> {
    let xs_discrete = response
        .get("xs_discrete_row_major")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(
                "process batch transform response missing 'xs_discrete_row_major' array",
            )
        })?;
    let xs_discrete_offsets = parse_process_offsets(
        response,
        "xs_discrete_offsets",
        nr_samples,
        domain.fixed_discrete_depth().unwrap_or(0),
        xs_discrete.len(),
        "batch transform",
    )?;
    let xs_continuous = response
        .get("xs_continuous_row_major")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(
                "process batch transform response missing 'xs_continuous_row_major' array",
            )
        })?;
    let xs_continuous_offsets = parse_process_offsets(
        response,
        "xs_continuous_offsets",
        nr_samples,
        domain.fixed_continuous_dims().unwrap_or(0),
        xs_continuous.len(),
        "batch transform",
    )?;
    let xs_discrete_row_major = xs_discrete
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_i64().ok_or_else(|| {
                EngineError::engine(format!(
                    "process batch transform returned non-i64 value at xs_discrete_row_major[{index}]"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let xs_continuous_row_major = xs_continuous
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let value = value.as_f64().ok_or_else(|| {
                EngineError::engine(format!(
                    "process batch transform returned non-f64 value at xs_continuous_row_major[{index}]"
                ))
            })?;
            if !value.is_finite() {
                return Err(EngineError::engine(format!(
                    "process batch transform returned non-finite value at xs_continuous_row_major[{index}]"
                )));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let weights = response
        .get("weights")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine("process batch transform response missing 'weights' array")
        })?;
    if weights.len() != nr_samples {
        return Err(EngineError::engine(format!(
            "process batch transform weights size mismatch: expected {} values, got {}",
            nr_samples,
            weights.len()
        )));
    }
    let weights = weights
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let weight = value.as_f64().ok_or_else(|| {
                EngineError::engine(format!(
                    "process batch transform returned non-f64 value at weights[{index}]"
                ))
            })?;
            if !weight.is_finite() || weight <= 0.0 {
                return Err(EngineError::engine(format!(
                    "process batch transform returned non-positive or non-finite value at weights[{index}]"
                )));
            }
            Ok(weight)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut points = Vec::with_capacity(nr_samples);
    for sample_idx in 0..nr_samples {
        let discrete_start = xs_discrete_offsets[sample_idx];
        let discrete_end = xs_discrete_offsets[sample_idx + 1];
        let continuous_start = xs_continuous_offsets[sample_idx];
        let continuous_end = xs_continuous_offsets[sample_idx + 1];
        points.push(Point::new(
            xs_continuous_row_major[continuous_start..continuous_end].to_vec(),
            xs_discrete_row_major[discrete_start..discrete_end].to_vec(),
            weights[sample_idx],
        ));
    }
    Batch::new(points).engine_err()
}

fn expect_ack(response: Value) -> Result<(), String> {
    if response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(());
    }
    Err("process batch transform initialize result missing ok=true".to_string())
}

#[cfg(test)]
mod tests {
    use super::{ProcessBatchTransform, ProcessBatchTransformParams};
    use crate::evaluation::{Batch, BatchTransform, Point};
    use crate::utils::domain::Domain;
    use serde_json::json;

    const SCALE_TRANSFORM_WORKER: &str = r#"
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
    elif method == "transform_batch":
        send_result(req.get("id"), {
            "xs_discrete_row_major": params["xs_discrete_row_major"],
            "xs_discrete_offsets": params["xs_discrete_offsets"],
            "xs_continuous_row_major": [value * 2.0 for value in params["xs_continuous_row_major"]],
            "xs_continuous_offsets": params["xs_continuous_offsets"],
            "weights": [value * 3.0 for value in params["weights"]],
        })
    else:
        raise ValueError(f"unknown method: {method}")
"#;

    #[test]
    fn process_batch_transform_deserializes_command_and_args() {
        let params = toml::from_str::<ProcessBatchTransformParams>(
            r#"
command = ["python", "-u", "worker.py"]
cwd = "$resources"
args = { scale = 2.0 }
"#,
        )
        .expect("process batch transform config should parse");

        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.cwd.as_deref(), Some("$resources"));
        assert_eq!(params.args, json!({"scale": 2.0}));
    }

    #[test]
    fn process_batch_transform_transforms_batch() {
        let python =
            std::env::var("GAMMABOARD_TEST_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let params = ProcessBatchTransformParams {
            command: vec![
                python,
                "-u".to_string(),
                "-c".to_string(),
                SCALE_TRANSFORM_WORKER.to_string(),
            ],
            cwd: None,
            args: json!({}),
        };
        let transform = ProcessBatchTransform::from_params(params).expect("transform");
        transform
            .validate_domain(&Domain::continuous(2))
            .expect("validate domain");
        let batch = Batch::from_points([
            Point::new(vec![0.25, 0.5], Vec::new(), 1.0),
            Point::new(vec![0.75, 1.0], Vec::new(), 2.0),
        ])
        .expect("batch");

        let transformed = transform.apply(batch).expect("transform batch");

        assert_eq!(transformed.points()[0].continuous, vec![0.5, 1.0]);
        assert_eq!(transformed.points()[1].continuous, vec![1.5, 2.0]);
        assert_eq!(transformed.weights(), vec![3.0, 6.0]);
    }
}
