use crate::api::ApiError;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct RestartDbResult {
    pub deleted: bool,
    pub started: bool,
}

/// Restarts the local database by invoking existing CLI lifecycle commands.
pub fn restart_local_database(
    binary_path: &Path,
    cli_config_path: &Path,
) -> Result<RestartDbResult, ApiError> {
    run_db_command(binary_path, cli_config_path, "delete", &["--yes"])?;
    run_db_command(binary_path, cli_config_path, "start", &[])?;
    Ok(RestartDbResult {
        deleted: true,
        started: true,
    })
}

fn run_db_command(
    binary_path: &Path,
    cli_config_path: &Path,
    command: &str,
    extra_args: &[&str],
) -> Result<(), ApiError> {
    let status = Command::new(binary_path)
        .arg("--cli-config")
        .arg(cli_config_path)
        .arg("db")
        .arg(command)
        .args(extra_args)
        .status()
        .map_err(|err| ApiError::Internal(format!("failed to run db {command}: {err}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(ApiError::Internal(format!(
            "db {command} failed with status {status}"
        )))
    }
}
