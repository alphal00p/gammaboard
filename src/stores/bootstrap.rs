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
            .min_connections(0)
            .idle_timeout(Duration::from_secs(30))
            .after_connect(|connection, _| {
                Box::pin(async move {
                    // Bound active queries and abandoned transactions: a suspended
                    // client must not hold row locks indefinitely between statements.
                    // CPU evaluation and process callbacks run outside transactions.
                    sqlx::query("SELECT set_config('statement_timeout','30s',false), set_config('lock_timeout','5s',false), set_config('idle_in_transaction_session_timeout','30s',false)")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
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
    check_database_compatibility(&pool).await?;
    Ok(PgStore::new(pool))
}

/// Check before registering workers or reading persisted runtime models.
pub async fn check_database_compatibility(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    let version = sqlx::query_scalar::<_, i32>(
        "SELECT model_version FROM gammaboard_compatibility WHERE singleton = true",
    ).fetch_optional(pool).await.map_err(|error| {
        match &error {
            sqlx::Error::Database(db) if db.code().as_deref() == Some("42P01") =>
                sqlx::Error::Protocol("incompatible GammaBoard database: missing model version; back up/export with the old executable, then initialize a fresh database (see docs/compatibility.md)".into()),
            _ => error,
        }
    })?;
    if version != Some(crate::compatibility::MODEL_VERSION as i32) {
        return Err(sqlx::Error::Protocol(format!(
            "incompatible GammaBoard database model {version:?}; this executable requires {}; use matching server and workers",
            crate::compatibility::MODEL_VERSION
        )));
    }
    Ok(())
}
