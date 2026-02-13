use gammaboard::get_pg_pool;
use rand::Rng;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting live view test...");

    // Connect to database
    let pool = get_pg_pool(5).await?;

    // Create a test run
    let run_id = create_test_run(&pool).await?;
    println!("✅ Created test run with ID: {}", run_id);

    // Spawn background task to simulate sample processing
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        if let Err(e) = process_samples_loop(pool_clone, run_id).await {
            eprintln!("❌ Processing error: {}", e);
        }
    });

    // Main loop: generate samples continuously
    loop {
        // Generate batch of samples
        let batch_size = rand::thread_rng().gen_range(5..20);
        generate_sample_batch(&pool, run_id, batch_size).await?;
        println!("📝 Generated {} samples", batch_size);

        // Wait a bit before next batch
        let delay = rand::thread_rng().gen_range(500..2000);
        sleep(Duration::from_millis(delay)).await;

        // Periodically cleanup old completed samples
        if rand::thread_rng().gen_bool(0.2) {
            cleanup_old_samples(&pool, run_id).await?;
        }
    }
}

async fn create_test_run(pool: &PgPool) -> Result<i32, sqlx::Error> {
    let result = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO runs (parameters)
        VALUES ('{"expression": "sin(x)*exp(-x^2)", "bounds": [0, 10]}'::jsonb)
        RETURNING id
        "#
    )
    .fetch_one(pool)
    .await?;

    Ok(result)
}

async fn generate_sample_batch(
    pool: &PgPool,
    run_id: i32,
    count: i32,
) -> Result<(), sqlx::Error> {
    let mut rng = rand::thread_rng();

    // Generate random steps and values
    let mut steps = Vec::new();
    let mut values: Vec<f64> = Vec::new();

    for _ in 0..count {
        steps.push(rng.gen_range(0..10000));
        // Simulate some computation result
        let x: f64 = rng.r#gen();
        let x = x * 10.0;
        values.push(x.sin() * (-x * x).exp());
    }

    // Batch insert using UNNEST
    sqlx::query(
        r#"
        INSERT INTO results (run_id, step, value)
        SELECT $1, * FROM UNNEST($2::int[], $3::double precision[])
        "#
    )
    .bind(run_id)
    .bind(&steps)
    .bind(&values)
    .execute(pool)
    .await?;

    Ok(())
}

async fn process_samples_loop(pool: PgPool, run_id: i32) -> Result<(), sqlx::Error> {
    println!("🔄 Started background processing task");

    loop {
        // Simulate processing some samples (mark as processed by updating them)
        let processed = sqlx::query_scalar::<_, i64>(
            r#"
            DELETE FROM results
            WHERE id IN (
                SELECT id FROM results
                WHERE run_id = $1
                ORDER BY created_at
                LIMIT 5
            )
            RETURNING id
            "#
        )
        .bind(run_id)
        .fetch_all(&pool)
        .await?;

        if !processed.is_empty() {
            println!("🗑️  Cleaned up {} old samples", processed.len());
        }

        // Wait before next processing batch
        sleep(Duration::from_millis(1500)).await;
    }
}

async fn cleanup_old_samples(pool: &PgPool, run_id: i32) -> Result<(), sqlx::Error> {
    // Keep only the most recent 1000 samples
    let deleted = sqlx::query(
        r#"
        DELETE FROM results
        WHERE id IN (
            SELECT id FROM results
            WHERE run_id = $1
            ORDER BY created_at DESC
            OFFSET 1000
        )
        "#
    )
    .bind(run_id)
    .execute(pool)
    .await?;

    if deleted.rows_affected() > 0 {
        println!("🧹 Cleaned up {} old samples to keep table lean", deleted.rows_affected());
    }

    Ok(())
}
