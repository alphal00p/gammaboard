//! Generate the default native physics state with GammaBoard's pinned GammaLoop.
#[cfg(feature = "gammaloop")]
fn main() -> anyhow::Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "resources/states/showcase/gg_hhh".into());
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
            card.write_all(include_str!("gammaloop/showcase.toml").as_bytes())?;
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
#[cfg(not(feature = "gammaloop"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("prepare_showcase requires the gammaloop feature")
}
