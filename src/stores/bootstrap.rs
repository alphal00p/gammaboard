use super::PgStore;
use dotenvy::dotenv;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use std::env;
use tokio::time::{Duration, sleep};

/// Create a PostgreSQL connection pool.
///
/// Loads DATABASE_URL from environment (via .env file) and creates
/// a connection pool with the specified maximum number of connections.
pub async fn get_pg_pool(max_connections: u32) -> Result<Pool<Postgres>, sqlx::Error> {
    dotenv().ok();
    let db_url =
        env::var("DATABASE_URL").map_err(|err| sqlx::Error::Configuration(Box::new(err)))?;
    // Retry transient DB startup races (e.g. connection resets while Postgres is coming up).
    const MAX_ATTEMPTS: u32 = 20;
    const BASE_DELAY_MS: u64 = 150;
    const MAX_DELAY_MS: u64 = 2_000;

    let mut attempt = 1;
    loop {
        match PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(&db_url)
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
pub async fn init_pg_store(max_connections: u32) -> Result<PgStore, sqlx::Error> {
    let pool = get_pg_pool(max_connections).await?;
    Ok(PgStore::new(pool))
}
