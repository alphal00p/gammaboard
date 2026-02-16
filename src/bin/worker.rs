use gammaboard::{BatchResults, claim_batch, get_pg_pool, submit_batch_results};
use std::{env, time::Duration};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let run_id = env::var("WORKER_RUN_ID")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(1);
    let worker_id = env::var("WORKER_ID").unwrap_or_else(|_| "worker-1".to_string());

    println!("🚀 Starting worker {worker_id} for run {run_id}");
    let pool = get_pg_pool(5).await?;

    loop {
        let claimed = claim_batch(&pool, run_id, &worker_id).await?;
        if let Some((batch_id, batch)) = claimed {
            let mut values = Vec::with_capacity(batch.size());
            for point in &batch.points {
                let x = point.point.as_f64().unwrap_or(0.0);
                values.push(x.sin() * (-x * x).exp());
            }

            let results = BatchResults::new(values);
            let eval_time_ms = 100.0 * batch.size() as f64;
            submit_batch_results(&pool, batch_id, &results, eval_time_ms).await?;
            println!("✅ {worker_id} completed batch {batch_id}");
        } else {
            sleep(Duration::from_millis(250)).await;
        }
    }
}
