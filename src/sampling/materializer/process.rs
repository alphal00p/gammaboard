use crate::core::EngineResultExt;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Materializer, Point};
use crate::process_runtime::{
    build_process_worker_command, default_process_args, parse_process_offsets,
};
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker, pipe_process_stderr};
use crate::sampling::LatentBatch;
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessMaterializerParams {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default = "default_process_args")]
    pub args: Value,
}

pub struct ProcessMaterializer {
    domain: Domain,
    worker: ProcessMaterializerWorker,
}

impl ProcessMaterializer {
    pub fn from_params_and_domain(
        params: ProcessMaterializerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_params(&params)?;
        let worker = ProcessMaterializerWorker::spawn(&params, domain.clone())?;
        Ok(Self {
            domain: domain.clone(),
            worker,
        })
    }
}

impl Materializer for ProcessMaterializer {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        if domain != &self.domain {
            return Err(BuildError::build(format!(
                "process_materializer domain mismatch: expected {:?}, got {:?}",
                self.domain, domain
            )));
        }
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        self.worker.materialize_batch(latent_batch)
    }
}

fn validate_params(params: &ProcessMaterializerParams) -> Result<(), BuildError> {
    if params.command.is_empty() {
        return Err(BuildError::build(
            "process_materializer command must not be empty",
        ));
    }
    if !params.args.is_object() {
        return Err(BuildError::build(
            "process_materializer args must be a TOML table / JSON object",
        ));
    }
    Ok(())
}

struct ProcessMaterializerWorker {
    process: ProcessWorker,
    domain: Domain,
}

impl ProcessMaterializerWorker {
    fn spawn(params: &ProcessMaterializerParams, domain: Domain) -> Result<Self, BuildError> {
        let mut command =
            build_process_worker_command(&params.command, params.cwd.as_deref(), "materializer")?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            BuildError::build(format!(
                "failed to start process materializer worker: {error}"
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BuildError::build("process materializer worker stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BuildError::build("process materializer worker stdout not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BuildError::build("process materializer worker stderr not available"))?;
        let stderr_tail = pipe_process_stderr("process materializer", stderr);

        let mut worker = Self {
            process: ProcessWorker::new("process materializer", child, stdin, stdout, stderr_tail),
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
                    "role": "materializer",
                    "domain": self.domain,
                    "args": args,
                }),
            )
            .map_err(BuildError::build)?;
        expect_ack(response).map_err(BuildError::build)
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        let response = self
            .process
            .request(
                "materialize_batch",
                serde_json::json!({
                    "nr_samples": latent_batch.nr_samples,
                    "latent_batch": latent_batch,
                }),
            )
            .map_err(EngineError::engine)?;
        decode_materialized_batch(&response, latent_batch.nr_samples, &self.domain)
    }
}

fn decode_materialized_batch(
    response: &Value,
    nr_samples: usize,
    domain: &Domain,
) -> Result<Batch, EngineError> {
    let xs_discrete = response
        .get("xs_discrete_row_major")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(
                "process materializer response missing 'xs_discrete_row_major' array",
            )
        })?;
    let xs_discrete_offsets = parse_process_offsets(
        response,
        "xs_discrete_offsets",
        nr_samples,
        domain.fixed_discrete_depth().unwrap_or(0),
        xs_discrete.len(),
        "materializer",
    )?;
    let xs_continuous = response
        .get("xs_continuous_row_major")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine(
                "process materializer response missing 'xs_continuous_row_major' array",
            )
        })?;
    let xs_continuous_offsets = parse_process_offsets(
        response,
        "xs_continuous_offsets",
        nr_samples,
        domain.fixed_continuous_dims().unwrap_or(0),
        xs_continuous.len(),
        "materializer",
    )?;
    let xs_discrete_row_major = xs_discrete
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_i64().ok_or_else(|| {
                EngineError::engine(format!(
                    "process materializer returned non-i64 value at xs_discrete_row_major[{index}]"
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
                    "process materializer returned non-f64 value at xs_continuous_row_major[{index}]"
                ))
            })?;
            if !value.is_finite() {
                return Err(EngineError::engine(format!(
                    "process materializer returned non-finite value at xs_continuous_row_major[{index}]"
                )));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let weights = response
        .get("weights")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            EngineError::engine("process materializer response missing 'weights' array")
        })?;
    if weights.len() != nr_samples {
        return Err(EngineError::engine(format!(
            "process materializer weights size mismatch: expected {} values, got {}",
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
                    "process materializer returned non-f64 value at weights[{index}]"
                ))
            })?;
            if !weight.is_finite() || weight <= 0.0 {
                return Err(EngineError::engine(format!(
                    "process materializer returned non-positive or non-finite value at weights[{index}]"
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
    Err("process materializer initialize result missing ok=true".to_string())
}

#[cfg(test)]
mod tests {
    use super::{ProcessMaterializer, ProcessMaterializerParams};
    use crate::evaluation::{Batch, Materializer, Point};
    use crate::sampling::LatentBatchSpec;
    use crate::utils::domain::Domain;
    use serde_json::json;

    const ECHO_MATERIALIZER_WORKER: &str = r#"
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
    elif method == "materialize_batch":
        batch = params["latent_batch"]["payload"]
        offsets = [0]
        for width in batch["continuous_layouts"]:
            offsets.append(offsets[-1] + width)
        send_result(req.get("id"), {
            "xs_discrete_row_major": [],
            "xs_discrete_offsets": [0, 0, 0],
            "xs_continuous_row_major": [value * 2.0 for value in batch["continuous_values"]],
            "xs_continuous_offsets": offsets,
            "weights": batch["weights"],
        })
    else:
        raise ValueError(f"unknown method: {method}")
"#;

    #[test]
    fn process_materializer_deserializes_command_and_args() {
        let params = toml::from_str::<ProcessMaterializerParams>(
            r#"
command = ["python", "-u", "worker.py"]
cwd = "$resources"
args = { scale = 2.0 }
"#,
        )
        .expect("process materializer config should parse");

        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.cwd.as_deref(), Some("$resources"));
        assert_eq!(params.args, json!({"scale": 2.0}));
    }

    #[test]
    fn process_materializer_materializes_latent_batch() {
        let python =
            std::env::var("GAMMABOARD_TEST_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let params = ProcessMaterializerParams {
            command: vec![
                python,
                "-u".to_string(),
                "-c".to_string(),
                ECHO_MATERIALIZER_WORKER.to_string(),
            ],
            cwd: None,
            args: json!({}),
        };
        let mut materializer =
            ProcessMaterializer::from_params_and_domain(params, &Domain::continuous(2))
                .expect("process materializer should initialize");
        let batch = Batch::from_points([
            Point::new(vec![0.25, 0.5], Vec::new(), 1.0),
            Point::new(vec![0.75, 1.0], Vec::new(), 2.0),
        ])
        .expect("batch");
        let latent = LatentBatchSpec::from_batch(&batch).build();

        let materialized = materializer
            .materialize_batch(&latent)
            .expect("materialize batch");

        assert_eq!(materialized.points()[0].continuous, vec![0.5, 1.0]);
        assert_eq!(materialized.points()[1].continuous, vec![1.5, 2.0]);
        assert_eq!(materialized.weights(), vec![1.0, 2.0]);
    }
}
