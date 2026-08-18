use anyhow::{Context, Result, anyhow};
use clap::{Args, ValueEnum};
use gammaboard::config::RuntimeConfig;
use gammaboard::core::{RunReadStore, WorkerRole};
use gammaboard::stores::RunProgress;
use gammaboard::tracing::init_tracing;
use gammaboard::{PgStore, init_pg_store};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::Instrument;

static JSON_OUTPUT: AtomicBool = AtomicBool::new(false);

pub fn set_json_output(enabled: bool) {
    JSON_OUTPUT.store(enabled, Ordering::Relaxed);
}

pub fn json_output_enabled() -> bool {
    JSON_OUTPUT.load(Ordering::Relaxed)
}

pub fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string(value).expect("CLI output must be JSON serializable")
    );
}

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
    #[arg(short = 'a', long = "all", conflicts_with = "run_refs")]
    pub all: bool,
    #[arg(value_name = "RUN", required_unless_present = "all")]
    pub run_refs: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunRemovalSelection {
    #[command(flatten)]
    pub runs: RunSelection,
    #[arg(long, help = "Confirm destructive run removal in non-interactive use")]
    pub yes: bool,
    #[arg(long, help = "Show selected runs without changing them")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct NodeSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "node_names")]
    pub all: bool,
    #[arg(value_name = "NODE_NAME", required_unless_present = "all")]
    pub node_names: Vec<String>,
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

pub fn control_command_span(name: &'static str) -> tracing::Span {
    tracing::span!(
        tracing::Level::TRACE,
        "control_command",
        source = "control",
        command = name
    )
}

pub async fn with_control_store<T, F, Fut>(
    config: &RuntimeConfig,
    db_pool_size: u32,
    quiet: bool,
    command_name: &'static str,
    f: F,
) -> Result<T>
where
    F: FnOnce(PgStore) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    with_cli_store(
        config,
        db_pool_size,
        quiet,
        control_command_span(command_name),
        f,
    )
    .await
}

pub async fn resolve_run_ref(store: &impl RunReadStore, run_ref: &str) -> Result<RunProgress> {
    if let Ok(run_id) = run_ref.parse::<i32>() {
        if let Some(run) = store.get_run_progress(run_id).await? {
            return Ok(run);
        }
    }

    let matches = store.get_runs_by_name(run_ref).await?;

    match matches.as_slice() {
        [] => Err(anyhow!("run '{run_ref}' not found")),
        [run] => Ok(run.clone()),
        many => Err(anyhow!(format_ambiguous_runs(run_ref, many))),
    }
}

pub async fn resolve_run_selection(
    store: &impl RunReadStore,
    selection: RunSelection,
) -> Result<Vec<RunProgress>> {
    if selection.all {
        return store.get_all_runs().await.map_err(Into::into);
    }

    let mut resolved = Vec::with_capacity(selection.run_refs.len());
    for run_ref in selection.run_refs {
        resolved.push(resolve_run_ref(store, &run_ref).await?);
    }
    Ok(resolved)
}

pub async fn list_runs_by_name(
    store: &impl RunReadStore,
    run_name: &str,
) -> Result<Vec<RunProgress>> {
    store.get_runs_by_name(run_name).await.map_err(Into::into)
}

fn format_ambiguous_runs(run_ref: &str, runs: &[RunProgress]) -> String {
    let mut message =
        format!("run name '{run_ref}' matches multiple runs. Use the numeric id instead:\n");
    for run in runs {
        let line = format!(
            "  id={} name={} state={}",
            run.run_id, run.run_name, run.lifecycle_state
        );
        message.push_str(&line);
        message.push('\n');
    }
    message.trim_end().to_string()
}
