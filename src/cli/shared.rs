use anyhow::{Context, Result, anyhow};
use clap::{Args, ValueEnum};
use gammaboard::core::WorkerRole;
use gammaboard::tracing::init_tracing;
use gammaboard::{PgStore, init_pg_store};
use std::future::Future;
use tracing::Instrument;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RoleArg {
    Evaluator,
    SamplerAggregator,
}

impl From<RoleArg> for WorkerRole {
    fn from(value: RoleArg) -> Self {
        match value {
            RoleArg::Evaluator => WorkerRole::Evaluator,
            RoleArg::SamplerAggregator => WorkerRole::SamplerAggregator,
        }
    }
}

#[derive(Debug, Args)]
pub struct RunSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "run_ids")]
    pub all: bool,
    #[arg(value_name = "RUN_ID", required_unless_present = "all")]
    pub run_ids: Vec<i32>,
}

#[derive(Debug, Args)]
pub struct NodeSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "node_ids")]
    pub all: bool,
    #[arg(value_name = "NODE_ID", required_unless_present = "all")]
    pub node_ids: Vec<String>,
}

fn env_true(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let value = value.trim();
            value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
        })
        .unwrap_or(false)
}

pub fn init_cli_tracing(store: &PgStore, quiet: bool) -> Result<()> {
    let runtime_log_store = if env_true("GAMMABOARD_DISABLE_DB_LOGS") {
        None
    } else {
        Some(store.clone())
    };
    init_tracing(runtime_log_store, quiet).map_err(|err| anyhow!(err.to_string()))?;
    Ok(())
}

pub async fn init_cli_store(db_pool_size: u32, quiet: bool) -> Result<PgStore> {
    let store = init_pg_store(db_pool_size)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store, quiet)?;
    Ok(store)
}

pub async fn with_cli_store<T, F, Fut>(
    db_pool_size: u32,
    quiet: bool,
    span: tracing::Span,
    f: F,
) -> Result<T>
where
    F: FnOnce(PgStore) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let store = init_cli_store(db_pool_size, quiet).await?;
    async move { f(store).await }.instrument(span).await
}

pub fn control_command_span(name: &'static str) -> tracing::Span {
    tracing::span!(
        tracing::Level::TRACE,
        "control_command",
        source = "control",
        command = name
    )
}

pub async fn with_control_store<T, F, Fut>(
    db_pool_size: u32,
    quiet: bool,
    command_name: &'static str,
    f: F,
) -> Result<T>
where
    F: FnOnce(PgStore) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    with_cli_store(db_pool_size, quiet, control_command_span(command_name), f).await
}
