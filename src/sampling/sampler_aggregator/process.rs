use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError};
use crate::process_runtime::{
    build_process_worker_command, default_process_args, parse_process_offsets,
};
use crate::process_worker::{
    PROCESS_PROTOCOL, ProcessWorker, default_process_shutdown_grace_seconds, extend_le_f64,
    read_le_f64, read_le_i64,
};
use crate::sampling::latent_batch::IndexedBatchBuilder;
use crate::sampling::{
    DiscreteSubspace, Generation, LatentBatchSpec, PdfPoint, SamplerAggregator,
    SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessSamplerParams {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub requires_training_values: bool,
    #[serde(default = "default_process_args")]
    pub args: Value,
    #[serde(default = "default_process_shutdown_grace_seconds")]
    pub shutdown_grace_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProcessSamplerSnapshot {
    params: ProcessSamplerParams,
    sampler_state: Value,
}

pub struct ProcessSampler {
    params: ProcessSamplerParams,
    domain: Domain,
    worker: ProcessSamplerWorker,
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
            worker,
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
            worker,
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

    fn generate(
        &mut self,
        remaining_sample_budget: Option<usize>,
    ) -> Result<Generation, EngineError> {
        self.worker.generate(remaining_sample_budget)
    }

    fn feedback(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        self.worker.feedback(training_values)
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker.pdf_batch(points)
    }

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker.discrete_pdf_batch(subspaces)
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let sampler_state = self.worker.snapshot()?;
        let raw = serde_json::to_value(ProcessSamplerSnapshot {
            params: self.params.clone(),
            sampler_state,
        })
        .map_err(EngineError::from)?;
        Ok(SamplerAggregatorSnapshot::ProcessSampler { raw })
    }

    fn get_diagnostics(&mut self) -> Value {
        self.worker
            .get_diagnostics()
            .unwrap_or_else(|_| serde_json::json!({}))
    }
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
        let process = ProcessWorker::spawn(
            &mut command,
            "process sampler",
            params.shutdown_grace_seconds,
        )?;
        let mut worker = Self { process, domain };
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

    fn generate(
        &mut self,
        remaining_sample_budget: Option<usize>,
    ) -> Result<Generation, EngineError> {
        let (response, binary) = self
            .process
            .request_with_binary(
                "generate",
                serde_json::json!({"remaining_sample_budget": remaining_sample_budget}),
                &[],
            )
            .map_err(EngineError::engine)?;
        decode_generation(&self.domain, remaining_sample_budget, &response, &binary)
    }

    fn feedback(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let mut binary = Vec::with_capacity(training_values.len() * 8);
        extend_le_f64(&mut binary, training_values);
        let (response, _binary) = self
            .process
            .request_with_binary(
                "feedback",
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

/// Decode one generation without changing its sampler-owned flat representation.
fn decode_generation(
    domain: &Domain,
    remaining_sample_budget: Option<usize>,
    response: &Value,
    binary: &[u8],
) -> Result<Generation, EngineError> {
    match response.get("kind").and_then(Value::as_str) {
        Some("waiting") if binary.is_empty() => return Ok(Generation::Waiting),
        Some("finished") if binary.is_empty() => return Ok(Generation::Finished),
        Some("batch") => {}
        _ => return Err(EngineError::engine("invalid process generation result")),
    }
    let nr_samples = response
        .get("nr_samples")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0 && remaining_sample_budget.is_none_or(|budget| *n <= budget))
        .ok_or_else(|| {
            EngineError::engine("process draw exceeds budget or has invalid sample count")
        })?;
    let training_remaining = match response.get("training_remaining") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n >= nr_samples)
                .ok_or_else(|| EngineError::engine("invalid process training window"))?,
        ),
    };
    // Binary block layout: i64 discrete, f64 continuous, f64 weights. Offsets
    // (and thus the array lengths) come from the JSON envelope.
    let discrete_dims = domain.fixed_discrete_depth().unwrap_or(0);
    let continuous_dims = domain.fixed_continuous_dims().unwrap_or(0);
    let discrete_len =
        offsets_total_len(response, "xs_discrete_offsets", nr_samples, discrete_dims)?;
    let continuous_len = offsets_total_len(
        response,
        "xs_continuous_offsets",
        nr_samples,
        continuous_dims,
    )?;
    let (xs_discrete_row_major, next) =
        read_le_i64(binary, 0, discrete_len).map_err(EngineError::engine)?;
    let (xs_continuous_row_major, next) =
        read_le_f64(binary, next, continuous_len).map_err(EngineError::engine)?;
    let (weights, _next) = read_le_f64(binary, next, nr_samples).map_err(EngineError::engine)?;
    for (index, weight) in weights.iter().enumerate() {
        if !weight.is_finite() || *weight <= 0.0 {
            return Err(EngineError::engine(format!(
                "process sampler returned non-positive or non-finite value at weights[{index}]"
            )));
        }
    }
    if _next != binary.len() {
        return Err(EngineError::engine("trailing process sample bytes"));
    }
    let homogeneous = domain.fixed_rectangular_dims().filter(|_| {
        response.get("xs_discrete_offsets").is_none()
            && response.get("xs_continuous_offsets").is_none()
    });
    let payload = if let Some((continuous_dims, discrete_dims)) = homogeneous {
        IndexedBatchBuilder::from_homogeneous(
            &xs_discrete_row_major,
            discrete_dims,
            xs_continuous_row_major,
            continuous_dims,
            weights,
        )
    } else {
        let xs_discrete_offsets = parse_process_offsets(
            response,
            "xs_discrete_offsets",
            nr_samples,
            discrete_dims,
            discrete_len,
            "sampler",
        )?;
        let xs_continuous_offsets = parse_process_offsets(
            response,
            "xs_continuous_offsets",
            nr_samples,
            continuous_dims,
            continuous_len,
            "sampler",
        )?;
        let mut builder = IndexedBatchBuilder::new(nr_samples);
        for index in 0..nr_samples {
            builder.push(
                &xs_discrete_row_major[xs_discrete_offsets[index]..xs_discrete_offsets[index + 1]],
                &xs_continuous_row_major
                    [xs_continuous_offsets[index]..xs_continuous_offsets[index + 1]],
                weights[index],
            );
        }
        builder.finish()
    };
    Ok(Generation::batch(
        LatentBatchSpec {
            nr_samples,
            accumulator: crate::core::AccumulatorConfig::scalar(),
            payload,
        },
        training_remaining,
    ))
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

#[cfg(test)]
mod tests {
    use super::{ProcessSampler, ProcessSamplerParams};
    use crate::sampling::{SamplerAggregator, SamplerAggregatorSnapshot};
    use crate::utils::domain::Domain;
    use serde_json::json;

    #[test]
    fn flat_generation_matches_explicit_offsets_and_preserves_training_window() {
        for discrete_dims in [0, 1] {
            let domain = Domain::rectangular_with_cardinalities(2, vec![2; discrete_dims]);
            let discrete = if discrete_dims == 0 {
                vec![]
            } else {
                vec![0_i64, 1, 0]
            };
            let continuous = [0.1_f64, 0.2, 0.3, 0.4, 0.5, 0.6];
            let weights = [1.0_f64, 2.0, 3.0];
            let binary: Vec<u8> = discrete
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .chain(continuous.iter().flat_map(|v| v.to_le_bytes()))
                .chain(weights.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            let mut response = json!({"kind":"batch", "nr_samples":3, "training_remaining":10});
            let crate::Generation::Batch {
                batch,
                training_remaining,
            } = super::decode_generation(&domain, Some(3), &response, &binary).unwrap()
            else {
                panic!("expected batch");
            };
            assert_eq!(training_remaining, Some(10));
            batch.clone().build().validate_nr_samples().unwrap();
            let materialized = batch.payload.as_batch().unwrap();
            domain.validate_batch(&materialized).unwrap();
            for (index, point) in materialized.points().iter().enumerate() {
                assert_eq!(point.continuous, continuous[index * 2..index * 2 + 2]);
                assert_eq!(
                    point.discrete,
                    discrete[index * discrete_dims..(index + 1) * discrete_dims]
                );
                assert_eq!(point.total_weight(), weights[index]);
            }
            response["xs_continuous_offsets"] = json!([0, 2, 4, 6]);
            response["xs_discrete_offsets"] =
                json!((0..=3).map(|n| n * discrete_dims).collect::<Vec<_>>());
            let generic = super::decode_generation(&domain, Some(3), &response, &binary)
                .unwrap()
                .into_batch()
                .unwrap();
            assert_eq!(batch, generic);
        }
    }

    #[test]
    fn generation_decoder_preserves_ragged_layout_and_rejects_invalid_frames() {
        use crate::utils::domain::DomainBranch;
        let domain = Domain::discrete(
            None,
            [
                DomainBranch::new(0, Domain::continuous(1)),
                DomainBranch::new(1, Domain::continuous(2)),
            ],
        );
        let response = json!({"kind":"batch", "nr_samples":2,
            "xs_discrete_offsets":[0,1,2], "xs_continuous_offsets":[0,1,3]});
        let binary: Vec<u8> = [0_i64, 1]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .chain(
                [0.2_f64, 0.3, 0.4, 1.0, 3.0]
                    .iter()
                    .flat_map(|v| v.to_le_bytes()),
            )
            .collect();
        let batch = super::decode_generation(&domain, None, &response, &binary)
            .unwrap()
            .into_batch()
            .unwrap();
        let materialized = batch.payload.into_batch().unwrap();
        domain.validate_batch(&materialized).unwrap();
        assert_eq!(materialized.points()[0].continuous, [0.2]);
        assert_eq!(materialized.points()[1].continuous, [0.3, 0.4]);
        assert!(super::decode_generation(&domain, Some(1), &response, &binary).is_err());
        for offsets in [
            json!([0, 3, 2]),
            json!([0, 1]),
            json!([1, 2, 3]),
            json!(null),
        ] {
            let mut invalid = response.clone();
            invalid["xs_continuous_offsets"] = offsets;
            assert!(super::decode_generation(&domain, None, &invalid, &binary).is_err());
        }
        for size in [binary.len() - 1, binary.len() + 1] {
            let mut invalid = binary.clone();
            invalid.resize(size, 0);
            assert!(super::decode_generation(&domain, None, &response, &invalid).is_err());
        }
        for weight in [0.0_f64, -1.0, f64::NAN, f64::INFINITY] {
            let mut invalid = binary.clone();
            invalid[binary.len() - 8..].copy_from_slice(&weight.to_le_bytes());
            assert!(super::decode_generation(&domain, None, &response, &invalid).is_err());
        }
    }

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
            shutdown_grace_seconds: 30,
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
            shutdown_grace_seconds: 30,
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
