use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator, GammaLoopAccumulatorState,
};
use crate::process_runtime::{build_process_worker_command, default_process_args};
use crate::process_worker::{
    PROCESS_PROTOCOL, ProcessWorker, default_process_shutdown_grace_seconds,
};
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessAccumulatorKind {
    #[default]
    Vector,
    FullVector,
    Gammaloop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessEvaluatorParams {
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub domain: Domain,
    #[serde(default = "default_components")]
    pub components: Vec<String>,
    #[serde(default)]
    pub accumulator: ProcessAccumulatorKind,
    #[serde(default = "default_process_args")]
    pub args: Value,
    #[serde(default = "default_process_shutdown_grace_seconds")]
    pub shutdown_grace_seconds: u64,
}

pub struct ProcessEvaluator {
    domain: Domain,
    components: Vec<String>,
    metadata: Value,
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
        if !matches!(params.accumulator, ProcessAccumulatorKind::Gammaloop)
            && params.components.is_empty()
        {
            return Err(BuildError::build(
                "process_evaluator components must not be empty",
            ));
        }
        let domain = params.domain.clone();
        let worker = ProcessRuntimeWorker::spawn(&params, domain.clone())?;
        Ok(Self {
            domain,
            components: params.components,
            metadata: worker.metadata.clone(),
            worker,
        })
    }
}

impl Evaluator for ProcessEvaluator {
    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn metadata(&self) -> Value {
        self.metadata.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let inputs = ragged_row_major_inputs(batch);

        if let AccumulatorState::Gammaloop(gl_state) = &mut observable_state {
            let (new_state, training_values) =
                self.worker.eval_batch_gammaloop(&inputs, batch.size())?;
            *gl_state = new_state;
            return Ok(BatchResult::new(training_values, observable_state));
        }

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

/// Pack the row-major eval inputs into the request binary block: little-endian
/// `i64` discrete values followed by little-endian `f64` continuous values. The
/// receiver splits them using the offsets carried in the JSON envelope.
fn pack_eval_inputs(inputs: &RaggedRowMajorInputs) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        inputs.xs_discrete_row_major.len() * 8 + inputs.xs_continuous_row_major.len() * 8,
    );
    for value in &inputs.xs_discrete_row_major {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in &inputs.xs_continuous_row_major {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decode a little-endian `f64` binary block into a `Vec<f64>`.
fn unpack_f64_le(bytes: &[u8], what: &str) -> Result<Vec<f64>, EvalError> {
    if !bytes.len().is_multiple_of(8) {
        return Err(EvalError::eval(format!(
            "process worker {what} binary block length {} is not a multiple of 8",
            bytes.len()
        )));
    }
    Ok(bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect())
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

fn add_required_offsets(params: &mut Value, domain: &Domain, inputs: &RaggedRowMajorInputs) {
    let object = params
        .as_object_mut()
        .expect("eval_batch params are constructed as an object");
    if domain.fixed_discrete_depth().is_none() {
        object.insert(
            "xs_discrete_offsets".to_string(),
            serde_json::json!(inputs.xs_discrete_offsets),
        );
    }
    if domain.fixed_continuous_dims().is_none() {
        object.insert(
            "xs_continuous_offsets".to_string(),
            serde_json::json!(inputs.xs_continuous_offsets),
        );
    }
}

struct ProcessRuntimeWorker {
    process: ProcessWorker,
    domain: Domain,
    components: Vec<String>,
    accumulator_kind: ProcessAccumulatorKind,
    metadata: Value,
}

impl ProcessRuntimeWorker {
    fn spawn(params: &ProcessEvaluatorParams, domain: Domain) -> Result<Self, BuildError> {
        let mut command =
            build_process_worker_command(&params.command, params.cwd.as_deref(), "evaluator")?;
        let process = ProcessWorker::spawn(
            &mut command,
            "process evaluator",
            params.shutdown_grace_seconds,
        )?;
        let mut worker = Self {
            process,
            domain,
            components: params.components.clone(),
            accumulator_kind: params.accumulator.clone(),
            metadata: default_process_args(),
        };
        worker.send_init(params.args.clone())?;
        Ok(worker)
    }

    fn send_init(&mut self, args: Value) -> Result<(), BuildError> {
        let init_params = match self.accumulator_kind {
            ProcessAccumulatorKind::Gammaloop => serde_json::json!({
                "protocol": PROCESS_PROTOCOL,
                "role": "evaluator",
                "accumulator": "gammaloop",
                "domain": self.domain,
                "args": args,
            }),
            _ => serde_json::json!({
                "protocol": PROCESS_PROTOCOL,
                "role": "evaluator",
                "components": self.components,
                "observable": {
                    "components": self.components,
                },
                "domain": self.domain,
                "args": args,
            }),
        };
        let response = self
            .process
            .request("initialize", init_params)
            .map_err(BuildError::build)?;
        self.metadata = Self::expect_ack(response).map_err(BuildError::build)?;
        Ok(())
    }

    fn eval_batch(
        &mut self,
        inputs: &RaggedRowMajorInputs,
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError> {
        let mut params = serde_json::json!({
            "nr_samples": nr_samples,
            "components": self.components,
        });
        add_required_offsets(&mut params, &self.domain, inputs);
        let (_result, response_binary) = self
            .process
            .request_with_binary("eval_batch", params, &pack_eval_inputs(inputs))
            .map_err(EvalError::eval)?;
        let values = unpack_f64_le(&response_binary, "values")?;
        let expected_len = nr_samples.saturating_mul(self.components.len());
        if values.len() != expected_len {
            return Err(EvalError::eval(format!(
                "process evaluator output size mismatch: expected {}, got {}",
                expected_len,
                values.len()
            )));
        }
        Ok(values)
    }

    fn eval_batch_gammaloop(
        &mut self,
        inputs: &RaggedRowMajorInputs,
        nr_samples: usize,
    ) -> Result<(GammaLoopAccumulatorState, Option<Vec<f64>>), EvalError> {
        let mut params = serde_json::json!({
            "nr_samples": nr_samples,
            "accumulator": "gammaloop",
        });
        add_required_offsets(&mut params, &self.domain, inputs);
        let (response, _response_binary) = self
            .process
            .request_with_binary("eval_batch", params, &pack_eval_inputs(inputs))
            .map_err(EvalError::eval)?;
        let state: GammaLoopAccumulatorState = serde_json::from_value(
            response
                .get("gammaloop_state")
                .ok_or_else(|| {
                    EvalError::eval("process worker response missing 'gammaloop_state'")
                })?
                .clone(),
        )
        .map_err(|err| EvalError::eval(format!("failed to parse gammaloop_state: {err}")))?;
        let training_values = response
            .get("training_values")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .enumerate()
                    .map(|(index, v)| {
                        v.as_f64().ok_or_else(|| {
                            EvalError::eval(format!("non-f64 in training_values at index {index}"))
                        })
                    })
                    .collect::<Result<Vec<f64>, EvalError>>()
            })
            .transpose()?;
        if let Some(ref tv) = training_values
            && tv.len() != nr_samples
        {
            return Err(EvalError::eval(format!(
                "training_values length {} does not match nr_samples {nr_samples}",
                tv.len()
            )));
        }
        Ok((state, training_values))
    }

    fn expect_ack(response: Value) -> Result<Value, String> {
        if response.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(response
                .get("metadata")
                .cloned()
                .unwrap_or_else(default_process_args));
        }
        Err("process worker initialize result missing ok=true".to_string())
    }
}

fn default_components() -> Vec<String> {
    vec!["value".to_string()]
}

#[cfg(test)]
mod tests {
    use super::{ProcessEvaluatorParams, ProcessRuntimeWorker, RaggedRowMajorInputs};
    use crate::utils::domain::Domain;
    use serde_json::json;

    #[test]
    fn eval_input_pack_and_value_unpack_roundtrip() {
        let inputs = RaggedRowMajorInputs {
            xs_discrete_row_major: vec![0_i64, 1, 2, 3],
            xs_discrete_offsets: vec![0, 2, 4],
            xs_continuous_row_major: vec![0.5_f64, 1.5, 2.5, 3.5],
            xs_continuous_offsets: vec![0, 2, 4],
        };
        let packed = super::pack_eval_inputs(&inputs);
        // 4 i64 + 4 f64 = 64 bytes; discrete block first.
        assert_eq!(packed.len(), 4 * 8 + 4 * 8);
        assert_eq!(&packed[0..8], &0_i64.to_le_bytes());
        assert_eq!(&packed[24..32], &3_i64.to_le_bytes());
        assert_eq!(&packed[32..40], &0.5_f64.to_le_bytes());

        let values = vec![1.0_f64, -2.0, 3.25];
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(super::unpack_f64_le(&bytes, "values").unwrap(), values);
        assert!(super::unpack_f64_le(&[0_u8; 7], "values").is_err());
    }

    #[test]
    fn eval_offsets_are_omitted_only_for_fixed_domain_dimensions() {
        let inputs = RaggedRowMajorInputs {
            xs_discrete_row_major: vec![0, 1, 1],
            xs_discrete_offsets: vec![0, 1, 3],
            xs_continuous_row_major: vec![0.5, 1.5, 2.5],
            xs_continuous_offsets: vec![0, 1, 3],
        };

        let mut fixed_params = json!({});
        super::add_required_offsets(&mut fixed_params, &Domain::rectangular(2, 2), &inputs);
        assert_eq!(fixed_params, json!({}));

        let ragged_domain = Domain::discrete(
            None,
            [
                crate::utils::domain::DomainBranch {
                    index: 0,
                    domain: Box::new(Domain::continuous(1)),
                },
                crate::utils::domain::DomainBranch {
                    index: 1,
                    domain: Box::new(Domain::rectangular(2, 1)),
                },
            ],
        );
        let mut ragged_params = json!({});
        super::add_required_offsets(&mut ragged_params, &ragged_domain, &inputs);
        assert_eq!(
            ragged_params,
            json!({
                "xs_discrete_offsets": [0, 1, 3],
                "xs_continuous_offsets": [0, 1, 3],
            })
        );
    }

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
    fn process_evaluator_initialize_metadata_defaults_to_empty_object() {
        let metadata = ProcessRuntimeWorker::expect_ack(json!({"ok": true}))
            .expect("initialize ack should parse");
        assert_eq!(metadata, json!({}));
    }

    #[test]
    fn process_evaluator_initialize_metadata_roundtrips() {
        let metadata = json!({"space": "momentum", "loop_count": 2});
        let parsed = ProcessRuntimeWorker::expect_ack(json!({
            "ok": true,
            "metadata": metadata,
        }))
        .expect("initialize ack should parse");
        assert_eq!(parsed, metadata);
    }
}
