use anyhow::Context;
use serde::Deserialize;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const DEFAULT_RUNTIME_CONFIG_PATH: &str = "ops/local/config/runtime.toml";
pub const DEFAULT_SERVER_CONFIG_PATH: &str = "ops/local/config/server.toml";
const DEFAULT_RUNTIME_CONFIG_TOML: &str = include_str!("config_defaults/runtime.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub resources: ResourceConfig,
    #[serde(default)]
    pub local_postgres: LocalPostgresConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(default = "default_database_url")]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    #[serde(default = "default_persist_runtime_logs")]
    pub persist_runtime_logs: bool,
    #[serde(default = "default_db_gammaboard_level")]
    pub db_gammaboard_level: String,
    #[serde(default = "default_db_external_level")]
    pub db_external_level: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ResourceConfig {
    #[serde(default)]
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPostgresConfig {
    #[serde(default = "default_postgres_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_postgres_socket_dir")]
    pub socket_dir: String,
    #[serde(default = "default_postgres_log_file")]
    pub log_file: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_listen_addresses")]
    pub listen_addresses: String,
    #[serde(default = "default_host_auth_cidr")]
    pub host_auth_cidr: String,
    #[serde(default = "default_shared_buffers")]
    pub shared_buffers: String,
    #[serde(default = "default_effective_cache_size")]
    pub effective_cache_size: String,
    #[serde(default = "default_work_mem")]
    pub work_mem: String,
    #[serde(default = "default_checkpoint_timeout")]
    pub checkpoint_timeout: String,
    #[serde(default = "default_max_wal_size")]
    pub max_wal_size: String,
    #[serde(default = "default_wal_compression")]
    pub wal_compression: bool,
    #[serde(default = "default_synchronous_commit")]
    pub synchronous_commit: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_database_url(),
        }
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            persist_runtime_logs: default_persist_runtime_logs(),
            db_gammaboard_level: default_db_gammaboard_level(),
            db_external_level: default_db_external_level(),
        }
    }
}

impl Default for LocalPostgresConfig {
    fn default() -> Self {
        Self {
            data_dir: default_postgres_data_dir(),
            socket_dir: default_postgres_socket_dir(),
            log_file: default_postgres_log_file(),
            max_connections: default_max_connections(),
            listen_addresses: default_listen_addresses(),
            host_auth_cidr: default_host_auth_cidr(),
            shared_buffers: default_shared_buffers(),
            effective_cache_size: default_effective_cache_size(),
            work_mem: default_work_mem(),
            checkpoint_timeout: default_checkpoint_timeout(),
            max_wal_size: default_max_wal_size(),
            wal_compression: default_wal_compression(),
            synchronous_commit: default_synchronous_commit(),
        }
    }
}

impl RuntimeConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = read_toml_with_default_fallback(
            path,
            DEFAULT_RUNTIME_CONFIG_PATH,
            DEFAULT_RUNTIME_CONFIG_TOML,
            "runtime config",
        )?;
        toml::from_str(&raw)
            .with_context(|| format!("failed parsing runtime config {}", path.display()))
    }

    pub fn primary_resource_root(&self) -> PathBuf {
        primary_resource_root_from_config(&self.resources.roots)
    }
}

pub fn normalize_config_path(base_dir: &Path, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path.trim());
    if candidate.is_absolute() {
        candidate
    } else {
        base_dir.join(candidate)
    }
}

pub fn config_base_dir(path: &Path) -> anyhow::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        return Ok(parent.to_path_buf());
    }
    std::env::current_dir().context("failed resolving current working directory")
}

pub fn read_toml_with_default_fallback(
    path: &Path,
    default_path: &str,
    default_toml: &str,
    label: &str,
) -> anyhow::Result<String> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(raw),
        Err(err) if err.kind() == ErrorKind::NotFound && path == Path::new(default_path) => {
            Ok(default_toml.to_string())
        }
        Err(err) => Err(err).with_context(|| format!("failed reading {label} {}", path.display())),
    }
}

fn default_database_url() -> String {
    "postgresql://postgres:NqVj2yt5WsCE5nYCOx01MkeFD8n8awoZ@127.0.0.1:5400/gammaboard_db"
        .to_string()
}

fn default_persist_runtime_logs() -> bool {
    true
}

fn default_db_gammaboard_level() -> String {
    "info".to_string()
}

fn default_db_external_level() -> String {
    "warn".to_string()
}

fn default_postgres_data_dir() -> String {
    "db/postgres".to_string()
}

fn default_postgres_socket_dir() -> String {
    "db/socket".to_string()
}

fn default_postgres_log_file() -> String {
    "db/logfile".to_string()
}

fn default_max_connections() -> u32 {
    128
}

fn default_shared_buffers() -> String {
    "256MB".to_string()
}

fn default_listen_addresses() -> String {
    "localhost".to_string()
}

fn default_host_auth_cidr() -> String {
    "127.0.0.1/32".to_string()
}

fn default_effective_cache_size() -> String {
    "2GB".to_string()
}

fn default_work_mem() -> String {
    "16MB".to_string()
}

fn default_checkpoint_timeout() -> String {
    "30min".to_string()
}

fn default_max_wal_size() -> String {
    "8GB".to_string()
}

fn default_wal_compression() -> bool {
    true
}

fn default_synchronous_commit() -> bool {
    false
}

pub fn primary_resource_root_from_config(roots: &[String]) -> PathBuf {
    roots
        .iter()
        .map(|root| root.trim())
        .find(|root| !root.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("resources"))
}

pub fn normalize_local_postgres_paths(local_postgres: &mut LocalPostgresConfig, roots: &[String]) {
    let resource_root = primary_resource_root_from_config(roots);
    local_postgres.data_dir = normalize_config_path(&resource_root, &local_postgres.data_dir)
        .display()
        .to_string();
    local_postgres.socket_dir = normalize_config_path(&resource_root, &local_postgres.socket_dir)
        .display()
        .to_string();
    local_postgres.log_file = normalize_config_path(&resource_root, &local_postgres.log_file)
        .display()
        .to_string();
}
