use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{AccumulatorConfig, EngineError};
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, GammaLoopAccumulatorState,
};
use crate::process_runtime::{build_process_worker_command, default_process_args};
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker};
use crate::utils::domain::Domain;

/// The callback's output contract, independent of the task's storage policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessOutput {
    Values {
        #[serde(default = "default_components")]
        components: Vec<String>,
    },
    Gammaloop,
}
impl Default for ProcessOutput {
    fn default() -> Self {
        Self::Values {
            components: default_components(),
        }
    }
}
impl ProcessOutput {
    fn components(&self) -> &[String] {
        match self {
            Self::Values { components } => components,
            Self::Gammaloop => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProcessEvaluatorParams {
    #[serde(flatten)]
    pub process: crate::ProcessConfig,
    pub domain: Domain,
    #[serde(default)]
    pub output: ProcessOutput,
}

pub struct ProcessEvaluator {
    worker: ProcessRuntimeWorker,
}

impl ProcessEvaluator {
    pub fn from_params(params: ProcessEvaluatorParams) -> Result<Self, EngineError> {
        params.process.validate()?;
        if let ProcessOutput::Values { components } = &params.output {
            AccumulatorConfig::vector(components.clone(), crate::TrainingProjection::Norm)
                .validate()
                .map_err(EngineError::build)?;
        }
        Ok(Self {
            worker: ProcessRuntimeWorker::spawn(&params)?,
        })
    }
}

impl Evaluator for ProcessEvaluator {
    fn domain(&self) -> Domain {
        self.worker.domain.clone()
    }

    fn metadata(&self) -> Value {
        self.worker.metadata.clone()
    }

    fn diagnostics(&mut self) -> Value {
        match self
            .worker
            .process
            .request("diagnostics", serde_json::json!({}))
        {
            Ok(response) => response.get("diagnostics").cloned().unwrap_or_else(
                || serde_json::json!({"error": "process evaluator response missing 'diagnostics'"}),
            ),
            Err(error) => serde_json::json!({"error": error}),
        }
    }

    fn eval_values(
        &mut self,
        batch: &Batch,
        components: &[String],
    ) -> Result<Vec<f64>, EngineError> {
        if !matches!(&self.worker.output, ProcessOutput::Values { .. })
            || self.worker.output.components() != components
        {
            return Err(EngineError::invalid_input(
                "stratification requires matching per-sample evaluator values",
            ));
        }
        self.worker
            .eval_batch(&pack_eval_inputs(batch, &self.worker.domain), batch.len())
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EngineError> {
        let mut inputs = pack_eval_inputs(batch, &self.worker.domain);
        if matches!(accumulator, AccumulatorConfig::Gammaloop) {
            for point in batch.points() {
                inputs
                    .binary
                    .extend_from_slice(&point.total_weight().to_le_bytes());
            }
            let response = self
                .worker
                .eval_batch_gammaloop(&inputs, batch.len(), options)?;
            return crate::evaluation::stratification::finish_gammaloop(
                batch,
                response.values.as_deref(),
                response.state,
                response.training_values,
                options,
            );
        }
        let values = self.worker.eval_batch(&inputs, batch.len())?;
        crate::evaluation::stratification::accumulate_values(
            batch,
            self.worker.output.components(),
            &values,
            accumulator,
            options,
        )
    }
}

struct GammaLoopResponse {
    state: GammaLoopAccumulatorState,
    training_values: Option<Vec<f64>>,
    values: Option<Vec<f64>>,
}

struct PackedEvalInputs {
    binary: Vec<u8>,
    xs_discrete_offsets: Option<Vec<usize>>,
    xs_continuous_offsets: Option<Vec<usize>>,
}

/// Write coordinates directly into the wire block, discrete values first.
/// Fixed-width dimensions need no offsets; ragged dimensions retain them.
fn pack_eval_inputs(batch: &Batch, domain: &Domain) -> PackedEvalInputs {
    let elements: usize = batch
        .points()
        .iter()
        .map(|point| point.discrete.len() + point.continuous.len())
        .sum();
    let mut binary = Vec::with_capacity(elements * 8);
    let mut discrete_offsets = domain
        .fixed_discrete_depth()
        .is_none()
        .then(|| Vec::with_capacity(batch.len() + 1));
    let mut continuous_offsets = domain
        .fixed_continuous_dims()
        .is_none()
        .then(|| Vec::with_capacity(batch.len() + 1));
    if let Some(offsets) = &mut discrete_offsets {
        offsets.push(0);
    }
    for point in batch.points() {
        for value in &point.discrete {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(offsets) = &mut discrete_offsets {
            offsets.push(binary.len() / 8);
        }
    }
    let continuous_start = binary.len();
    if let Some(offsets) = &mut continuous_offsets {
        offsets.push(0);
    }
    for point in batch.points() {
        for value in &point.continuous {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(offsets) = &mut continuous_offsets {
            offsets.push((binary.len() - continuous_start) / 8);
        }
    }
    PackedEvalInputs {
        binary,
        xs_discrete_offsets: discrete_offsets,
        xs_continuous_offsets: continuous_offsets,
    }
}

/// Decode a little-endian `f64` binary block into a `Vec<f64>`.
fn unpack_f64_le(bytes: &[u8], what: &str) -> Result<Vec<f64>, EngineError> {
    if !bytes.len().is_multiple_of(8) {
        return Err(EngineError::eval(format!(
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

fn add_required_offsets(params: &mut Value, inputs: &PackedEvalInputs) {
    if let Some(offsets) = &inputs.xs_discrete_offsets {
        params["xs_discrete_offsets"] = serde_json::json!(offsets);
    }
    if let Some(offsets) = &inputs.xs_continuous_offsets {
        params["xs_continuous_offsets"] = serde_json::json!(offsets);
    }
}

struct ProcessRuntimeWorker {
    process: ProcessWorker,
    domain: Domain,
    output: ProcessOutput,
    metadata: Value,
}

impl ProcessRuntimeWorker {
    fn spawn(params: &ProcessEvaluatorParams) -> Result<Self, EngineError> {
        let mut command = build_process_worker_command(
            &params.process.command,
            params.process.cwd.as_deref(),
            "evaluator",
        )?;
        let process = ProcessWorker::spawn(
            &mut command,
            "process evaluator",
            params.process.shutdown_grace_seconds,
        )?;
        let mut worker = Self {
            process,
            domain: params.domain.clone(),
            output: params.output.clone(),
            metadata: default_process_args(),
        };
        worker.send_init(params.process.args.clone())?;
        Ok(worker)
    }

    fn send_init(&mut self, args: Value) -> Result<(), EngineError> {
        let init_params = serde_json::json!({
            "protocol": PROCESS_PROTOCOL, "role": "evaluator", "output": self.output,
            "domain": self.domain, "args": args,
        });
        let response = self
            .process
            .request("initialize", init_params)
            .map_err(EngineError::build)?;
        self.metadata = Self::expect_ack(response).map_err(EngineError::build)?;
        Ok(())
    }

    fn eval_batch(
        &mut self,
        inputs: &PackedEvalInputs,
        nr_samples: usize,
    ) -> Result<Vec<f64>, EngineError> {
        let mut params = serde_json::json!({
            "nr_samples": nr_samples,
            "components": self.output.components(),
        });
        add_required_offsets(&mut params, inputs);
        let (_result, response_binary) = self
            .process
            .request_with_binary("eval_batch", params, &inputs.binary)
            .map_err(EngineError::eval)?;
        let values = unpack_f64_le(&response_binary, "values")?;
        let expected_len = nr_samples.saturating_mul(self.output.components().len());
        if values.len() != expected_len {
            return Err(EngineError::eval(format!(
                "process evaluator output size mismatch: expected {}, got {}",
                expected_len,
                values.len()
            )));
        }
        Ok(values)
    }

    fn eval_batch_gammaloop(
        &mut self,
        inputs: &PackedEvalInputs,
        nr_samples: usize,
        options: EvalBatchOptions<'_>,
    ) -> Result<GammaLoopResponse, EngineError> {
        let require_training_values = options.require_training_values;
        options.validate(nr_samples, &["real".into(), "imag".into()])?;
        let mut params = serde_json::json!({
            "nr_samples": nr_samples,
            "require_training_values": require_training_values,
            "stratification": options.stratification.map(|(plan,labels)| serde_json::json!({"partition":plan.partition,"labels":labels})),
        });
        add_required_offsets(&mut params, inputs);
        let (response, response_binary) = self
            .process
            .request_with_binary("eval_batch", params, &inputs.binary)
            .map_err(EngineError::eval)?;
        let state: GammaLoopAccumulatorState = serde_json::from_value(
            response
                .get("gammaloop_state")
                .ok_or_else(|| {
                    EngineError::eval("process worker response missing 'gammaloop_state'")
                })?
                .clone(),
        )
        .map_err(|err| EngineError::eval(format!("failed to parse gammaloop_state: {err}")))?;
        let decoded = unpack_f64_le(&response_binary, "GammaLoop values")?;
        let feedback_len = usize::from(require_training_values) * nr_samples;
        let values_len = usize::from(options.stratification.is_some()) * nr_samples * 2;
        if decoded.len() != feedback_len + values_len {
            return Err(EngineError::eval(
                "GammaLoop response length does not match requested feedback and real/imag values",
            ));
        }
        let training_values = require_training_values.then(|| decoded[..feedback_len].to_vec());
        let values = options
            .stratification
            .is_some()
            .then(|| decoded[feedback_len..].to_vec());
        state.validate_values()?;
        if let Some(values) = &training_values
            && let Some(sample) = values.iter().position(|value| !value.is_finite())
        {
            return Err(EngineError::eval(format!(
                "sample {sample}: non-finite training feedback"
            )));
        }
        Ok(GammaLoopResponse {
            state,
            training_values,
            values,
        })
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
    use super::{ProcessEvaluatorParams, ProcessRuntimeWorker};
    use crate::utils::domain::Domain;
    use crate::{Batch, Point};
    use serde_json::json;

    #[test]
    fn eval_input_pack_and_value_unpack_roundtrip() {
        let batch = Batch::new(vec![
            Point::new(vec![0.5, 1.5], vec![0, 1], 1.0),
            Point::new(vec![2.5, 3.5], vec![2, 3], 1.0),
        ]);
        let inputs = super::pack_eval_inputs(&batch, &Domain::rectangular(2, 2));
        assert!(inputs.xs_discrete_offsets.is_none());
        assert!(inputs.xs_continuous_offsets.is_none());
        let packed = inputs.binary;
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
        let batch = Batch::new(vec![
            Point::new(vec![0.5], vec![0], 1.0),
            Point::new(vec![1.5, 2.5], vec![1, 1], 1.0),
        ]);
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
        let inputs = super::pack_eval_inputs(&batch, &ragged_domain);
        super::add_required_offsets(&mut ragged_params, &inputs);
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

        assert_eq!(params.process.command, ["python", "-u", "worker.py"]);
        assert_eq!(params.process.cwd.as_deref(), Some("$resources"));
        assert_eq!(params.domain.fixed_rectangular_dims(), Some((2, 2)));
        assert_eq!(
            params.domain.fixed_discrete_cardinalities(),
            Some(vec![2, 3])
        );
        assert_eq!(params.output.components(), ["value"]);
        assert!(params.process.args.is_object());
    }

    #[test]
    fn process_evaluator_deserializes_inhomogeneous_domain() {
        let params = toml::from_str::<ProcessEvaluatorParams>(
            r#"
command = ["worker"]
domain = { discrete = { axis_label = "d0", branches = [
  { index = 0, domain = { rectangular = { discrete_cardinalities = [], continuous_dims = 3 } } },
  { index = 1, domain = { discrete = { axis_label = "d1", branches = [
    { index = 0, domain = { rectangular = { discrete_cardinalities = [], continuous_dims = 1 } } },
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
