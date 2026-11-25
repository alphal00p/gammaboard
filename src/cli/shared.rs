use anyhow::{Context, Result, anyhow};
use gammaboard::{PgStore, config::RuntimeConfig, init_pg_store, tracing::init_tracing};
use std::future::Future;
use tracing::Instrument;
pub fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string(value).expect("serializable output")
    );
}
pub fn init_cli_tracing(store: &PgStore, config: &RuntimeConfig, quiet: bool) -> Result<()> {
    let runtime_log_store = if config.tracing.persist_runtime_logs {
        Some(store.clone())
    } else {
        None
    };
    init_tracing(runtime_log_store, &config.tracing, quiet)
        .map_err(|err| anyhow!(err.to_string()))?;
    Ok(())
}

pub async fn init_cli_store(
    config: &RuntimeConfig,
    db_pool_size: u32,
    quiet: bool,
) -> Result<PgStore> {
    let store = init_pg_store(&config.database.url, db_pool_size)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store, config, quiet)?;
    Ok(store)
}

pub async fn with_cli_store<T, F, Fut>(
    config: &RuntimeConfig,
    db_pool_size: u32,
    quiet: bool,
    span: tracing::Span,
    f: F,
) -> Result<T>
where
    F: FnOnce(PgStore) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let store = init_cli_store(config, db_pool_size, quiet).await?;
    async move { f(store).await }.instrument(span).await
}
