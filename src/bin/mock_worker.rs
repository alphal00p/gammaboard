//! Mock evaluator worker binary.
//!
//! This binary uses hardcoded constants and the generic `WorkerRunner`.

use gammaboard::{
    AssignmentLeaseStore, ComponentInstance, ComponentRegistryStore, ComponentRole, EvalError,
    Evaluator, InstanceStatus, PgStore, WorkerRunner, WorkerRunnerConfig, get_pg_pool,
};
use serde_json::Value as JsonValue;
use serde_json::json;
use std::time::Duration;

const RUN_ID: i32 = 1;
const INSTANCE_ID: &str = "mock-worker-1";
const LOOP_SLEEP_MS: u64 = 200;
const MIN_EVAL_TIME_PER_SAMPLE_MS: u64 = 2;

struct MockEvaluator;

impl Evaluator for MockEvaluator {
    fn eval_point(&self, point: &JsonValue) -> Result<f64, EvalError> {
        let x = point
            .as_f64()
            .ok_or_else(|| EvalError::new("expected f64 point"))?;
        Ok(x.sin() * (-x * x).exp())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "🛠️ Starting mock worker: run_id={} instance_id={}",
        RUN_ID, INSTANCE_ID
    );

    let pool = get_pg_pool(5).await?;
    let store = PgStore::new(pool);

    store
        .register_instance(&ComponentInstance {
            instance_id: INSTANCE_ID.to_string(),
            node_id: None,
            role: ComponentRole::Evaluator,
            implementation: "mock_evaluator".to_string(),
            version: "v1".to_string(),
            node_specs: json!({ "mock": true }),
            status: InstanceStatus::Active,
            last_seen: None,
        })
        .await?;
    store.assign_evaluator(RUN_ID, INSTANCE_ID).await?;

    let mut runner = WorkerRunner::new(
        RUN_ID,
        INSTANCE_ID,
        MockEvaluator,
        store.clone(),
        WorkerRunnerConfig {
            min_eval_time_per_sample: Duration::from_millis(MIN_EVAL_TIME_PER_SAMPLE_MS),
        },
    );

    loop {
        store.heartbeat_instance(INSTANCE_ID).await?;

        match runner.tick().await {
            Ok(tick) => {
                if let Some(batch_id) = tick.claimed_batch_id {
                    println!(
                        "✅ completed batch={} samples={} eval_time_ms={:.2}",
                        batch_id, tick.processed_samples, tick.eval_time_ms
                    );
                }
            }
            Err(err) => {
                eprintln!("❌ mock worker tick failed: {}", err);
            }
        }

        tokio::time::sleep(Duration::from_millis(LOOP_SLEEP_MS)).await;
    }
}
