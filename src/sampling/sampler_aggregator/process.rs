use serde_json::Value;

use crate::core::EngineError;
use crate::process_runtime::{build_process_worker_command, parse_process_offsets};
use crate::process_worker::{
    PROCESS_PROTOCOL, ProcessWorker, extend_le_f64, read_le_f64, read_le_i64,
};
use crate::sampling::latent_batch::IndexedBatchBuilder;
use crate::sampling::{DiscreteSubspace, Generation, LatentBatch, PdfPoint, Sampler};
use crate::utils::domain::Domain;

use crate::ProcessConfig;

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Initialization {
    Fresh,
    Restore {
        state: Value,
    },
    FromStage {
        source_state: Value,
        source_args: Value,
    },
}

pub struct ProcessSampler {
    domain: Domain,
    worker: ProcessSamplerWorker,
}

impl ProcessSampler {
    pub(crate) fn from_params_and_domain(
        params: ProcessConfig,
        domain: &Domain,
        evaluator_metadata: Value,
    ) -> Result<Self, EngineError> {
        params.validate()?;
        let worker = ProcessSamplerWorker::spawn(
            &params,
            domain.clone(),
            Initialization::Fresh,
            evaluator_metadata,
        )?;
        Ok(Self {
            domain: domain.clone(),
            worker,
        })
    }

    pub(crate) fn from_stage(
        params: ProcessConfig,
        source: &crate::sampling::SavedSampler,
        domain: &Domain,
        evaluator_metadata: Value,
    ) -> Result<Self, EngineError> {
        params.validate()?;
        let crate::core::SamplerConfig::ProcessSampler {
            params: source_params,
        } = &source.config.sampler
        else {
            return Err(EngineError::build(
                "process stage handoff requires a process sampler source",
            ));
        };
        let worker = ProcessSamplerWorker::spawn(
            &params,
            domain.clone(),
            Initialization::FromStage {
                source_state: source.state.clone(),
                source_args: source_params.args.clone(),
            },
            evaluator_metadata,
        )?;
        Ok(Self {
            domain: domain.clone(),
            worker,
        })
    }

    pub(crate) fn from_snapshot(
        params: ProcessConfig,
        state: Value,
        domain: &Domain,
        evaluator_metadata: Value,
    ) -> Result<Self, EngineError> {
        params.validate()?;
        let worker = ProcessSamplerWorker::spawn(
            &params,
            domain.clone(),
            Initialization::Restore { state },
            evaluator_metadata,
        )?;
        Ok(Self {
            domain: domain.clone(),
            worker,
        })
    }
}

impl Sampler for ProcessSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        if domain != &self.domain {
            return Err(EngineError::build(format!(
                "process_sampler domain mismatch: expected {:?}, got {:?}",
                self.domain, domain
            )));
        }
        Ok(())
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        if max_samples == 0 {
            return Err(EngineError::invalid_input("max_samples must be positive"));
        }
        self.worker.generate(max_samples)
    }

    fn feedback(&mut self, feedback: &crate::sampling::Feedback) -> Result<(), EngineError> {
        self.worker.feedback(feedback)
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker.pdf_batch(points)
    }

    fn discrete_mass_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker.discrete_mass_batch(subspaces)
    }

    fn snapshot(&mut self) -> Result<Value, EngineError> {
        self.worker.snapshot()
    }

    fn persisted_output(&mut self) -> Result<Option<Value>, EngineError> {
        let output = self.worker.request_field("persisted_output", "output")?;
        Ok((!output.is_null()).then_some(output))
    }

    fn diagnostics(&mut self) -> Value {
        self.worker
            .diagnostics()
            .unwrap_or_else(|error| serde_json::json!({"error": error.to_string()}))
    }
}

struct ProcessSamplerWorker {
    process: ProcessWorker,
    domain: Domain,
}

impl ProcessSamplerWorker {
    fn spawn(
        params: &ProcessConfig,
        domain: Domain,
        initialization: Initialization,
        evaluator_metadata: Value,
    ) -> Result<Self, EngineError> {
        let mut command =
            build_process_worker_command(&params.command, params.cwd.as_deref(), "sampler")?;
        let process = ProcessWorker::spawn(
            &mut command,
            "process sampler",
            params.shutdown_grace_seconds,
        )?;
        let mut worker = Self { process, domain };
        worker.send_init(params.args.clone(), initialization, evaluator_metadata)?;
        Ok(worker)
    }

    fn send_init(
        &mut self,
        args: Value,
        initialization: Initialization,
        evaluator_metadata: Value,
    ) -> Result<(), EngineError> {
        let response = self
            .process
            .request(
                "initialize",
                serde_json::json!({
                    "protocol": PROCESS_PROTOCOL,
                    "role": "sampler",
                    "domain": self.domain,
                    "args": args,
                    "initialization": initialization,
                    "evaluator_metadata": evaluator_metadata,
                }),
            )
            .map_err(EngineError::build)?;
        Self::expect_ack(response).map_err(EngineError::build)
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        let (response, binary) = self
            .process
            .request_with_binary(
                "generate",
                serde_json::json!({"max_samples": max_samples}),
                &[],
            )
            .map_err(EngineError::engine)?;
        decode_generation(&self.domain, max_samples, &response, &binary)
    }

    fn feedback(&mut self, feedback: &crate::sampling::Feedback) -> Result<(), EngineError> {
        feedback.validate()?;
        let prefixes = feedback.strata.as_ref().map(|s| &s.prefixes);
        let mut binary =
            Vec::with_capacity(feedback.len() * if prefixes.is_some() { 24 } else { 8 });
        extend_le_f64(&mut binary, &feedback.unmasked_values);
        if let Some(strata) = &feedback.strata {
            for &label in strata.requested.iter().chain(&strata.actual) {
                binary.extend_from_slice(&(label as u64).to_le_bytes());
            }
        }
        let (response, _binary) = self
            .process
            .request_with_binary(
                "feedback",
                serde_json::json!({ "nr_values": feedback.len(), "stratum_prefixes": prefixes }),
                &binary,
            )
            .map_err(EngineError::engine)?;
        Self::expect_ack(response).map_err(EngineError::engine)
    }

    fn snapshot(&mut self) -> Result<Value, EngineError> {
        let snapshot = self.request_field("snapshot", "snapshot")?;
        if snapshot.is_null() {
            return Err(EngineError::engine(
                "process sampler snapshot must not be null",
            ));
        }
        Ok(snapshot)
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
        decode_probabilities(&response, points.len(), false)
    }

    fn discrete_mass_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        let response = self
            .process
            .request(
                "discrete_mass",
                serde_json::json!({
                    "subspaces": subspaces.iter().map(|subspace| {
                        serde_json::json!({
                            "fixed_dims": subspace.fixed_dims.iter().map(|(dim, value)| {
                                serde_json::json!({"dim": dim, "value": value})
                            }).collect::<Vec<_>>(),
                        })
                    }).collect::<Vec<_>>(),
                }),
            )
            .map_err(EngineError::engine)?;
        decode_probabilities(&response, subspaces.len(), true)
    }

    fn diagnostics(&mut self) -> Result<Value, EngineError> {
        self.request_field("diagnostics", "diagnostics")
    }

    fn request_field(&mut self, method: &str, field: &str) -> Result<Value, EngineError> {
        let response = self
            .process
            .request(method, serde_json::json!({}))
            .map_err(EngineError::engine)?;
        response.get(field).cloned().ok_or_else(|| {
            EngineError::engine(format!(
                "process sampler {method} response missing '{field}'"
            ))
        })
    }

    fn expect_ack(response: Value) -> Result<(), String> {
        if response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(());
        }
        Err("process sampler initialize result missing ok=true".to_string())
    }
}

/// Decode one generation without changing its sampler-owned flat representation.
fn decode_probabilities(
    response: &Value,
    size: usize,
    mass: bool,
) -> Result<Vec<Option<f64>>, EngineError> {
    match response.get("values") {
        Some(Value::Null) => Ok(vec![None; size]),
        Some(Value::Array(values)) if values.len() == size => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    return Ok(None);
                }
                value
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.0 && (!mass || *v <= 1.0))
                    .map(Some)
                    .ok_or_else(|| EngineError::engine("invalid normalized probability result"))
            })
            .collect(),
        _ => Err(EngineError::engine(
            "probability response must contain one number-or-null per query, or null",
        )),
    }
}

fn decode_generation(
    domain: &Domain,
    max_samples: usize,
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
        .filter(|n| *n > 0 && *n <= max_samples)
        .ok_or_else(|| {
            EngineError::engine("process draw exceeds max_samples or has invalid sample count")
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
        LatentBatch {
            nr_samples,
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
            } = super::decode_generation(&domain, 3, &response, &binary).unwrap()
            else {
                panic!("expected batch");
            };
            assert_eq!(training_remaining, Some(10));
            batch.validate_nr_samples().unwrap();
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
            let generic = super::decode_generation(&domain, 3, &response, &binary)
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
        let batch = super::decode_generation(&domain, 100, &response, &binary)
            .unwrap()
            .into_batch()
            .unwrap();
        let materialized = batch.payload.into_batch().unwrap();
        domain.validate_batch(&materialized).unwrap();
        assert_eq!(materialized.points()[0].continuous, [0.2]);
        assert_eq!(materialized.points()[1].continuous, [0.3, 0.4]);
        assert!(super::decode_generation(&domain, 1, &response, &binary).is_err());
        for offsets in [
            json!([0, 3, 2]),
            json!([0, 1]),
            json!([1, 2, 3]),
            json!(null),
        ] {
            let mut invalid = response.clone();
            invalid["xs_continuous_offsets"] = offsets;
            assert!(super::decode_generation(&domain, 100, &invalid, &binary).is_err());
        }
        for size in [binary.len() - 1, binary.len() + 1] {
            let mut invalid = binary.clone();
            invalid.resize(size, 0);
            assert!(super::decode_generation(&domain, 100, &response, &invalid).is_err());
        }
        for weight in [0.0_f64, -1.0, f64::NAN, f64::INFINITY] {
            let mut invalid = binary.clone();
            invalid[binary.len() - 8..].copy_from_slice(&weight.to_le_bytes());
            assert!(super::decode_generation(&domain, 100, &response, &invalid).is_err());
        }
    }
}
