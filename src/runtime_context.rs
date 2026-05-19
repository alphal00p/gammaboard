use crate::config::{LocalPostgresConfig, RuntimeConfig, normalize_local_postgres_paths};
use crate::resources::{initialize_resource_roots, primary_resource_root};
use anyhow::Context;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct RuntimeOverrides {
    pub database_url: Option<String>,
    pub resource_roots: Vec<String>,
    pub postgres_data_dir: Option<String>,
    pub postgres_socket_dir: Option<String>,
    pub postgres_log_file: Option<String>,
    pub postgres_listen_addresses: Option<String>,
    pub postgres_host_auth_cidr: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    runtime_config_path: PathBuf,
    runtime_config: RuntimeConfig,
    primary_resource_root: PathBuf,
}

impl RuntimeContext {
    pub fn load(
        runtime_config_path: impl AsRef<Path>,
        overrides: RuntimeOverrides,
    ) -> anyhow::Result<Self> {
        let runtime_config_path = runtime_config_path.as_ref().to_path_buf();
        let mut runtime_config = RuntimeConfig::load(&runtime_config_path)?;
        apply_runtime_overrides(&mut runtime_config, overrides);
        normalize_local_postgres_paths(
            &mut runtime_config.local_postgres,
            &runtime_config.resources.roots,
        );
        initialize_resource_roots(&runtime_config.resources.roots);
        let primary_resource_root = runtime_config.primary_resource_root();
        Ok(Self {
            runtime_config_path,
            runtime_config,
            primary_resource_root,
        })
    }

    pub fn runtime_config_path(&self) -> &Path {
        &self.runtime_config_path
    }

    pub fn runtime_config(&self) -> &RuntimeConfig {
        &self.runtime_config
    }

    pub fn primary_resource_root(&self) -> &Path {
        &self.primary_resource_root
    }

    pub fn runtime_cli_args(&self) -> Vec<String> {
        let mut args = vec![
            "--runtime-config".to_string(),
            self.runtime_config_path.display().to_string(),
            "--database-url".to_string(),
            self.runtime_config.database.url.clone(),
        ];
        for root in &self.runtime_config.resources.roots {
            args.push("--resource-root".to_string());
            args.push(root.clone());
        }
        args.extend([
            "--postgres-data-dir".to_string(),
            self.runtime_config.local_postgres.data_dir.clone(),
            "--postgres-socket-dir".to_string(),
            self.runtime_config.local_postgres.socket_dir.clone(),
            "--postgres-log-file".to_string(),
            self.runtime_config.local_postgres.log_file.clone(),
            "--postgres-listen-addresses".to_string(),
            self.runtime_config.local_postgres.listen_addresses.clone(),
            "--postgres-host-auth-cidr".to_string(),
            self.runtime_config.local_postgres.host_auth_cidr.clone(),
        ]);
        args
    }

    pub fn node_log_paths(&self, node_name: &str) -> anyhow::Result<(PathBuf, PathBuf)> {
        let dir = self.primary_resource_root.join("logs/nodes");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed creating node log directory {}", dir.display()))?;
        Ok((
            dir.join(format!("{node_name}.stdout.log")),
            dir.join(format!("{node_name}.stderr.log")),
        ))
    }

    pub fn deploy_tmp_dir(&self, frontend_port: u16) -> PathBuf {
        self.primary_resource_root
            .join(format!("nginx/{frontend_port}"))
    }

    pub fn evaluator_tmp_dir(&self, kind: &str) -> PathBuf {
        self.primary_resource_root
            .join(format!("tmp/evaluators/{kind}"))
    }
}

pub fn evaluator_tmp_dir(kind: &str) -> anyhow::Result<PathBuf> {
    let root = primary_resource_root().context("failed resolving primary resource root")?;
    Ok(root.join(format!("tmp/evaluators/{kind}")))
}

fn apply_runtime_overrides(config: &mut RuntimeConfig, overrides: RuntimeOverrides) {
    if let Some(value) = overrides.database_url {
        config.database.url = value;
    }
    if !overrides.resource_roots.is_empty() {
        config.resources.roots = overrides.resource_roots;
    }
    apply_local_postgres_overrides(
        &mut config.local_postgres,
        overrides.postgres_data_dir,
        overrides.postgres_socket_dir,
        overrides.postgres_log_file,
        overrides.postgres_listen_addresses,
        overrides.postgres_host_auth_cidr,
    );
}

fn apply_local_postgres_overrides(
    local_postgres: &mut LocalPostgresConfig,
    postgres_data_dir: Option<String>,
    postgres_socket_dir: Option<String>,
    postgres_log_file: Option<String>,
    postgres_listen_addresses: Option<String>,
    postgres_host_auth_cidr: Option<String>,
) {
    if let Some(value) = postgres_data_dir {
        local_postgres.data_dir = value;
    }
    if let Some(value) = postgres_socket_dir {
        local_postgres.socket_dir = value;
    }
    if let Some(value) = postgres_log_file {
        local_postgres.log_file = value;
    }
    if let Some(value) = postgres_listen_addresses {
        local_postgres.listen_addresses = value;
    }
    if let Some(value) = postgres_host_auth_cidr {
        local_postgres.host_auth_cidr = value;
    }
}
