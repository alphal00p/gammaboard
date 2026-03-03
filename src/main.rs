mod cli;

use clap::Parser;
use cli::Cli;
use gammaboard::BinResult;

#[tokio::main]
async fn main() -> BinResult {
    let cli = Cli::parse();
    cli::dispatch(cli).await
}
