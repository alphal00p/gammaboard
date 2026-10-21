use serde_json::Value;

use crate::core::EngineError;
use crate::evaluation::{Batch, BatchTransform};
use crate::process_runtime::{
    build_process_worker_command, decode_process_batch, expect_process_ack,
};
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker};
use crate::utils::domain::Domain;

use crate::ProcessConfig;

pub struct ProcessBatchTransform {
    worker: ProcessBatchTransformWorker,
}

impl ProcessBatchTransform {
    pub fn from_params(params: ProcessConfig, domain: &Domain) -> Result<Self, EngineError> {
        params.validate()?;
        Ok(Self {
            worker: ProcessBatchTransformWorker::spawn(&params, domain.clone())?,
        })
    }
}

impl BatchTransform for ProcessBatchTransform {
    fn apply(&mut self, batch: Batch) -> Result<Batch, EngineError> {
        self.worker.transform_batch(&batch)
    }
}

struct ProcessBatchTransformWorker {
    process: ProcessWorker,
    domain: Domain,
}

impl ProcessBatchTransformWorker {
    fn spawn(params: &ProcessConfig, domain: Domain) -> Result<Self, EngineError> {
        let mut command = build_process_worker_command(
            &params.command,
            params.cwd.as_deref(),
            "batch_transform",
        )?;
        let process = ProcessWorker::spawn(
            &mut command,
            "process batch transform",
            params.shutdown_grace_seconds,
        )?;
        let mut worker = Self { process, domain };
        worker.send_init(params.args.clone())?;
        Ok(worker)
    }

    fn send_init(&mut self, args: Value) -> Result<(), EngineError> {
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
            .map_err(EngineError::build)?;
        expect_process_ack(response, "batch transform").map_err(EngineError::build)
    }

    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError> {
        let inputs = row_major_batch(batch);
        let response = self
            .process
            .request(
                "transform_batch",
                serde_json::json!({
                    "nr_samples": batch.len(),
                    "xs_discrete_row_major": inputs.xs_discrete_row_major,
                    "xs_discrete_offsets": inputs.xs_discrete_offsets,
                    "xs_continuous_row_major": inputs.xs_continuous_row_major,
                    "xs_continuous_offsets": inputs.xs_continuous_offsets,
                    "weights": inputs.weights,
                }),
            )
            .map_err(EngineError::engine)?;
        decode_process_batch(&response, batch.len(), &self.domain, "batch transform")
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
    let mut xs_discrete_offsets = Vec::with_capacity(batch.len() + 1);
    let mut xs_continuous_row_major = Vec::new();
    let mut xs_continuous_offsets = Vec::with_capacity(batch.len() + 1);
    let mut weights = Vec::with_capacity(batch.len());
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

#[cfg(test)]
mod tests {
    use super::{ProcessBatchTransform, ProcessConfig};
    use crate::evaluation::{Batch, BatchTransform, Point};
    use crate::utils::domain::Domain;
    use serde_json::json;

    #[test]
    fn process_batch_transform_transforms_batch() {
        let python =
            std::env::var("GAMMABOARD_TEST_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let params = ProcessConfig {
            shutdown_grace_seconds: 30,
            command: vec![
                python,
                "-u".to_string(),
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/json_worker.py").to_string(),
                "transform".to_string(),
            ],
            cwd: None,
            args: json!({}),
        };
        let mut transform =
            ProcessBatchTransform::from_params(params, &Domain::continuous(2)).expect("transform");
        let batch = Batch::from_iter([
            Point::new(vec![0.25, 0.5], Vec::new(), 1.0),
            Point::new(vec![0.75, 1.0], Vec::new(), 2.0),
        ]);

        let transformed = transform.apply(batch).expect("transform batch");

        assert_eq!(transformed.points()[0].continuous, vec![0.5, 1.0]);
        assert_eq!(transformed.points()[1].continuous, vec![1.5, 2.0]);
        assert_eq!(transformed.weights(), vec![3.0, 6.0]);
    }
}
