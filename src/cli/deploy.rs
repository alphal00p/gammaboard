use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gammaboard::api::nodes as node_api;
use gammaboard::config::{
    DEFAULT_DEPLOY_CONFIG_PATH, DeployConfig, RuntimeConfig, normalize_config_path,
    primary_resource_root_from_config,
};
use gammaboard::server::ServerConfig;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use url::Url;

use super::db;
use super::shared::with_control_store;

#[derive(Debug, Args)]
pub struct DeployArgs {
    #[command(subcommand)]
    pub command: DeployCommand,
}

#[derive(Debug, Subcommand)]
pub enum DeployCommand {
    /// Run the local deploy stack in the foreground.
    Run(DeployRunArgs),
}

#[derive(Debug, Args)]
pub struct DeployRunArgs {
    #[arg(long = "deploy-config", default_value = DEFAULT_DEPLOY_CONFIG_PATH, value_name = "PATH")]
    deploy_config: PathBuf,
    #[arg(long, default_value_t = 0)]
    port_offset: u16,
    #[arg(long)]
    api_port: Option<u16>,
    #[arg(long = "allowed-origin", value_name = "ORIGIN")]
    allowed_origins: Vec<String>,
}

pub async fn run_deploy_command(
    args: DeployArgs,
    runtime_config: &RuntimeConfig,
    runtime_config_path: &Path,
) -> Result<()> {
    match args.command {
        DeployCommand::Run(args) => deploy_run(args, runtime_config, runtime_config_path).await,
    }
}

async fn deploy_run(
    args: DeployRunArgs,
    runtime_config: &RuntimeConfig,
    runtime_config_path: &Path,
) -> Result<()> {
    let deploy_config = DeployConfig::load(&args.deploy_config)?;
    let mut deploy_config = deploy_config;
    let mut server_config = ServerConfig::load(&deploy_config.api_server.api_server_config)?;
    let runtime_config = runtime_config.clone();
    let mut runtime_config = runtime_config;
    resolve_local_postgres_paths_against_resources(&mut runtime_config);
    apply_port_offset(
        &mut deploy_config,
        &mut server_config,
        &mut runtime_config,
        args.port_offset,
    )?;
    let frontend_port = deploy_config.frontend_http.frontend_port;
    if let Some(api_port) = args.api_port {
        server_config.api_port = api_port;
    }
    server_config
        .allowed_origins
        .extend(args.allowed_origins.clone());
    validate_frontend_build(&deploy_config)?;

    if deploy_config.database.ensure_started {
        db::start_db(&runtime_config.local_postgres, &runtime_config.database.url)?;
    }

    let deploy_paths = DeployRuntimePaths::new(frontend_port);
    prepare_runtime_dirs(&deploy_paths)?;
    write_nginx_config(&deploy_config, &server_config, frontend_port, &deploy_paths)?;

    let mut backend = start_backend(
        &deploy_config,
        &server_config,
        runtime_config_path,
        &runtime_config,
    )?;
    let mut nginx = start_nginx(&deploy_config, &deploy_paths)?;

    println!("deploy running");
    println!("deploy_config: {}", args.deploy_config.display());
    println!(
        "api_server_config: {}",
        deploy_config.api_server.api_server_config
    );
    println!(
        "frontend_build_dir: {}",
        deploy_config.static_site.frontend_build_dir
    );
    println!(
        "frontend_bind: {}:{}",
        deploy_config.frontend_http.frontend_host, frontend_port
    );
    for url in deploy_config.advertised_urls(frontend_port) {
        println!("open: {url}");
    }

    let result = supervise_children(&mut backend, &mut nginx).await;
    let cleanup_result = cleanup_deploy(&deploy_config, &runtime_config, &mut backend, &mut nginx)
        .await
        .context("deploy cleanup failed");

    match (result, cleanup_result) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn validate_frontend_build(deploy_config: &DeployConfig) -> Result<()> {
    let frontend_dir = Path::new(&deploy_config.static_site.frontend_build_dir);
    if !frontend_dir.is_dir() {
        bail!(
            "frontend build dir does not exist or is not a directory: {}",
            frontend_dir.display()
        );
    }
    let index = frontend_dir.join("index.html");
    if !index.is_file() {
        bail!(
            "frontend build is missing index.html at {}; check static_site.frontend_build_dir and rebuild/sync frontend artifacts",
            index.display()
        );
    }
    Ok(())
}

fn apply_port_offset(
    deploy_config: &mut DeployConfig,
    server_config: &mut ServerConfig,
    runtime_config: &mut RuntimeConfig,
    port_offset: u16,
) -> Result<()> {
    if port_offset == 0 {
        return Ok(());
    }

    deploy_config.frontend_http.frontend_port = checked_add_port(
        deploy_config.frontend_http.frontend_port,
        port_offset,
        "frontend port",
    )?;
    server_config.api_port = checked_add_port(server_config.api_port, port_offset, "api port")?;

    for origin in &mut server_config.allowed_origins {
        let Ok(parsed) = Url::parse(origin) else {
            continue;
        };
        let Some(port) = parsed.port() else {
            continue;
        };
        let shifted = checked_add_port(port, port_offset, "allowed origin port")?;
        let mut updated = parsed;
        updated
            .set_port(Some(shifted))
            .map_err(|_| anyhow::anyhow!("failed setting shifted port for origin {origin}"))?;
        *origin = updated.origin().ascii_serialization();
    }

    let mut database_url = Url::parse(&runtime_config.database.url).with_context(|| {
        format!(
            "invalid runtime database URL: {}",
            runtime_config.database.url
        )
    })?;
    let base_db_port = database_url
        .port()
        .ok_or_else(|| anyhow::anyhow!("runtime database URL must include an explicit port"))?;
    let shifted_db_port = checked_add_port(base_db_port, port_offset, "postgres port")?;
    database_url
        .set_port(Some(shifted_db_port))
        .map_err(|_| anyhow::anyhow!("failed setting shifted postgres port"))?;
    runtime_config.database.url = database_url.to_string();

    let old_data_dir = PathBuf::from(&runtime_config.local_postgres.data_dir);
    let old_log_file = PathBuf::from(&runtime_config.local_postgres.log_file);
    let suffix = format!("-{port_offset}");
    let new_data_dir = append_suffix_to_path(&old_data_dir, &suffix)?;
    runtime_config.local_postgres.data_dir = new_data_dir.display().to_string();
    runtime_config.local_postgres.socket_dir = append_suffix_to_path(
        Path::new(&runtime_config.local_postgres.socket_dir),
        &suffix,
    )?
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
    runtime_config.local_postgres.log_file = new_log_file.display().to_string();
    Ok(())
}

fn checked_add_port(base: u16, offset: u16, label: &str) -> Result<u16> {
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

fn resolve_local_postgres_paths_against_resources(runtime_config: &mut RuntimeConfig) {
    let resource_root = primary_resource_root_from_config(&runtime_config.resources.roots);
    runtime_config.local_postgres.data_dir =
        normalize_config_path(&resource_root, &runtime_config.local_postgres.data_dir)
            .display()
            .to_string();
    runtime_config.local_postgres.socket_dir =
        normalize_config_path(&resource_root, &runtime_config.local_postgres.socket_dir)
            .display()
            .to_string();
    runtime_config.local_postgres.log_file =
        normalize_config_path(&resource_root, &runtime_config.local_postgres.log_file)
            .display()
            .to_string();
}

async fn cleanup_deploy(
    deploy_config: &DeployConfig,
    runtime_config: &RuntimeConfig,
    backend: &mut Child,
    nginx: &mut Child,
) -> Result<()> {
    let sampler_drain_error = with_control_store(
        runtime_config,
        10,
        true,
        "deploy_stop_all_nodes_gracefully",
        |store| async move {
            let stopped = node_api::stop_all_nodes_gracefully(
                &store,
                node_api::GracefulNodeShutdownParams {
                    sampler_drain_timeout: Duration::from_secs(
                        deploy_config.cleanup.sampler_drain_timeout_seconds,
                    ),
                    node_stop_timeout: Duration::from_secs(
                        deploy_config.cleanup.node_stop_timeout_seconds,
                    ),
                    poll_interval: Duration::from_millis(deploy_config.cleanup.poll_interval_ms),
                },
            )
            .await?;
            tracing::info!(
                assignments_cleared = stopped.assignments_cleared,
                rows_updated = stopped.rows_updated,
                active_samplers_remaining = stopped.active_samplers_remaining,
                live_nodes_remaining = stopped.live_nodes_remaining,
                sampler_drain_timed_out = stopped.sampler_drain_timed_out,
                node_stop_timed_out = stopped.node_stop_timed_out,
                "graceful node shutdown completed"
            );
            if stopped.sampler_drain_timed_out {
                bail!(
                    "timed out waiting for sampler nodes to persist state: active_samplers_remaining={}",
                    stopped.active_samplers_remaining
                );
            }
            Ok(())
        },
    )
    .await
    .err();

    terminate_child(nginx, "nginx")?;
    terminate_child(backend, "backend")?;

    if let Some(err) = sampler_drain_error {
        return Err(err.context("database left running because sampler shutdown did not complete"));
    }

    if deploy_config.database.ensure_started {
        db::stop_db(&runtime_config.local_postgres)?;
    }
    Ok(())
}

async fn supervise_children(backend: &mut Child, nginx: &mut Child) -> Result<()> {
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("failed to install SIGTERM handler")?;

    loop {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("received Ctrl-C; shutting down deploy");
                    return Ok(());
                }
                _ = sigterm.recv() => {
                    println!("received SIGTERM; shutting down deploy");
                    return Ok(());
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if check_child_running(backend, "backend", true)? {
                        return Ok(());
                    }
                    check_child_running(nginx, "nginx", false)?;
                }
            }
        }

        #[cfg(not(unix))]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("received Ctrl-C; shutting down deploy");
                    return Ok(());
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if check_child_running(backend, "backend", true)? {
                        return Ok(());
                    }
                    check_child_running(nginx, "nginx", false)?;
                }
            }
        }
    }
}

fn check_child_running(
    child: &mut Child,
    label: &str,
    success_means_shutdown: bool,
) -> Result<bool> {
    if let Some(status) = child
        .try_wait()
        .with_context(|| format!("failed checking {label} process"))?
    {
        if success_means_shutdown && status.success() {
            println!("{label} exited; shutting down deploy");
            return Ok(true);
        }
        bail!("{label} exited unexpectedly with status {status}");
    }
    Ok(false)
}

struct DeployRuntimePaths {
    root: PathBuf,
    nginx_config: PathBuf,
    nginx_pid: PathBuf,
    client_body_temp: PathBuf,
    proxy_temp: PathBuf,
    fastcgi_temp: PathBuf,
    uwsgi_temp: PathBuf,
    scgi_temp: PathBuf,
}

impl DeployRuntimePaths {
    fn new(frontend_port: u16) -> Self {
        let root = PathBuf::from(format!("tmp/deploy/{frontend_port}"));
        Self {
            nginx_config: root.join("nginx.conf"),
            nginx_pid: root.join("nginx.pid"),
            client_body_temp: root.join("nginx/client_body"),
            proxy_temp: root.join("nginx/proxy"),
            fastcgi_temp: root.join("nginx/fastcgi"),
            uwsgi_temp: root.join("nginx/uwsgi"),
            scgi_temp: root.join("nginx/scgi"),
            root,
        }
    }
}

fn prepare_runtime_dirs(paths: &DeployRuntimePaths) -> Result<()> {
    for path in [
        &paths.root,
        &paths.client_body_temp,
        &paths.proxy_temp,
        &paths.fastcgi_temp,
        &paths.uwsgi_temp,
        &paths.scgi_temp,
    ] {
        fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
    }
    Ok(())
}

fn start_backend(
    deploy_config: &DeployConfig,
    server_config: &ServerConfig,
    runtime_config_path: &Path,
    runtime_config: &RuntimeConfig,
) -> Result<Child> {
    let binary = std::env::current_exe().context("failed to resolve current executable path")?;
    let mut command = Command::new(&binary);
    command.arg("--runtime-config").arg(runtime_config_path);
    command
        .arg("--database-url")
        .arg(&runtime_config.database.url)
        .arg("--postgres-data-dir")
        .arg(&runtime_config.local_postgres.data_dir)
        .arg("--postgres-socket-dir")
        .arg(&runtime_config.local_postgres.socket_dir)
        .arg("--postgres-log-file")
        .arg(&runtime_config.local_postgres.log_file);
    command
        .arg("server")
        .arg("--server-config")
        .arg(&deploy_config.api_server.api_server_config);
    command
        .arg("--api-port")
        .arg(server_config.api_port.to_string());
    for origin in &server_config.allowed_origins {
        command.arg("--allowed-origin").arg(origin);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to spawn backend {}", binary.display()))
}

fn start_nginx(_deploy_config: &DeployConfig, paths: &DeployRuntimePaths) -> Result<Child> {
    Command::new("nginx")
        .arg("-e")
        .arg("/dev/stderr")
        .arg("-p")
        .arg(std::env::current_dir()?)
        .arg("-c")
        .arg(&paths.nginx_config)
        .arg("-g")
        .arg("daemon off;")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn nginx")
}

fn terminate_child(child: &mut Child, label: &str) -> Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        let _ = Command::new("kill").arg(child.id().to_string()).status();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    child
        .kill()
        .with_context(|| format!("failed to kill {label} process"))?;
    let _ = child.wait();
    Ok(())
}

fn write_nginx_config(
    deploy_config: &DeployConfig,
    server_config: &ServerConfig,
    frontend_port: u16,
    paths: &DeployRuntimePaths,
) -> Result<()> {
    let backend = server_config.bind_addr();
    let access_log = if deploy_config.frontend_http.access_log {
        "access_log /dev/stdout;"
    } else {
        "access_log off;"
    };
    let config = format!(
        "worker_processes 1;\n\
pid {pid_file};\n\
\n\
events {{\n    worker_connections 1024;\n}}\n\
\n\
http {{\n\
    default_type application/octet-stream;\n\
    types {{\n\
        text/html html htm shtml;\n\
        text/css css;\n\
        text/xml xml;\n\
        application/javascript js mjs;\n\
        application/json json;\n\
        application/wasm wasm;\n\
        image/svg+xml svg svgz;\n\
        image/png png;\n\
        image/jpeg jpg jpeg;\n\
        image/gif gif;\n\
        font/woff woff;\n\
        font/woff2 woff2;\n\
    }}\n\
    types_hash_max_size 4096;\n\
    sendfile on;\n\
\n\
    {access_log}\n\
    error_log /dev/stderr warn;\n\
    client_body_temp_path {client_body_temp};\n\
    proxy_temp_path {proxy_temp};\n\
    fastcgi_temp_path {fastcgi_temp};\n\
    uwsgi_temp_path {uwsgi_temp};\n\
    scgi_temp_path {scgi_temp};\n\
\n\
    server {{\n\
        listen {listen_host}:{listen_port};\n\
        server_name {server_name};\n\
\n\
        root {static_dir};\n\
        index index.html;\n\
\n\
        location / {{\n\
            try_files $uri /index.html;\n\
        }}\n\
\n\
        location /api/ {{\n\
            proxy_pass http://{backend}/api/;\n\
            proxy_set_header Host $host;\n\
            proxy_set_header X-Forwarded-Proto http;\n\
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
        }}\n\
    }}\n\
}}\n",
        listen_host = deploy_config.frontend_http.frontend_host,
        listen_port = frontend_port,
        server_name = deploy_config.frontend_http.frontend_server_name,
        static_dir = deploy_config.static_site.frontend_build_dir,
        backend = backend,
        access_log = access_log,
        pid_file = paths.nginx_pid.display(),
        client_body_temp = paths.client_body_temp.display(),
        proxy_temp = paths.proxy_temp.display(),
        fastcgi_temp = paths.fastcgi_temp.display(),
        uwsgi_temp = paths.uwsgi_temp.display(),
        scgi_temp = paths.scgi_temp.display(),
    );
    fs::write(&paths.nginx_config, config)
        .with_context(|| format!("failed writing {}", paths.nginx_config.display()))
}
