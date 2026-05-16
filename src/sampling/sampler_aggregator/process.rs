use std::process::Stdio;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Point};
use crate::process_runtime::build_process_worker_command;
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker, pipe_process_stderr};
use crate::sampling::{
    LatentBatchSpec, PdfPoint, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessSamplerParams {
    pub command: Vec<String>,
    pub continuous_dims: usize,
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
    discrete_cardinalities: Vec<usize>,
    worker: Mutex<ProcessSamplerWorker>,
}

impl ProcessSampler {
    pub(crate) fn from_params_and_domain(
        params: ProcessSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_params(&params)?;
        let discrete_cardinalities = validate_homogeneous_domain(domain, params.continuous_dims)?;
        let worker = ProcessSamplerWorker::spawn(&params, &discrete_cardinalities, None)?;
        Ok(Self {
            params,
            discrete_cardinalities,
            worker: Mutex::new(worker),
        })
    }

    pub(crate) fn from_snapshot(
        snapshot: ProcessSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_params(&snapshot.params)?;
        let discrete_cardinalities =
            validate_homogeneous_domain(domain, snapshot.params.continuous_dims)?;
        let worker = ProcessSamplerWorker::spawn(
            &snapshot.params,
            &discrete_cardinalities,
            Some(snapshot.sampler_state),
        )?;
        Ok(Self {
            params: snapshot.params,
            discrete_cardinalities,
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
        let discrete_cardinalities =
            validate_homogeneous_domain(domain, self.params.continuous_dims)?;
        if discrete_cardinalities != self.discrete_cardinalities {
            return Err(BuildError::build(format!(
                "process_sampler expects discrete_cardinalities={:?}, got {:?} from domain",
                self.discrete_cardinalities, discrete_cardinalities
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
        let discrete_dims = self.discrete_cardinalities.len();
        for sample_idx in 0..nr_samples {
            let discrete_start = sample_idx * discrete_dims;
            let discrete_end = discrete_start + discrete_dims;
            let continuous_start = sample_idx * self.params.continuous_dims;
            let continuous_end = continuous_start + self.params.continuous_dims;
            points.push(Point::new(
                sample_batch.xs_continuous_row_major[continuous_start..continuous_end].to_vec(),
                sample_batch.xs_discrete_row_major[discrete_start..discrete_end].to_vec(),
                sample_batch.weights[sample_idx],
            ));
        }
        let batch = Batch::new(points).map_err(|err| EngineError::engine(err.to_string()))?;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        self.worker_mut()?.ingest_training_values(training_values)
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        self.worker_mut()?.pdf_batch(points)
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

fn validate_homogeneous_domain(
    domain: &Domain,
    expected_continuous_dims: usize,
) -> Result<Vec<usize>, BuildError> {
    let (actual_continuous_dims, actual_discrete_dims) = domain
        .fixed_rectangular_dims()
        .ok_or_else(|| BuildError::build("process_sampler requires a fixed rectangular domain"))?;
    if actual_continuous_dims != expected_continuous_dims {
        return Err(BuildError::build(format!(
            "process_sampler expects continuous_dims={}, got {} from domain",
            expected_continuous_dims, actual_continuous_dims
        )));
    }
    let discrete_cardinalities = domain.fixed_discrete_cardinalities().ok_or_else(|| {
        BuildError::build("process_sampler requires fixed discrete cardinalities")
    })?;
    if discrete_cardinalities.len() != actual_discrete_dims {
        return Err(BuildError::build(format!(
            "process_sampler domain cardinality mismatch: depth={}, cardinalities={:?}",
            actual_discrete_dims, discrete_cardinalities
        )));
    }
    Ok(discrete_cardinalities)
}

struct ProcessSampleBatch {
    xs_discrete_row_major: Vec<i64>,
    xs_continuous_row_major: Vec<f64>,
    weights: Vec<f64>,
}

struct ProcessSamplerWorker {
    process: ProcessWorker,
    discrete_cardinalities: Vec<usize>,
    continuous_dims: usize,
}

impl ProcessSamplerWorker {
    fn spawn(
        params: &ProcessSamplerParams,
        discrete_cardinalities: &[usize],
        snapshot: Option<Value>,
    ) -> Result<Self, BuildError> {
        let mut command = build_process_worker_command(&params.command, "sampler")?;
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
            discrete_cardinalities: discrete_cardinalities.to_vec(),
            continuous_dims: params.continuous_dims,
        };
        worker.send_init(
            discrete_cardinalities,
            params.continuous_dims,
            params.args.clone(),
            snapshot,
        )?;
        Ok(worker)
    }

    fn send_init(
        &mut self,
        discrete_cardinalities: &[usize],
        continuous_dims: usize,
        args: Value,
        snapshot: Option<Value>,
    ) -> Result<(), BuildError> {
        let response = self
            .process
            .request(
                "initialize",
                serde_json::json!({
                    "protocol": PROCESS_PROTOCOL,
                    "role": "sampler",
                    "domain": {
                        "continuous_dims": continuous_dims,
                        "discrete_cardinalities": discrete_cardinalities,
                    },
                    "discrete_cardinalities": discrete_cardinalities,
                    "continuous_dims": continuous_dims,
                    "args": args,
                    "snapshot": snapshot,
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
        let response = self
            .process
            .request(
                "produce_latent_batch",
                serde_json::json!({
                    "nr_samples": nr_samples,
                }),
            )
            .map_err(EngineError::engine)?;
        let xs_discrete = response
            .get("xs_discrete_row_major")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine(
                    "process sampler response missing 'xs_discrete_row_major' array",
                )
            })?;
        let discrete_dims = self.discrete_cardinalities.len();
        let expected_discrete_len = nr_samples.saturating_mul(discrete_dims);
        if xs_discrete.len() != expected_discrete_len {
            return Err(EngineError::engine(format!(
                "process sampler discrete output size mismatch: expected {} values, got {}",
                expected_discrete_len,
                xs_discrete.len()
            )));
        }
        let xs_continuous = response
            .get("xs_continuous_row_major")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine(
                    "process sampler response missing 'xs_continuous_row_major' array",
                )
            })?;
        let expected_continuous_len = nr_samples.saturating_mul(self.continuous_dims);
        if xs_continuous.len() != expected_continuous_len {
            return Err(EngineError::engine(format!(
                "process sampler continuous output size mismatch: expected {} values, got {}",
                expected_continuous_len,
                xs_continuous.len()
            )));
        }
        let xs_discrete_row_major = xs_discrete
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_i64().ok_or_else(|| {
                    EngineError::engine(format!(
                        "process sampler returned non-i64 value at xs_discrete_row_major[{index}]"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let xs_continuous_row_major = xs_continuous
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_f64().ok_or_else(|| {
                    EngineError::engine(format!(
                        "process sampler returned non-f64 value at xs_continuous_row_major[{index}]"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let weights = response
            .get("weights")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                EngineError::engine("process sampler response missing 'weights' array")
            })?;
        if weights.len() != nr_samples {
            return Err(EngineError::engine(format!(
                "process sampler weights size mismatch: expected {} values, got {}",
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
                        "process sampler returned non-f64 value at weights[{index}]"
                    ))
                })?;
                if !weight.is_finite() || weight <= 0.0 {
                    return Err(EngineError::engine(format!(
                        "process sampler returned non-positive or non-finite value at weights[{index}]"
                    )));
                }
                Ok(weight)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ProcessSampleBatch {
            xs_discrete_row_major,
            xs_continuous_row_major,
            weights,
        })
    }

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let response = self
            .process
            .request(
                "ingest_training_values",
                serde_json::json!({
                    "training_values": training_values,
                }),
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
        let discrete_dims = self.discrete_cardinalities.len();
        let continuous_dims = self.continuous_dims;
        let mut xs_discrete_row_major = Vec::with_capacity(points.len() * discrete_dims);
        let mut xs_continuous_row_major = Vec::with_capacity(points.len() * continuous_dims);
        for (index, point) in points.iter().enumerate() {
            if point.0.len() != discrete_dims {
                return Err(EngineError::engine(format!(
                    "process sampler pdf discrete dimension mismatch at point {index}: expected {}, got {}",
                    discrete_dims,
                    point.0.len()
                )));
            }
            if point.1.len() != continuous_dims {
                return Err(EngineError::engine(format!(
                    "process sampler pdf continuous dimension mismatch at point {index}: expected {}, got {}",
                    continuous_dims,
                    point.1.len()
                )));
            }
            xs_discrete_row_major.extend_from_slice(&point.0);
            xs_continuous_row_major.extend_from_slice(&point.1);
        }
        let response = self
            .process
            .request(
                "pdf",
                serde_json::json!({
                    "nr_samples": points.len(),
                    "xs_discrete_row_major": xs_discrete_row_major,
                    "xs_continuous_row_major": xs_continuous_row_major,
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

#[cfg(test)]
mod tests {
    use super::{ProcessSamplerParams, validate_homogeneous_domain};
    use crate::utils::domain::Domain;

    #[test]
    fn process_sampler_deserializes_command_and_args() {
        let params = toml::from_str::<ProcessSamplerParams>(
            r#"
command = ["python", "-u", "worker.py"]
continuous_dims = 2
requires_training_values = true
args = { module = "my_module", class = "MySampler", seed = 0 }
"#,
        )
        .expect("process sampler config should parse");

        assert_eq!(params.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.continuous_dims, 2);
        assert!(params.requires_training_values);
        assert!(params.args.is_object());
    }

    #[test]
    fn process_sampler_domain_accepts_fixed_rectangular_mixed_dims() {
        assert_eq!(
            validate_homogeneous_domain(&Domain::rectangular(3, 2), 3).expect("valid domain"),
            vec![1, 1]
        );
    }

    #[test]
    fn process_sampler_domain_rejects_continuous_dim_mismatch() {
        let err = validate_homogeneous_domain(&Domain::rectangular(3, 1), 2)
            .expect_err("expected mismatch");
        assert!(err.to_string().contains("continuous_dims=2"));
    }
}
