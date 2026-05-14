use anyhow::Context;
use serde::Deserialize;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const DEFAULT_RUNTIME_CONFIG_PATH: &str = "ops/local/config/runtime.toml";
pub const DEFAULT_SERVER_CONFIG_PATH: &str = "ops/local/config/server.toml";
pub const DEFAULT_DEPLOY_CONFIG_PATH: &str = "ops/local/config/deploy.toml";
const DEFAULT_RUNTIME_CONFIG_TOML: &str = include_str!("config_defaults/runtime.toml");
const DEFAULT_DEPLOY_CONFIG_TOML: &str = include_str!("../ops/local/config/deploy.toml");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub database: DatabaseConfig,
    pub tracing: TracingConfig,
    #[serde(default)]
    pub resources: ResourceConfig,
    pub local_postgres: LocalPostgresConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    pub persist_runtime_logs: bool,
    pub db_gammaboard_level: String,
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
    pub data_dir: String,
    pub socket_dir: String,
    pub log_file: String,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployConfig {
    pub api_server: DeployApiServerConfig,
    pub static_site: DeployStaticSiteConfig,
    pub frontend_http: DeployFrontendHttpConfig,
    #[serde(default)]
    pub database: DeployDatabaseConfig,
    #[serde(default)]
    pub cleanup: DeployCleanupConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployApiServerConfig {
    pub api_server_config: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployStaticSiteConfig {
    pub frontend_build_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployFrontendHttpConfig {
    pub frontend_host: String,
    pub frontend_port: u16,
    pub frontend_server_name: String,
    #[serde(default)]
    pub frontend_advertise_hosts: Vec<String>,
    #[serde(default)]
    pub access_log: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployDatabaseConfig {
    #[serde(default = "default_database_ensure_started")]
    pub ensure_started: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployCleanupConfig {
    #[serde(default = "default_sampler_drain_timeout_seconds")]
    pub sampler_drain_timeout_seconds: u64,
    #[serde(default = "default_node_stop_timeout_seconds")]
    pub node_stop_timeout_seconds: u64,
    #[serde(default = "default_cleanup_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

impl Default for DeployDatabaseConfig {
    fn default() -> Self {
        Self {
            ensure_started: default_database_ensure_started(),
        }
    }
}

fn default_database_ensure_started() -> bool {
    true
}

impl Default for DeployCleanupConfig {
    fn default() -> Self {
        Self {
            sampler_drain_timeout_seconds: default_sampler_drain_timeout_seconds(),
            node_stop_timeout_seconds: default_node_stop_timeout_seconds(),
            poll_interval_ms: default_cleanup_poll_interval_ms(),
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

impl DeployConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = read_toml_with_default_fallback(
            path,
            DEFAULT_DEPLOY_CONFIG_PATH,
            DEFAULT_DEPLOY_CONFIG_TOML,
            "deploy config",
        )?;
        let mut parsed: Self = toml::from_str(&raw)
            .with_context(|| format!("failed parsing deploy config {}", path.display()))?;
        let base_dir = config_base_dir(path)?;
        parsed.api_server.api_server_config =
            normalize_config_path(&base_dir, &parsed.api_server.api_server_config)
                .display()
                .to_string();
        parsed.static_site.frontend_build_dir =
            normalize_config_path(&base_dir, &parsed.static_site.frontend_build_dir)
                .display()
                .to_string();
        Ok(parsed)
    }

    pub fn advertised_urls(&self, port: u16) -> Vec<String> {
        let hosts = if self.frontend_http.frontend_advertise_hosts.is_empty() {
            vec![default_advertise_host(
                &self.frontend_http.frontend_host,
                &self.frontend_http.frontend_server_name,
            )]
        } else {
            self.frontend_http.frontend_advertise_hosts.clone()
        };
        hosts
            .into_iter()
            .map(|host| format!("http://{host}:{port}"))
            .collect()
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

fn default_advertise_host(host: &str, server_name: &str) -> String {
    if !server_name.trim().is_empty() && server_name != "_" {
        server_name.to_string()
    } else if host == "0.0.0.0" {
        "localhost".to_string()
    } else {
        host.to_string()
    }
}

fn default_shared_buffers() -> String {
    "4GB".to_string()
}

fn default_listen_addresses() -> String {
    "localhost".to_string()
}

fn default_host_auth_cidr() -> String {
    "127.0.0.1/32".to_string()
}

fn default_effective_cache_size() -> String {
    "32GB".to_string()
}

fn default_work_mem() -> String {
    "64MB".to_string()
}

fn default_checkpoint_timeout() -> String {
    "30min".to_string()
}

fn default_sampler_drain_timeout_seconds() -> u64 {
    60
}

fn default_node_stop_timeout_seconds() -> u64 {
    15
}

fn default_cleanup_poll_interval_ms() -> u64 {
    250
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
