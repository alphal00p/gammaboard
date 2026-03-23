use super::PgStore;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use tokio::time::{Duration, sleep};

/// Create a PostgreSQL connection pool.
pub async fn get_pg_pool(
    database_url: &str,
    max_connections: u32,
) -> Result<Pool<Postgres>, sqlx::Error> {
    // Retry transient DB startup races (e.g. connection resets while Postgres is coming up).
    const MAX_ATTEMPTS: u32 = 20;
    const BASE_DELAY_MS: u64 = 150;
    const MAX_DELAY_MS: u64 = 2_000;

    let mut attempt = 1;
    loop {
        match PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(err) if attempt < MAX_ATTEMPTS => {
                let backoff_ms = (BASE_DELAY_MS
                    .saturating_mul(1u64 << (attempt.saturating_sub(1))))
                .min(MAX_DELAY_MS);
                eprintln!(
                    "database connect attempt {attempt}/{MAX_ATTEMPTS} failed: {err}; retrying in {backoff_ms}ms"
                );
                sleep(Duration::from_millis(backoff_ms)).await;
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Initialize a Postgres-backed store with the given max pool size.
pub async fn init_pg_store(
    database_url: &str,
    max_connections: u32,
) -> Result<PgStore, sqlx::Error> {
    let pool = get_pg_pool(database_url, max_connections).await?;
    Ok(PgStore::new(pool))
}
