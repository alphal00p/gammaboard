use gammaboard::{EvalError, Evaluator, PgStore, WorkerRunner, WorkerRunnerConfig, get_pg_pool};
use serde_json::Value as JsonValue;
use std::{env, time::Duration};
use tokio::time::sleep;

struct DefaultEvaluator;

impl Evaluator for DefaultEvaluator {
    fn eval_point(&self, point: &JsonValue) -> Result<f64, EvalError> {
        let x = point
            .as_f64()
            .ok_or_else(|| EvalError::new("expected f64 point"))?;
        Ok(x.sin() * (-x * x).exp())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let run_id = env::var("WORKER_RUN_ID")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(1);
    let worker_id = env::var("WORKER_ID").unwrap_or_else(|_| "worker-1".to_string());
    let loop_sleep_ms = env::var("WORKER_LOOP_SLEEP_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(250);
    let min_eval_time_per_sample_ms = env::var("WORKER_MIN_EVAL_TIME_PER_SAMPLE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(100);

    println!("🚀 Starting worker {worker_id} for run {run_id}");
    let pool = get_pg_pool(5).await?;
    let store = PgStore::new(pool);
    let mut runner = WorkerRunner::new(
        run_id,
        worker_id.clone(),
        DefaultEvaluator,
        store,
        WorkerRunnerConfig {
            min_eval_time_per_sample: Duration::from_millis(min_eval_time_per_sample_ms),
        },
    );

    loop {
        match runner.tick().await {
            Ok(tick) => {
                if let Some(batch_id) = tick.claimed_batch_id {
                    println!("✅ {worker_id} completed batch {batch_id}");
                }
            }
            Err(err) => {
                eprintln!("❌ {worker_id} tick failed: {err}");
            }
        }
        sleep(Duration::from_millis(loop_sleep_ms)).await;
    }
}
