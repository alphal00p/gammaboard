use anyhow::Context;
use clap::Args;
use gammaboard::init_pg_store;
use gammaboard::server::{resolve_bind, serve};
use std::net::SocketAddr;

use super::shared::init_cli_tracing;

#[derive(Debug, Args)]
pub struct ServerArgs {
    #[arg(long)]
    bind: Option<SocketAddr>,
    #[arg(long, default_value_t = 10)]
    db_pool_size: u32,
}

pub async fn run_server(args: ServerArgs, quiet: bool) -> anyhow::Result<()> {
    let store = init_pg_store(args.db_pool_size)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store, quiet)?;
    let bind = resolve_bind(args.bind)?;
    serve(store, bind).await
}
