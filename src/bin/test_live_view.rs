use gammaboard::{get_pg_pool, Batch, BatchResults, WeightedPoint};
use rand::Rng;
use serde_json::json;
use sqlx::{PgPool, Row};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting live view test with batch-based schema...");

    // Connect to database
    let pool = get_pg_pool(5).await?;

    // Create a test run
    let run_id = create_test_run(&pool).await?;
    println!("✅ Created test run with ID: {}", run_id);

    // Spawn background task to simulate worker processing batches
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        if let Err(e) = worker_loop(pool_clone, run_id).await {
            eprintln!("❌ Worker error: {}", e);
        }
    });

    // Main loop: sampler generates batches continuously
    loop {
        // Generate a batch of samples
        let batch_size = rand::thread_rng().gen_range(5..20);
        generate_batch(&pool, run_id, batch_size).await?;
        println!("📝 Generated batch with {} samples", batch_size);

        // Wait a bit before next batch (simulating adaptive sampler thinking)
        let delay = rand::thread_rng().gen_range(500..2000);
        sleep(Duration::from_millis(delay)).await;

        // Periodically cleanup old completed batches
        if rand::thread_rng().gen_bool(0.3) {
            cleanup_completed_batches(&pool, run_id).await?;
        }
    }
}

async fn create_test_run(pool: &PgPool) -> Result<i32, sqlx::Error> {
    let result = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO runs (integration_params, status)
        VALUES ($1, 'running')
        RETURNING id
        "#,
    )
    .bind(json!({
        "expression": "sin(x)*exp(-x^2)",
        "bounds": [0.0, 10.0],
        "target_error": 0.001
    }))
    .fetch_one(pool)
    .await?;

    Ok(result)
}

async fn generate_batch(pool: &PgPool, run_id: i32, size: usize) -> Result<(), sqlx::Error> {
    let mut rng = rand::thread_rng();

    // Generate random points with importance weights
    let mut weighted_points = Vec::new();
    for _ in 0..size {
        // Random point in [0, 10]
        let x: f64 = rng.r#gen();
        let x = x * 10.0;

        // Random importance weight (adaptive sampler would compute this intelligently)
        let weight: f64 = rng.r#gen();
        let weight = 0.5 + weight; // weight in [0.5, 1.5]

        weighted_points.push(WeightedPoint::new(json!(x), weight));
    }

    let batch = Batch::new(weighted_points);
    let batch_json = batch.to_json();

    // Insert batch into work queue
    sqlx::query(
        r#"
        INSERT INTO batches (run_id, points, batch_size, status)
        VALUES ($1, $2, $3, 'pending')
        "#,
    )
    .bind(run_id)
    .bind(batch_json)
    .bind(size as i32)
    .execute(pool)
    .await?;

    Ok(())
}

async fn worker_loop(pool: PgPool, run_id: i32) -> Result<(), sqlx::Error> {
    println!("🔄 Started worker simulation");

    loop {
        // Try to claim a batch (simulating worker claiming work)
        let claimed = claim_batch(&pool, run_id, "worker-test").await?;

        if let Some((batch_id, batch)) = claimed {
            println!("⚙️  Worker claimed batch {} with {} samples", batch_id, batch.size());

            // Simulate evaluation time
            sleep(Duration::from_millis(100)).await;

            // Evaluate the batch (simulate computation)
            let mut results = Vec::new();
            for point in &batch.points {
                // Extract x value and compute sin(x)*exp(-x^2)
                let x: f64 = if let Some(x_val) = point.point.as_f64() {
                    x_val
                } else {
                    0.0 // fallback
                };
                let value = x.sin() * (-x * x).exp();
                results.push(value);
            }

            let batch_results = BatchResults::new(results);
            let eval_time = 100.0 * batch.size() as f64; // Simulate ~100ms per sample

            // Submit results
            submit_batch_results(&pool, batch_id, batch_results, eval_time).await?;
            println!("✅ Worker completed batch {}", batch_id);
        } else {
            // No work available, wait a bit
            sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn claim_batch(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<Option<(i64, Batch)>, sqlx::Error> {
    // Atomically claim a batch using FOR UPDATE SKIP LOCKED
    let row = sqlx::query(
        r#"
        UPDATE batches
        SET status = 'claimed',
            claimed_by = $1,
            claimed_at = now()
        WHERE id IN (
            SELECT id FROM batches
            WHERE run_id = $2
              AND status = 'pending'
            ORDER BY created_at
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING id, points
        "#,
    )
    .bind(worker_id)
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = row {
        let batch_id: i64 = row.get("id");
        let points_json: serde_json::Value = row.get("points");
        let batch = Batch::from_json(&points_json)
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Some((batch_id, batch)))
    } else {
        Ok(None)
    }
}

async fn submit_batch_results(
    pool: &PgPool,
    batch_id: i64,
    results: BatchResults,
    eval_time_ms: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE batches
        SET status = 'completed',
            results = $1,
            total_eval_time_ms = $2,
            completed_at = now()
        WHERE id = $3
        "#,
    )
    .bind(results.to_json())
    .bind(eval_time_ms)
    .bind(batch_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn cleanup_completed_batches(pool: &PgPool, run_id: i32) -> Result<(), sqlx::Error> {
    // Keep only the most recent 50 completed batches
    let deleted = sqlx::query(
        r#"
        DELETE FROM batches
        WHERE id IN (
            SELECT id FROM batches
            WHERE run_id = $1 AND status = 'completed'
            ORDER BY completed_at DESC
            OFFSET 50
        )
        "#,
    )
    .bind(run_id)
    .execute(pool)
    .await?;

    if deleted.rows_affected() > 0 {
        println!(
            "🧹 Cleaned up {} old batches to keep table lean",
            deleted.rows_affected()
        );
    }

    Ok(())
}
