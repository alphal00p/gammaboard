mod cli;

use anyhow::Result;
use clap::{Parser, error::ErrorKind};
use cli::Cli;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let json_requested = std::env::args_os().any(|argument| argument == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if json_requested
                && !matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                )
            {
                eprintln!(
                    "{}",
                    cli::format_error_json("invalid_arguments", error.to_string())
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(error.exit_code() as u8);
        }
    };
    let json_output = cli.json_output_requested();
    let result = match activate_symbolica_oem_license() {
        Ok(()) => cli::dispatch(cli).await,
        Err(error) => Err(error),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json_output {
                eprintln!("{}", cli::format_error_json("command_failed", error));
            } else {
                eprintln!("Error: {error}");
            }
            ExitCode::FAILURE
        }
    }
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
