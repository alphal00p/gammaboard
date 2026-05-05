mod cli;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    if option_env!("NO_SYMBOLICA_OEM_LICENSE").is_none() {
        symbolica::activate_oem_license!("SYMBOLICA_OEM_KEY_7facf394");
    };
    let cli = Cli::parse();
    cli::dispatch(cli).await
}
