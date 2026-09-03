use crate::core::EngineResultExt;
use std::process::Stdio;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Point};
use crate::process_runtime::build_process_worker_command;
use crate::process_worker::{
    PROCESS_PROTOCOL, ProcessWorker, extend_le_f64, pipe_process_stderr, read_le_f64, read_le_i64,
};
use crate::sampling::{
    DiscreteSubspace, LatentBatchSpec, PdfPoint, SamplePlan, SamplerAggregator,
    SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessSamplerParams {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub requires_training_values: bool,
    #[serde(default = "default_args")]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProcessSamplerSnapshot {
    params: ProcessSamplerParams,
    sampler_state: Value,
}

pub struct ProcessSampler {
    params: ProcessSamplerParams,
    domain: Domain,
    worker: Mutex<ProcessSamplerWorker>,
}

impl ProcessSampler {
    pub(crate) fn from_params_and_domain(
        params: ProcessSamplerParams,
        domain: &Domain,
        evaluator_metadata: Value,
    ) -> Result<Self, BuildError> {
        validate_params(&params)?;
        let worker =
            ProcessSamplerWorker::spawn(&params, domain.clone(), None, evaluator_metadata)?;
        Ok(Self {
            params,
            domain: domain.clone(),
            worker: Mutex::new(worker),
        })
    }

    pub(crate) fn from_snapshot(
        snapshot: ProcessSamplerSnapshot,
        domain: &Domain,
        evaluator_metadata: Value,
    ) -> Result<Self, BuildError> {
        validate_params(&snapshot.params)?;
        let worker = ProcessSamplerWorker::spawn(
            &snapshot.params,
            domain.clone(),
            Some(snapshot.sampler_state),
            evaluator_metadata,
        )?;
        Ok(Self {
            params: snapshot.params,
            domain: domain.clone(),
            worker: Mutex::new(worker),
        })
    }
}

fn validate_params(params: &ProcessSamplerParams) -> Result<(), BuildError> {
    if params.command.is_empty() {
        return Err(BuildError::build(
            "process_sampler command must not be empty",
        ));
    }
    if !params.args.is_object() {
        return Err(BuildError::build(
            "process_sampler args must be a TOML table / JSON object",
        ));
    }
    Ok(())
}

impl SamplerAggregator for ProcessSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        if domain != &self.domain {
            return Err(BuildError::build(format!(
                "process_sampler domain mismatch: expected {:?}, got {:?}",
                self.domain, domain
            )));
        }
        Ok(())
    }

    fn training_samples_remaining(&self) -> Option<usize> {
        let mut worker = match self.worker.lock() {
            Ok(guard) => guard,
            Err(_) => return None,
        };
        worker.training_samples_remaining().ok().flatten()
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        self.worker_mut()?.sample_plan()
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "process_sampler requires nr_samples > 0",
            ));
        }
        let sample_batch = self.worker_mut()?.produce_latent_batch(nr_samples)?;
        let mut points = Vec::with_capacity(nr_samples);
        for sample_idx in 0..nr_samples {
            let discrete_start = sample_batch.xs_discrete_offsets[sample_idx];
            let discrete_end = sample_batch.xs_discrete_offsets[sample_idx + 1];
            let continuous_start = sample_batch.xs_continuous_offsets[sample_idx];
            let continuous_end = sample_batch.xs_continuous_offsets[sample_idx + 1];
            points.push(Point::new(
                sample_batch.xs_continuous_row_major[continuous_start..continuous_end].to_vec(),
                sample_batch.xs_discrete_row_major[discrete_start..discrete_end].to_vec(),
                sample_batch.weights[sample_idx],
            ));
        }
        let batch = Batch::new(points).engine_err()?;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        self.worker_mut()?.ingest_training_values(training_values)
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker_mut()?.pdf_batch(points)
    }

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker_mut()?.discrete_pdf_batch(subspaces)
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let sampler_state = self.worker_mut()?.snapshot()?;
        let raw = serde_json::to_value(ProcessSamplerSnapshot {
            params: self.params.clone(),
            sampler_state,
        })
        .map_err(EngineError::from)?;
        Ok(SamplerAggregatorSnapshot::ProcessSampler { raw })
    }

    fn get_diagnostics(&mut self) -> Value {
        self.worker_mut()
            .and_then(|mut worker| worker.get_diagnostics())
            .unwrap_or_else(|_| serde_json::json!({}))
    }
}

impl ProcessSampler {
    fn worker_mut(&self) -> Result<std::sync::MutexGuard<'_, ProcessSamplerWorker>, EngineError> {
        self.worker
            .lock()
            .map_err(|_| EngineError::engine("process sampler worker mutex poisoned"))
    }
}

struct ProcessSampleBatch {
    xs_discrete_row_major: Vec<i64>,
    xs_discrete_offsets: Vec<usize>,
    xs_continuous_row_major: Vec<f64>,
    xs_continuous_offsets: Vec<usize>,
    weights: Vec<f64>,
}

struct ProcessSamplerWorker {
    process: ProcessWorker,
    domain: Domain,
}

impl ProcessSamplerWorker {
    fn spawn(
        params: &ProcessSamplerParams,
        domain: Domain,
        snapshot: Option<Value>,
        evaluator_metadata: Value,
    ) -> Result<Self, BuildError> {
        let mut command =
            build_process_worker_command(&params.command, params.cwd.as_deref(), "sampler")?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            BuildError::build(format!("failed to start process sampler worker: {error}"))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BuildError::build("process sampler worker stdin not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BuildError::build("process sampler worker stdout not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BuildError::build("process sampler worker stderr not available"))?;
        let stderr_tail = pipe_process_stderr("process sampler", stderr);

        let mut worker = Self {
            process: ProcessWorker::new("process sampler", child, stdin, stdout, stderr_tail),
            domain,
        };
        worker.send_init(params.args.clone(), snapshot, evaluator_metadata)?;
        Ok(worker)
    }

    fn send_init(
        &mut self,
        args: Value,
        snapshot: Option<Value>,
        evaluator_metadata: Value,
    ) -> Result<(), BuildError> {
        let response = self
            .process
            .request(
                "initialize",
                serde_json::json!({
                    "protocol": PROCESS_PROTOCOL,
                    "role": "sampler",
                    "domain": self.domain,
                    "args": args,
                    "snapshot": snapshot,
                    "evaluator_metadata": evaluator_metadata,
                }),
            )
            .map_err(BuildError::build)?;
        Self::expect_ack(response).map_err(BuildError::build)
    }

    fn training_samples_remaining(&mut self) -> Result<Option<usize>, EngineError> {
        let response = self
            .process
            .request("training_samples_remaining", serde_json::json!({}))
            .map_err(EngineError::engine)?;
        let remaining = match response.get("remaining") {
            Some(Value::Null) | None => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                EngineError::engine(
                    "process sampler response field 'remaining' must be u64 or null",
                )
            })? as usize),
        };
        Ok(remaining)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let response = self
            .process
            .request("sample_plan", serde_json::json!({}))
            .map_err(EngineError::engine)?;
        let plan_value = response
            .get("plan")
            .cloned()
            .ok_or_else(|| EngineError::engine("process sampler response missing 'plan'"))?;
        serde_json::from_value(plan_value).map_err(|err| {
            EngineError::engine(format!("invalid process sample_plan payload: {err}"))
        })
    }

    fn produce_latent_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<ProcessSampleBatch, EngineError> {
        let (response, binary) = self
            .process
            .request_with_binary(
                "produce_latent_batch",
                serde_json::json!({
                    "nr_samples": nr_samples,
                }),
                &[],
            )
            .map_err(EngineError::engine)?;
        // Binary block layout: i64 discrete, f64 continuous, f64 weights. Offsets
        // (and thus the array lengths) come from the JSON envelope.
        let discrete_dims = self.domain.fixed_discrete_depth().unwrap_or(0);
        let continuous_dims = self.domain.fixed_continuous_dims().unwrap_or(0);
        let discrete_len =
            offsets_total_len(&response, "xs_discrete_offsets", nr_samples, discrete_dims)?;
        let continuous_len = offsets_total_len(
            &response,
            "xs_continuous_offsets",
            nr_samples,
            continuous_dims,
        )?;
        let xs_discrete_offsets = parse_offsets_or_fixed(
            &response,
            "xs_discrete_offsets",
            nr_samples,
            discrete_dims,
            discrete_len,
        )?;
        let xs_continuous_offsets = parse_offsets_or_fixed(
            &response,
            "xs_continuous_offsets",
            nr_samples,
            continuous_dims,
            continuous_len,
        )?;
        let (xs_discrete_row_major, next) =
            read_le_i64(&binary, 0, discrete_len).map_err(EngineError::engine)?;
        let (xs_continuous_row_major, next) =
            read_le_f64(&binary, next, continuous_len).map_err(EngineError::engine)?;
        let (weights, _next) =
            read_le_f64(&binary, next, nr_samples).map_err(EngineError::engine)?;
        for (index, weight) in weights.iter().enumerate() {
            if !weight.is_finite() || *weight <= 0.0 {
                return Err(EngineError::engine(format!(
                    "process sampler returned non-positive or non-finite value at weights[{index}]"
                )));
            }
        }
        Ok(ProcessSampleBatch {
            xs_discrete_row_major,
            xs_discrete_offsets,
            xs_continuous_row_major,
            xs_continuous_offsets,
            weights,
        })
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let mut binary = Vec::with_capacity(training_values.len() * 8);
        extend_le_f64(&mut binary, training_values);
        let (response, _binary) = self
            .process
            .request_with_binary(
                "ingest_training_values",
                serde_json::json!({ "nr_values": training_values.len() }),
                &binary,
            )
            .map_err(EngineError::engine)?;
        Self::expect_ack(response).map_err(EngineError::engine)
    }

    fn snapshot(&mut self) -> Result<Value, EngineError> {
        let response = self
            .process
            .request("snapshot", serde_json::json!({}))
            .map_err(EngineError::engine)?;
        response
            .get("snapshot")
            .cloned()
            .ok_or_else(|| EngineError::engine("process sampler response missing 'snapshot'"))
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        let mut xs_discrete_row_major = Vec::new();
        let mut xs_discrete_offsets = Vec::with_capacity(points.len() + 1);
        let mut xs_continuous_row_major = Vec::new();
        let mut xs_continuous_offsets = Vec::with_capacity(points.len() + 1);
        xs_discrete_offsets.push(0);
        xs_continuous_offsets.push(0);
        for point in points {
            xs_discrete_row_major.extend_from_slice(&point.0);
            xs_discrete_offsets.push(xs_discrete_row_major.len());
            xs_continuous_row_major.extend_from_slice(&point.1);
            xs_continuous_offsets.push(xs_continuous_row_major.len());
        }
        let response = self
            .process
            .request(
                "pdf",
                serde_json::json!({
                    "nr_samples": points.len(),
                    "xs_discrete_row_major": xs_discrete_row_major,
                    "xs_discrete_offsets": xs_discrete_offsets,
                    "xs_continuous_row_major": xs_continuous_row_major,
                    "xs_continuous_offsets": xs_continuous_offsets,
                }),
            )
            .map_err(EngineError::engine)?;
        match response.get("values") {
            Some(Value::Null) | None => Ok(vec![None; points.len()]),
            Some(Value::Array(values)) => {
                if values.len() != points.len() {
                    return Err(EngineError::engine(format!(
                        "process sampler pdf output size mismatch: expected {}, got {}",
                        points.len(),
                        values.len()
                    )));
                }
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if value.is_null() {
                            return Ok(None);
                        }
                        value.as_f64().map(Some).ok_or_else(|| {
                            EngineError::engine(format!(
                                "process sampler response field 'values[{index}]' must be f64 or null"
                            ))
                        })
                    })
                    .collect()
            }
            Some(_) => Err(EngineError::engine(
                "process sampler response field 'values' must be an array or null",
            )),
        }
    }

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        let response = match self.process.request(
            "discrete_pdf",
            serde_json::json!({
                "subspaces": subspaces.iter().map(|subspace| {
                    serde_json::json!({
                        "fixed_dims": subspace.fixed_dims.iter().map(|(dim, value)| {
                            serde_json::json!({"dim": dim, "value": value})
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            }),
        ) {
            Ok(response) => response,
            Err(err)
                if err.contains("unknown method")
                    || err.contains("method not found")
                    || err.contains("Method not found") =>
            {
                return Ok(vec![None; subspaces.len()]);
            }
            Err(err) => return Err(EngineError::engine(err)),
        };
        match response.get("values") {
            Some(Value::Null) | None => Ok(vec![None; subspaces.len()]),
            Some(Value::Array(values)) => {
                if values.len() != subspaces.len() {
                    return Err(EngineError::engine(format!(
                        "process sampler discrete_pdf output size mismatch: expected {}, got {}",
                        subspaces.len(),
                        values.len()
                    )));
                }
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if value.is_null() {
                            return Ok(None);
                        }
                        value.as_f64().map(Some).ok_or_else(|| {
                            EngineError::engine(format!(
                                "process sampler response field 'values[{index}]' must be f64 or null"
                            ))
                        })
                    })
                    .collect()
            }
            Some(_) => Err(EngineError::engine(
                "process sampler response field 'values' must be an array or null",
            )),
        }
    }

    fn get_diagnostics(&mut self) -> Result<Value, EngineError> {
        let response = self
            .process
            .request("get_diagnostics", serde_json::json!({}))
            .map_err(EngineError::engine)?;
        Ok(response
            .get("diagnostics")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})))
    }

    fn expect_ack(response: Value) -> Result<(), String> {
        if response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(());
        }
        Err("process sampler initialize result missing ok=true".to_string())
    }
}

fn default_args() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Total row-major length implied by an offsets field: its last entry when
/// present, otherwise the homogeneous `nr_samples * fixed_width`.
fn offsets_total_len(
    response: &Value,
    field: &str,
    nr_samples: usize,
    fixed_width: usize,
) -> Result<usize, EngineError> {
    match response.get(field).and_then(Value::as_array) {
        Some(offsets) => offsets
            .last()
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| {
                EngineError::engine(format!(
                    "process sampler response field '{field}' must be a non-empty integer array"
                ))
            }),
        None => Ok(nr_samples.saturating_mul(fixed_width)),
    }
}

fn parse_offsets_or_fixed(
    response: &Value,
    field: &str,
    nr_samples: usize,
    fixed_width: usize,
    values_len: usize,
) -> Result<Vec<usize>, EngineError> {
    let Some(raw) = response.get(field) else {
        let expected_len = nr_samples.saturating_mul(fixed_width);
        if values_len != expected_len {
            return Err(EngineError::engine(format!(
                "process sampler response missing '{field}' for ragged output with {values_len} values; fixed-width fallback expected {expected_len}"
            )));
        }
        return Ok((0..=nr_samples).map(|index| index * fixed_width).collect());
    };
    let offsets = raw.as_array().ok_or_else(|| {
        EngineError::engine(format!(
            "process sampler response field '{field}' must be an array"
        ))
    })?;
    if offsets.len() != nr_samples + 1 {
        return Err(EngineError::engine(format!(
            "process sampler response field '{field}' must contain nr_samples + 1 offsets"
        )));
    }
    let mut parsed = Vec::with_capacity(offsets.len());
    let mut previous = 0_usize;
    for (index, value) in offsets.iter().enumerate() {
        let offset = value.as_u64().ok_or_else(|| {
            EngineError::engine(format!(
                "process sampler response field '{field}[{index}]' must be a non-negative integer"
            ))
        })? as usize;
        if index == 0 && offset != 0 {
            return Err(EngineError::engine(format!(
                "process sampler response field '{field}' must start at 0"
            )));
        }
        if offset < previous {
            return Err(EngineError::engine(format!(
                "process sampler response field '{field}' must be non-decreasing"
            )));
        }
        if offset > values_len {
            return Err(EngineError::engine(format!(
                "process sampler response field '{field}[{index}]' exceeds row-major value length {values_len}"
            )));
        }
        previous = offset;
        parsed.push(offset);
    }
    if parsed.last().copied() != Some(values_len) {
        return Err(EngineError::engine(format!(
            "process sampler response field '{field}' must end at row-major value length {values_len}"
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{ProcessSampler, ProcessSamplerParams};
    use crate::sampling::{SamplerAggregator, SamplerAggregatorSnapshot};
    use crate::utils::domain::Domain;
    use serde_json::json;

    const METADATA_ECHO_SAMPLER_WORKER: &str = r#"
import json, sys

state = {}

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
        state["evaluator_metadata"] = params.get("evaluator_metadata")
        send_result(req.get("id"), {"ok": True})
    elif method == "snapshot":
        send_result(req.get("id"), {"snapshot": state})
    else:
        send_result(req.get("id"), {"ok": True})
"#;

    #[test]
    fn process_sampler_deserializes_command_and_args() {
        let params = toml::from_str::<ProcessSamplerParams>(
            r#"
command = ["python", "-u", "worker.py"]
cwd = "$resources"
requires_training_values = true
args = { seed = 0 }
"#,
        )
        .expect("process sampler config should parse");

        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.cwd.as_deref(), Some("$resources"));
        assert!(params.requires_training_values);
        assert!(params.args.is_object());
    }

    #[test]
    fn process_sampler_params_no_longer_define_domain_shape() {
        let params = ProcessSamplerParams {
            command: vec!["worker".to_string()],
            cwd: None,
            requires_training_values: false,
            args: serde_json::json!({}),
        };
        assert_eq!(params.command, ["worker"]);
        let domain = Domain::discrete(
            Some("branch".to_string()),
            [
                crate::utils::domain::DomainBranch::new(0, Domain::continuous(1)),
                crate::utils::domain::DomainBranch::new(1, Domain::continuous(3)),
            ],
        );
        assert_eq!(domain.fixed_rectangular_dims(), None);
    }

    #[test]
    fn process_sampler_initialize_receives_evaluator_metadata() {
        let python =
            std::env::var("GAMMABOARD_TEST_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let params = ProcessSamplerParams {
            command: vec![
                python,
                "-u".to_string(),
                "-c".to_string(),
                METADATA_ECHO_SAMPLER_WORKER.to_string(),
            ],
            cwd: None,
            requires_training_values: false,
            args: json!({}),
        };
        let metadata = json!({"space": "momentum", "loop_count": 2});
        let mut sampler = ProcessSampler::from_params_and_domain(
            params,
            &Domain::continuous(6),
            metadata.clone(),
        )
        .expect("process sampler should initialize");

        let SamplerAggregatorSnapshot::ProcessSampler { raw } =
            sampler.snapshot().expect("snapshot")
        else {
            panic!("expected process sampler snapshot");
        };
        assert_eq!(
            raw.pointer("/sampler_state/evaluator_metadata"),
            Some(&metadata)
        );
    }
}
