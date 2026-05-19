use crate::config::{LocalPostgresConfig, RuntimeConfig, normalize_local_postgres_paths};
use crate::resources::{initialize_resource_roots, primary_resource_root};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct RuntimeOverrides {
    pub database_url: Option<String>,
    pub resource_roots: Vec<String>,
    pub port_offset: u16,
    pub postgres_public: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    runtime_config_path: PathBuf,
    runtime_config: RuntimeConfig,
    primary_resource_root: PathBuf,
    database_url_override: Option<String>,
    port_offset: u16,
    postgres_public: bool,
}

impl RuntimeContext {
    pub fn load(
        runtime_config_path: impl AsRef<Path>,
        overrides: RuntimeOverrides,
    ) -> anyhow::Result<Self> {
        let runtime_config_path = runtime_config_path.as_ref().to_path_buf();
        let database_url_override = overrides.database_url.clone();
        let port_offset = overrides.port_offset;
        let postgres_public = overrides.postgres_public;
        let mut runtime_config = RuntimeConfig::load(&runtime_config_path)?;
        apply_runtime_overrides(&mut runtime_config, overrides)?;
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
            database_url_override,
            port_offset,
            postgres_public,
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

    pub fn port_offset(&self) -> u16 {
        self.port_offset
    }

    pub fn runtime_cli_args(&self) -> Vec<String> {
        let mut args = vec![
            "--runtime-config".to_string(),
            self.runtime_config_path.display().to_string(),
        ];
        for root in &self.runtime_config.resources.roots {
            args.push("--resource-root".to_string());
            args.push(root.clone());
        }
        if let Some(database_url) = &self.database_url_override {
            args.push("--database-url".to_string());
            args.push(database_url.clone());
        }
        if self.port_offset != 0 {
            args.push("--port-offset".to_string());
            args.push(self.port_offset.to_string());
        }
        if self.postgres_public {
            args.push("--postgres-public".to_string());
        }
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

fn apply_runtime_overrides(config: &mut RuntimeConfig, overrides: RuntimeOverrides) -> Result<()> {
    if let Some(value) = overrides.database_url {
        config.database.url = value;
    }
    if !overrides.resource_roots.is_empty() {
        config.resources.roots = overrides.resource_roots;
    }
    if overrides.port_offset != 0 {
        apply_port_offset(config, overrides.port_offset)?;
    }
    apply_local_postgres_overrides(&mut config.local_postgres, overrides.postgres_public);
    Ok(())
}

fn apply_local_postgres_overrides(local_postgres: &mut LocalPostgresConfig, postgres_public: bool) {
    if postgres_public {
        local_postgres.listen_addresses = "0.0.0.0".to_string();
        local_postgres.host_auth_cidr = "0.0.0.0/0".to_string();
    }
}

fn apply_port_offset(config: &mut RuntimeConfig, port_offset: u16) -> Result<()> {
    let mut database_url = Url::parse(&config.database.url)
        .with_context(|| format!("invalid runtime database URL: {}", config.database.url))?;
    let base_db_port = database_url
        .port()
        .ok_or_else(|| anyhow::anyhow!("runtime database URL must include an explicit port"))?;
    let shifted_db_port = checked_add_port(base_db_port, port_offset, "postgres port")?;
    database_url
        .set_port(Some(shifted_db_port))
        .map_err(|_| anyhow::anyhow!("failed setting shifted postgres port"))?;
    config.database.url = database_url.to_string();

    let old_data_dir = PathBuf::from(&config.local_postgres.data_dir);
    let old_log_file = PathBuf::from(&config.local_postgres.log_file);
    let suffix = format!("-{port_offset}");
    let new_data_dir = append_suffix_to_path(&old_data_dir, &suffix)?;
    config.local_postgres.data_dir = new_data_dir.display().to_string();
    config.local_postgres.socket_dir =
        append_suffix_to_path(Path::new(&config.local_postgres.socket_dir), &suffix)?
            .display()
            .to_string();
    let new_log_file = if old_log_file.starts_with(&old_data_dir) {
        let relative = old_log_file
            .strip_prefix(&old_data_dir)
            .map_err(|err| anyhow::anyhow!("failed deriving shifted postgres log path: {err}"))?;
        new_data_dir.join(relative)
    } else {
        append_suffix_to_path(&old_log_file, &suffix)?
    };
    config.local_postgres.log_file = new_log_file.display().to_string();
    Ok(())
}

pub fn checked_add_port(base: u16, offset: u16, label: &str) -> Result<u16> {
    base.checked_add(offset)
        .ok_or_else(|| anyhow::anyhow!("{label} overflow with port_offset={offset} (base={base})"))
}

fn append_suffix_to_path(path: &Path, suffix: &str) -> Result<PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot append suffix to path without file name: {}",
            path.display()
        )
    })?;
    let mut updated = file_name.to_os_string();
    updated.push(suffix);
    Ok(path.with_file_name(updated))
}
