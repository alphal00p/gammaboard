use anyhow::Context;
use serde::Deserialize;
use std::{fs, path::Path};

pub const DEFAULT_CLI_CONFIG_PATH: &str = "configs/cli/default.toml";
pub const DEFAULT_SERVER_CONFIG_PATH: &str = "configs/server/default.toml";

#[derive(Debug, Clone, Deserialize)]
pub struct CliConfig {
    pub database: DatabaseConfig,
    pub tracing: TracingConfig,
    pub local_postgres: LocalPostgresConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TracingConfig {
    pub persist_runtime_logs: bool,
    pub db_gammaboard_level: String,
    pub db_external_level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalPostgresConfig {
    pub data_dir: String,
    pub socket_dir: String,
    pub log_file: String,
}

impl CliConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed reading cli config {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("failed parsing cli config {}", path.display()))
    }
}
