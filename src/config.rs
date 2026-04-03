use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const DEFAULT_RUNTIME_CONFIG_PATH: &str = "configs/runtime/default.toml";
pub const DEFAULT_SERVER_CONFIG_PATH: &str = "configs/server/default.toml";
pub const DEFAULT_DEPLOY_CONFIG_PATH: &str = "configs/deploy/local.toml";

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
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
    pub max_connections: u32,
    pub shared_buffers: String,
    pub effective_cache_size: String,
    pub work_mem: String,
    pub checkpoint_timeout: String,
    pub max_wal_size: String,
    pub wal_compression: bool,
    pub synchronous_commit: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployConfig {
    pub api_server: DeployApiServerConfig,
    pub static_site: DeployStaticSiteConfig,
    pub frontend_http: DeployFrontendHttpConfig,
    pub database: DeployDatabaseConfig,
    pub cleanup: DeployCleanupConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployApiServerConfig {
    pub api_server_config: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployStaticSiteConfig {
    pub frontend_build_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployFrontendHttpConfig {
    pub frontend_host: String,
    pub frontend_port: u16,
    pub frontend_server_name: String,
    #[serde(default)]
    pub frontend_advertise_hosts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployDatabaseConfig {
    pub ensure_started: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeployCleanupConfig {
    pub pause_runs: bool,
    pub stop_nodes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployState {
    pub deploy_config: String,
    pub mode: String,
}

impl RuntimeConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed reading runtime config {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("failed parsing runtime config {}", path.display()))
    }
}

impl DeployConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed reading deploy config {}", path.display()))?;
        let mut parsed: Self = toml::from_str(&raw)
            .with_context(|| format!("failed parsing deploy config {}", path.display()))?;
        let base_dir = path
            .parent()
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")));
        parsed.api_server.api_server_config =
            normalize_config_path(base_dir, &parsed.api_server.api_server_config)
                .display()
                .to_string();
        parsed.static_site.frontend_build_dir =
            normalize_config_path(base_dir, &parsed.static_site.frontend_build_dir)
                .display()
                .to_string();
        Ok(parsed)
    }

    pub fn backend_pid_file(&self) -> PathBuf {
        PathBuf::from("logs/deploy-backend.pid")
    }

    pub fn backend_log_file(&self) -> PathBuf {
        PathBuf::from("logs/deploy-backend.log")
    }

    pub fn nginx_pid_file(&self) -> PathBuf {
        PathBuf::from("logs/nginx-deploy.pid")
    }

    pub fn nginx_error_log(&self) -> PathBuf {
        PathBuf::from("logs/nginx-deploy-error.log")
    }

    pub fn nginx_access_log(&self) -> PathBuf {
        PathBuf::from("logs/nginx-deploy-access.log")
    }

    pub fn nginx_generated_config(&self) -> PathBuf {
        PathBuf::from("tmp/deploy/nginx.conf")
    }

    pub fn deploy_state_file(&self) -> PathBuf {
        PathBuf::from("logs/deploy-state.toml")
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

fn default_advertise_host(host: &str, server_name: &str) -> String {
    if !server_name.trim().is_empty() && server_name != "_" {
        server_name.to_string()
    } else if host == "0.0.0.0" {
        "localhost".to_string()
    } else {
        host.to_string()
    }
}
