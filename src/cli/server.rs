use clap::Args;
use gammaboard::server::{resolve_bind, serve};
use std::net::SocketAddr;

use super::shared::with_cli_store;

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long)]
    bind: Option<SocketAddr>,
    #[arg(long, default_value_t = 10)]
    db_pool_size: u32,
}

pub async fn run_server(args: ServerArgs, quiet: bool) -> anyhow::Result<()> {
    let bind = resolve_bind(args.bind)?;
    let span = tracing::span!(
        tracing::Level::TRACE,
        "server",
        source = "server",
        bind = %bind
    );
    with_cli_store(args.db_pool_size, quiet, span, |store| async move {
        serve(store, bind).await
    })
    .await
}
