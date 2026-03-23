use clap::Args;
use gammaboard::config::{CliConfig, DEFAULT_SERVER_CONFIG_PATH};
use gammaboard::server::{ServerConfig, serve};
use std::path::PathBuf;

use super::shared::with_cli_store;

const SERVER_DB_POOL_SIZE: u32 = 10;

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long = "server-config", default_value = DEFAULT_SERVER_CONFIG_PATH, value_name = "PATH")]
    server_config: PathBuf,
}

pub async fn run_server(
    args: ServerArgs,
    cli_config: &CliConfig,
    quiet: bool,
) -> anyhow::Result<()> {
    let config = ServerConfig::load(&args.server_config)?;
    let bind = config.bind_addr();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "server",
        source = "server",
        bind = %bind
    );
    with_cli_store(
        cli_config,
        SERVER_DB_POOL_SIZE,
        quiet,
        span,
        |store| async move { serve(store, config).await },
    )
    .await
}
