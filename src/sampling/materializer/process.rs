use serde_json::Value;

use crate::core::EngineError;
use crate::evaluation::{Batch, Materializer};
use crate::process_runtime::{
    build_process_worker_command, decode_process_batch, expect_process_ack,
};
use crate::process_worker::{PROCESS_PROTOCOL, ProcessWorker};
use crate::sampling::LatentBatch;
use crate::utils::domain::Domain;

use crate::ProcessConfig;

pub struct ProcessMaterializer {
    domain: Domain,
    worker: ProcessMaterializerWorker,
}

impl ProcessMaterializer {
    pub fn from_params_and_domain(
        params: ProcessConfig,
        domain: &Domain,
    ) -> Result<Self, EngineError> {
        params.validate()?;
        let worker = ProcessMaterializerWorker::spawn(&params, domain.clone())?;
        Ok(Self {
            domain: domain.clone(),
            worker,
        })
    }
}

impl Materializer for ProcessMaterializer {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        if domain != &self.domain {
            return Err(EngineError::build(format!(
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

struct ProcessMaterializerWorker {
    process: ProcessWorker,
    domain: Domain,
}

impl ProcessMaterializerWorker {
    fn spawn(params: &ProcessConfig, domain: Domain) -> Result<Self, EngineError> {
        let mut command =
            build_process_worker_command(&params.command, params.cwd.as_deref(), "materializer")?;
        let process = ProcessWorker::spawn(
            &mut command,
            "process materializer",
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
                    "role": "materializer",
                    "domain": self.domain,
                    "args": args,
                }),
            )
            .map_err(EngineError::build)?;
        expect_process_ack(response, "materializer").map_err(EngineError::build)
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
        decode_process_batch(
            &response,
            latent_batch.nr_samples,
            &self.domain,
            "materializer",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessConfig, ProcessMaterializer};
    use crate::evaluation::{Batch, Materializer, Point};
    use crate::sampling::LatentBatch;
    use crate::utils::domain::Domain;
    use serde_json::json;

    #[test]
    fn process_materializer_materializes_latent_batch() {
        let python =
            std::env::var("GAMMABOARD_TEST_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let params = ProcessConfig {
            shutdown_grace_seconds: 30,
            command: vec![
                python,
                "-u".to_string(),
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/json_worker.py").to_string(),
                "materializer".to_string(),
            ],
            cwd: None,
            args: json!({}),
        };
        let mut materializer =
            ProcessMaterializer::from_params_and_domain(params, &Domain::continuous(2))
                .expect("process materializer should initialize");
        let batch = Batch::from_iter([
            Point::new(vec![0.25, 0.5], Vec::new(), 1.0),
            Point::new(vec![0.75, 1.0], Vec::new(), 2.0),
        ]);
        let latent = LatentBatch::from_batch(&batch);

        let materialized = materializer
            .materialize_batch(&latent)
            .expect("materialize batch");

        assert_eq!(materialized.points()[0].continuous, vec![0.5, 1.0]);
        assert_eq!(materialized.points()[1].continuous, vec![1.5, 2.0]);
        assert_eq!(materialized.weights(), vec![1.0, 2.0]);
    }
}
