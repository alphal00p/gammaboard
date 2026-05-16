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

    let Some(oem_license_name) = option_env!("SYMBOLICA_OEM_LICENSE") else {
        anyhow::bail!(
            "GammaBoard was built without a Symbolica OEM license name. \
             Rebuild with SYMBOLICA_OEM_LICENSE=SYMBOLICA_OEM_GAMMALOOP, \
             or set NO_SYMBOLICA_OEM_LICENSE=1 at build time to use a regular Symbolica runtime license."
        );
    };
    if !oem_license_name.starts_with("SYMBOLICA_OEM_") {
        anyhow::bail!(
            "invalid compile-time SYMBOLICA_OEM_LICENSE={oem_license_name:?}; expected a value starting with SYMBOLICA_OEM_"
        );
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
            "failed to activate Symbolica OEM license {oem_license_name:?}: {message}. \
             Ensure the binary was built with the matching SYMBOLICA_OEM_LICENSE value."
        )
    })
}
