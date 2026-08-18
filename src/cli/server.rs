use clap::Args;
use gammaboard::config::DEFAULT_SERVER_CONFIG_PATH;
use gammaboard::runtime_context::{RuntimeContext, checked_add_port};
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
    runtime: &RuntimeContext,
    quiet: bool,
) -> anyhow::Result<()> {
    let mut config = ServerConfig::load(&args.server_config)?;
    if let Some(api_port) = args.api_port {
        config.api_port = api_port;
    } else {
        config.api_port = checked_add_port(config.api_port, runtime.port_offset(), "API port")?;
    }
    config.allowed_origins.extend(args.allowed_origins);
    for warning in config.security_warnings() {
        eprintln!("WARNING: {warning}");
    }
    let bind = config.bind_addr();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "server",
        source = "server",
        bind = %bind
    );
    with_cli_store(
        runtime.runtime_config(),
        SERVER_DB_POOL_SIZE,
        quiet,
        span,
        |store| async move { serve(store, config, runtime.clone()).await },
    )
    .await
}
