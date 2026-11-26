//! File-selected local instances. Generated snapshots are private implementation details.
use crate::{
    config::{LocalPostgresConfig, RuntimeConfig, TracingConfig},
    server::ServerConfig,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    #[serde(default)]
    externally_managed: bool,
    name: String,
    paths: Paths,
    database: Database,
    #[serde(default)]
    server: toml::Table,
    #[serde(default)]
    logging: TracingConfig,
    #[serde(default)]
    shutdown: crate::api::nodes::GracefulNodeShutdownParams,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub template_dir: PathBuf,
    #[serde(default)]
    pub resource_roots: Vec<PathBuf>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Database {
    mode: String,
    url: Option<String>,
    data_dir: Option<PathBuf>,
    socket_dir: Option<PathBuf>,
    host: Option<String>,
    port: Option<u16>,
    name: Option<String>,
    user: Option<String>,
    #[serde(default)]
    tuning: LocalPostgresConfig,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Instance {
    pub source: PathBuf,
    pub name: String,
    pub paths: Paths,
    pub managed_database: bool,
    pub runtime: RuntimeConfig,
    pub server: ServerConfig,
}
impl Instance {
    pub fn load(path: &Path) -> Result<Self> {
        let source = fs::canonicalize(path).with_context(|| {
            format!(
                "cannot read instance {}; pass --instance FILE",
                path.display()
            )
        })?;
        let base = source.parent().unwrap();
        let mut definition: Definition = toml::from_str(&fs::read_to_string(&source)?)
            .with_context(|| format!("invalid instance {}", source.display()))?;
        ensure!(
            !definition.name.trim().is_empty(),
            "instance name must not be empty"
        );
        for path in [
            &mut definition.paths.state_dir,
            &mut definition.paths.log_dir,
            &mut definition.paths.temp_dir,
            &mut definition.paths.template_dir,
        ] {
            *path = resolve(base, path)?;
        }
        for path in &mut definition.paths.resource_roots {
            *path = resolve(base, path)?;
        }
        let db = definition.database;
        let managed_database = match db.mode.as_str() {
            "managed" => true,
            "external" => false,
            _ => bail!("database.mode must be managed or external"),
        };
        let mut local = db.tuning;
        let url = if managed_database {
            ensure!(
                db.url.is_none(),
                "managed databases use host/port/name/user, not database.url"
            );
            local.data_dir = resolve(
                base,
                &db.data_dir.context("managed database requires data_dir")?,
            )?
            .display()
            .to_string();
            local.socket_dir = resolve(
                base,
                &db.socket_dir
                    .context("managed database requires socket_dir")?,
            )?
            .display()
            .to_string();
            local.log_file = definition
                .paths
                .log_dir
                .join("postgres.log")
                .display()
                .to_string();
            let host = db.host.as_deref().unwrap_or("127.0.0.1");
            ensure!(
                matches!(host, "127.0.0.1" | "localhost" | "::1"),
                "managed local database host must be loopback"
            );
            let mut url = url::Url::parse("postgresql://localhost/")?;
            url.set_host(Some(host))
                .map_err(|_| anyhow::anyhow!("invalid database host"))?;
            url.set_port(Some(db.port.unwrap_or(5400)))
                .map_err(|_| anyhow::anyhow!("invalid database port"))?;
            url.set_username(db.user.as_deref().unwrap_or("postgres"))
                .map_err(|_| anyhow::anyhow!("invalid database user"))?;
            url.set_path(db.name.as_deref().unwrap_or("gammaboard"));
            url.to_string()
        } else {
            ensure!(
                db.data_dir.is_none()
                    && db.socket_dir.is_none()
                    && db.host.is_none()
                    && db.port.is_none()
                    && db.name.is_none()
                    && db.user.is_none(),
                "external databases use database.url only"
            );
            let value = db.url.context("external database requires url")?;
            let parsed = url::Url::parse(&value)?;
            ensure!(
                matches!(parsed.scheme(), "postgres" | "postgresql"),
                "expected a PostgreSQL URL"
            );
            value
        };
        let paths = definition.paths;
        if managed_database {
            let data = Path::new(&local.data_dir);
            ensure!(
                data.parent().is_some(),
                "database data_dir must not be a filesystem root"
            );
            for protected in [
                &paths.state_dir,
                &paths.log_dir,
                &paths.temp_dir,
                &paths.template_dir,
                &source,
            ] {
                ensure!(
                    !protected.starts_with(data),
                    "database data_dir must not contain configuration, control state, logs, temporary files, or templates"
                );
            }
        }
        let mut runtime: RuntimeConfig = toml::from_str("")?;
        runtime.externally_managed = definition.externally_managed;
        runtime.database.url = url;
        runtime.local_postgres = local;
        runtime.tracing = definition.logging;
        runtime.resources.roots = std::iter::once(paths.state_dir.join("resources"))
            .chain(paths.resource_roots.iter().cloned())
            .map(|p| p.display().to_string())
            .collect();
        runtime.resources.control_dir = Some(paths.state_dir.clone());
        runtime.resources.log_dir = Some(paths.log_dir.clone());
        runtime.resources.temp_dir = Some(paths.temp_dir.clone());
        let mut table = definition.server;
        let mut frontend = table
            .remove("frontend")
            .map(|v| v.try_into::<toml::Table>())
            .transpose()?
            .unwrap_or_default();
        if let Some(host) = table.remove("host") {
            table.insert("api_host".into(), host.clone());
        }
        table.remove("dashboard_port"); // accepted migration input; there is one serving port
        if let Some(port) = table.remove("port") {
            table.insert("api_port".into(), port);
        }
        for obsolete in ["host", "port", "advertise_hosts", "access_log"] {
            frontend.remove(obsolete);
        }
        if let Some(dir) = table.remove("dashboard_dir") {
            frontend.insert("build_dir".into(), dir);
        }
        let dir = frontend
            .get("build_dir")
            .and_then(toml::Value::as_str)
            .unwrap_or("dashboard/build");
        let dir = resolve(base, Path::new(dir))?;
        frontend.insert(
            "build_dir".into(),
            toml::Value::String(dir.display().to_string()),
        );
        let port = table
            .get("api_port")
            .and_then(toml::Value::as_integer)
            .unwrap_or(4000);
        table
            .entry("allowed_origins")
            .or_insert(toml::Value::Array(vec![
                toml::Value::String(format!("http://localhost:{port}")),
                toml::Value::String(format!("http://127.0.0.1:{port}")),
            ]));
        table.insert("frontend".into(), toml::Value::Table(frontend));
        for (key, folder) in [
            ("run_templates_dir", "runs"),
            ("task_templates_dir", "tasks"),
            ("node_templates_dir", "nodes"),
        ] {
            ensure!(
                !table.contains_key(key),
                "configure paths.template_dir instead of server.{key}"
            );
            table.insert(
                key.into(),
                toml::Value::String(paths.template_dir.join(folder).display().to_string()),
            );
        }
        ensure!(
            !table.contains_key("database") && !table.contains_key("cleanup"),
            "use top-level database and shutdown sections"
        );
        let mut server: ServerConfig = toml::Value::Table(table).try_into()?;
        server.allowed_origins = server
            .allowed_origins
            .iter()
            .map(|origin| crate::server::normalize_browser_origin(origin))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !server.allowed_origins.is_empty(),
            "server.allowed_origins must not be empty"
        );
        server.name = definition.name.clone();
        server.cleanup = definition.shutdown;
        if managed_database {
            ensure!(
                url::Url::parse(&runtime.database.url)?
                    .port()
                    .is_none_or(|p| p != server.api_port),
                "database and web ports must differ"
            );
        }
        Ok(Self {
            source,
            name: definition.name,
            paths,
            managed_database,
            runtime,
            server,
        })
    }
    pub fn require_local_processes(&self, allowed: bool, action: &str) -> Result<()> {
        crate::local_control::require_local_processes(
            self.runtime.externally_managed,
            allowed,
            action,
        )
    }
    pub fn normalized(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
    pub fn redacted(&self) -> Result<serde_json::Value> {
        let mut value = self.normalized()?;
        if let Ok(mut url) = url::Url::parse(&self.runtime.database.url) {
            if url.password().is_some() {
                let _ = url.set_password(Some("REDACTED"));
            }
            value["runtime"]["database"]["url"] = url.to_string().into();
        }
        if let Some(auth) = value["server"]["auth"].as_object_mut() {
            for key in ["session_secret", "admin_password_hash"] {
                auth.insert(key.into(), "REDACTED".into());
            }
        }
        Ok(value)
    }
    pub fn write_snapshot(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        private_permissions(dir, 0o700)?;
        for (name, data) in [
            ("runtime.toml", toml::to_string(&self.runtime)?),
            ("server.toml", toml::to_string(&self.server)?),
            ("instance.json", serde_json::to_string_pretty(self)?),
        ] {
            let path = dir.join(name);
            fs::write(&path, data)?;
            private_permissions(&path, 0o600)?;
        }
        Ok(())
    }
}
pub fn private_permissions(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}
pub fn resolve(base: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(!path.as_os_str().is_empty(), "paths must not be empty");
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut normalized = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            part => normalized.push(part.as_os_str()),
        }
    }
    let joined = normalized;
    // Canonicalize the existing ancestor, allowing destinations that do not exist yet.
    let mut ancestor = joined.as_path();
    let mut tail = Vec::new();
    while !ancestor.exists() {
        tail.push(ancestor.file_name().context("invalid path")?.to_os_string());
        ancestor = ancestor.parent().context("invalid path")?;
    }
    let mut result = ancestor.canonicalize()?;
    for part in tail.into_iter().rev() {
        result.push(part);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("instance.toml");
        fs::write(&path, "name='test'\n[paths]\nstate_dir='state'\nlog_dir='logs'\ntemp_dir='tmp'\ntemplate_dir='templates'\n[database]\nmode='managed'\ndata_dir='postgres'\nsocket_dir='socket'\n[server]\napi_port=8087\n").unwrap();
        path
    }
    #[test]
    fn paths_are_relative_to_config_and_snapshot_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let instance = Instance::load(&fixture(dir.path())).unwrap();
        assert_eq!(instance.paths.state_dir, dir.path().join("state"));
        assert_eq!(
            instance.runtime.local_postgres.data_dir,
            dir.path().join("postgres").display().to_string()
        );
        assert!(
            instance
                .server
                .allowed_origins
                .contains(&"http://localhost:8087".into())
        );
        instance
            .write_snapshot(&dir.path().join("snapshot"))
            .unwrap();
        let runtime = RuntimeConfig::load(dir.path().join("snapshot/runtime.toml")).unwrap();
        assert_eq!(runtime.database.url, instance.runtime.database.url);
    }
    #[test]
    fn worker_count_and_hpc_backend_are_not_implicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        let base = fs::read_to_string(&path).unwrap();
        for extra in [
            "\n[execution]\nbackend='slurm'",
            "\n[execution]\nlocal_workers=6",
        ] {
            fs::write(&path, format!("{base}{extra}")).unwrap();
            assert!(Instance::load(&path).is_err());
        }
    }
}
