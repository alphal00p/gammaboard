mod cli;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    activate_symbolica_oem_license()?;
    let cli = Cli::parse();
    cli::dispatch(cli).await
}

fn activate_symbolica_oem_license() -> Result<()> {
    if option_env!("NO_SYMBOLICA_OEM_LICENSE").is_some() {
        return Ok(());
    }

    std::panic::catch_unwind(|| {
        symbolica::activate_oem_license!("SYMBOLICA_OEM_KEY_7facf394");
    })
    .map_err(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown Symbolica OEM activation panic");
        anyhow::anyhow!(
            "failed to activate the bundled GammaBoard Symbolica OEM license: {message}. \
             To use a regular Symbolica license instead, rebuild with \
             NO_SYMBOLICA_OEM_LICENSE=1 and set SYMBOLICA_LICENSE at runtime."
        )
    })
}
