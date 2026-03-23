use clap::Args;
use gammaboard::server::{ServerConfig, serve};
use std::path::PathBuf;

use super::shared::with_cli_store;

const SERVER_DB_POOL_SIZE: u32 = 10;

#[derive(Debug, Args)]
pub struct ServerArgs {
    config_path: PathBuf,
}

pub async fn run_server(args: ServerArgs, quiet: bool) -> anyhow::Result<()> {
    let config = ServerConfig::load(&args.config_path)?;
    let bind = config.bind_addr();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "server",
        source = "server",
        bind = %bind
    );
    with_cli_store(SERVER_DB_POOL_SIZE, quiet, span, |store| async move {
        serve(store, config).await
    })
    .await
}
