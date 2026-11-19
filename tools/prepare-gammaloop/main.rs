//! Generate the default native physics state with GammaBoard's pinned GammaLoop.
use clap::Parser;

#[derive(Parser)]
#[command(about = "Generate the physics state for the GammaLoop dashboard example")]
struct Args {
    /// Fresh output directory; existing directories are never overwritten.
    #[arg(default_value = "resources/states/showcase/gg_hhh")]
    output: std::path::PathBuf,
}

fn main() -> anyhow::Result<()> {
    let output = Args::parse().output;
    anyhow::ensure!(
        !output.exists(),
        "{} already exists; choose a fresh output directory",
        output.display()
    );
    gammaboard::activate_symbolica_oem_license()?;
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || -> anyhow::Result<()> {
            use gammaloop_api::{StateLoadOption, state::CommandHistory};
            use std::io::Write;
            let mut card = tempfile::Builder::new().suffix(".toml").tempfile()?;
            card.write_all(include_str!("generation.toml").as_bytes())?;
            let mut loaded = StateLoadOption {
                state_folder: Some(output.clone()),
                boot_commands_path: Some(card.path().to_owned()),
                ..Default::default()
            }
            .load()
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
            let command = CommandHistory::from_raw_string("run generate")
                .map_err(|e| anyhow::anyhow!("{e:#}"))?;
            loaded
                .cli_session()
                .execute_command(command)
                .map_err(|e| anyhow::anyhow!("{e:#}"))?;
            anyhow::ensure!(
                output.join("state_manifest.toml").is_file(),
                "GammaLoop did not save the state"
            );
            println!("Prepared native GammaLoop showcase at {}", output.display());
            Ok(())
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("GammaLoop generation thread panicked"))?
}
