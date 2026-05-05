use clap::Args;
use gammaboard::config::{DEFAULT_SERVER_CONFIG_PATH, RuntimeConfig};
use gammaboard::server::{ServerConfig, serve};
use std::path::PathBuf;

use super::shared::with_cli_store;

const SERVER_DB_POOL_SIZE: u32 = 8;

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long = "server-config", default_value = DEFAULT_SERVER_CONFIG_PATH, value_name = "PATH")]
    server_config: PathBuf,
    #[arg(long = "api-port")]
    api_port: Option<u16>,
    #[arg(long = "allowed-origin", value_name = "ORIGIN")]
    allowed_origins: Vec<String>,
}

pub async fn run_server(
    args: ServerArgs,
    runtime_config: &RuntimeConfig,
    runtime_config_path: &std::path::Path,
    quiet: bool,
) -> anyhow::Result<()> {
    let mut config = ServerConfig::load(&args.server_config)?;
    if let Some(api_port) = args.api_port {
        config.api_port = api_port;
    }
    config.allowed_origins.extend(args.allowed_origins);
    let bind = config.bind_addr();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "server",
        source = "server",
        bind = %bind
    );
    with_cli_store(
        runtime_config,
        SERVER_DB_POOL_SIZE,
        quiet,
        span,
        |store| async move {
            serve(
                store,
                config,
                runtime_config_path.to_path_buf(),
                runtime_config.clone(),
            )
            .await
        },
    )
    .await
}
