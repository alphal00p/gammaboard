//! Service executable: local bootstrap, offline maintenance, and worker execution.
//! Ordinary management commands are handled by the lightweight client crate.
pub mod auth;
pub mod benchmark;
pub mod db;
pub mod instance;
pub mod service;
use service as local;
pub mod management;
pub mod node;
pub mod shared;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gammaboard::runtime_context::{RuntimeContext, RuntimeOverrides};
use std::path::PathBuf;
#[derive(Debug, Parser)]
#[command(
    name = "gammaboard",
    version,
    about = "GammaBoard service and worker executable; use gb for management"
)]
pub struct Cli {
    #[arg(long, global = true)]
    instance: Option<PathBuf>,
    #[arg(long, global = true)]
    allow_local_processes: bool,
    #[arg(long, global = true, default_value = "runtime.toml")]
    runtime_config: PathBuf,
    #[arg(long, global = true)]
    database_url: Option<String>,
    #[arg(long = "resource-root", global = true)]
    resource_roots: Vec<String>,
    #[arg(long, global = true, default_value_t = 0)]
    port_offset: u16,
    #[arg(long, global = true)]
    postgres_trusted_network: bool,
    #[arg(long, short = 'q', global = true)]
    quiet: bool,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}
impl Cli {
    pub fn json_output_requested(&self) -> bool {
        self.json
    }
}
pub fn format_error_json(code: &str, message: impl std::fmt::Display) -> String {
    serde_json::json!({"error":{"code":code,"message":message.to_string()}}).to_string()
}
#[derive(Debug, Subcommand)]
enum Command {
    Start(local::StartArgs),
    Restart(gammaboard_client::cli::StartupOptions),
    Db(db::DbArgs),
    #[command(visible_alias = "worker")]
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    Auth(auth::AuthArgs),
    Benchmark(benchmark::BenchmarkArgs),
    BuildInfo,
    /// Validate deployment configuration without starting services.
    CheckConfig,
    /// Validate or probe a run on this host.
    CheckRun {
        file: PathBuf,
        #[arg(long)]
        probe: bool,
    },
}
#[derive(Debug, Subcommand)]
enum NodeCommand {
    Run(node::NodeRunArgs),
}
pub fn dispatch(cli: Cli) -> Result<()> {
    let worker = matches!(cli.command, Command::Node { .. });
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(if worker { 1 } else { 4 })
        .enable_all()
        .build()?
        .block_on(dispatch_async(cli))
}
async fn dispatch_async(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Auth(args) => return auth::run_auth_command(args),
        Command::BuildInfo => {
            shared::print_json(&gammaboard::provenance::build_info());
            return Ok(());
        }

        _ => {}
    }
    let legacy_runtime =
        cli.runtime_config != std::path::Path::new("runtime.toml") || cli.database_url.is_some();
    let selected = cli.instance.or_else(|| {
        (!legacy_runtime && PathBuf::from("gammaboard.toml").is_file())
            .then(|| PathBuf::from("gammaboard.toml"))
    });
    let instance = selected
        .as_deref()
        .map(instance::Instance::load)
        .transpose()?;
    match cli.command {
        Command::Start(args) => {
            return local::start(
                instance
                    .as_ref()
                    .context("start requires --instance FILE")?,
                args,
                cli.allow_local_processes,
            )
            .await;
        }
        Command::Restart(mut args) => {
            let instance = instance
                .as_ref()
                .context("restart requires --instance FILE")?;
            instance
                .require_local_processes(cli.allow_local_processes, "restart local services")?;
            let started = std::time::Instant::now();
            local::stop(
                instance,
                local::StopArgs {
                    wait: false,
                    timeout: Some(args.timeout),
                },
            )
            .await?;
            args.timeout = args
                .timeout
                .checked_sub(started.elapsed())
                .context("restart deadline elapsed after shutdown; instance was not restarted")?;
            return local::start(
                instance,
                local::StartArgs {
                    component: None,
                    options: args,
                },
                cli.allow_local_processes,
            )
            .await;
        }
        Command::CheckConfig => {
            shared::print_json(
                &instance
                    .as_ref()
                    .context("check-config requires --instance FILE")?
                    .redacted()?,
            );
            Ok(())
        }
        Command::CheckRun { file, probe } => {
            let snapshot = tempfile::tempdir()?;
            if probe {
                let instance = instance
                    .as_ref()
                    .context("check-run --probe requires --instance FILE")?;
                instance.require_local_processes(
                    cli.allow_local_processes,
                    "probe a run on this host",
                )?;
                instance.write_snapshot(snapshot.path())?;
                RuntimeContext::load(
                    snapshot.path().join("runtime.toml"),
                    RuntimeOverrides::default(),
                )?;
            }
            let definition = gammaboard::api::runs::load_run_add_config_file(&file)?;
            shared::print_json(&gammaboard::api::runs::validate_run(definition, probe)?);
            Ok(())
        }
        Command::Db(args) if instance.is_some() => {
            let instance = instance.as_ref().unwrap();
            if instance.managed_database
                && !matches!(
                    args.command,
                    db::DbCommand::Status
                        | db::DbCommand::Stop
                        | db::DbCommand::Backup { .. }
                        | db::DbCommand::Purge { .. }
                )
            {
                instance.require_local_processes(
                    cli.allow_local_processes,
                    "perform local database maintenance",
                )?;
            }
            return management::database(instance, args, cli.allow_local_processes).await;
        }
        command => {
            let snapshot = tempfile::tempdir()?;
            let path = if let Some(instance) = instance {
                instance.write_snapshot(snapshot.path())?;
                snapshot.path().join("runtime.toml")
            } else {
                cli.runtime_config
            };
            let runtime = RuntimeContext::load(
                path,
                RuntimeOverrides {
                    database_url: cli.database_url,
                    resource_roots: cli.resource_roots,
                    port_offset: cli.port_offset,
                    postgres_trusted_network: cli.postgres_trusted_network,
                },
            )?
            .with_local_processes(cli.allow_local_processes);
            match command {
                Command::Node {
                    command: NodeCommand::Run(args),
                } => {
                    runtime.require_local_processes("run a local worker")?;
                    node::run_node(args, runtime.runtime_config(), cli.quiet).await
                }
                Command::Benchmark(args) => {
                    runtime.require_local_processes("run a local benchmark")?;
                    benchmark::run(args, runtime.runtime_config()).await
                }
                Command::Db(args) => db::run_db_command(args, runtime.runtime_config()),
                _ => unreachable!(),
            }
        }
    }
}
