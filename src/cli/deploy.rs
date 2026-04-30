use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gammaboard::api::nodes as node_api;
use gammaboard::config::{DEFAULT_DEPLOY_CONFIG_PATH, DeployConfig, RuntimeConfig};
use gammaboard::core::ControlPlaneStore;
use gammaboard::server::ServerConfig;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

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
    #[arg(long)]
    frontend_port: Option<u16>,
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
    let server_config = ServerConfig::load(&deploy_config.api_server.api_server_config)?;
    let frontend_port = args
        .frontend_port
        .unwrap_or(deploy_config.frontend_http.frontend_port);

    if deploy_config.database.ensure_started {
        db::start_db(&runtime_config.local_postgres, &runtime_config.database.url)?;
    }

    prepare_runtime_dirs()?;
    write_nginx_config(&deploy_config, &server_config, frontend_port)?;

    let mut backend = start_backend(&deploy_config, runtime_config_path)?;
    let mut nginx = start_nginx(&deploy_config)?;

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
    let cleanup_result = cleanup_deploy(&deploy_config, runtime_config, &mut backend, &mut nginx)
        .await
        .context("deploy cleanup failed");

    match (result, cleanup_result) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn cleanup_deploy(
    deploy_config: &DeployConfig,
    runtime_config: &RuntimeConfig,
    backend: &mut Child,
    nginx: &mut Child,
) -> Result<()> {
    if deploy_config.cleanup.pause_runs {
        let _ = with_control_store(
            runtime_config,
            10,
            true,
            "deploy_pause_all_runs",
            |store| async move {
                let assignments_cleared = store.clear_all_desired_assignments().await?;
                tracing::info!("paused all runs: assignments_cleared={assignments_cleared}");
                Ok(())
            },
        )
        .await;
    }
    if deploy_config.cleanup.stop_nodes {
        let _ = with_control_store(
            runtime_config,
            10,
            true,
            "deploy_stop_all_nodes",
            |store| async move {
                let stopped = node_api::stop_all_nodes(&store).await?;
                tracing::info!(
                    "requested shutdown for all nodes: rows_updated={}",
                    stopped.rows_updated
                );
                Ok(())
            },
        )
        .await;
    }

    terminate_child(nginx, "nginx")?;
    terminate_child(backend, "backend")?;

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
                    check_child_running(backend, "backend")?;
                    check_child_running(nginx, "nginx")?;
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
                    check_child_running(backend, "backend")?;
                    check_child_running(nginx, "nginx")?;
                }
            }
        }
    }
}

fn check_child_running(child: &mut Child, label: &str) -> Result<()> {
    if let Some(status) = child
        .try_wait()
        .with_context(|| format!("failed checking {label} process"))?
    {
        bail!("{label} exited unexpectedly with status {status}");
    }
    Ok(())
}

fn prepare_runtime_dirs() -> Result<()> {
    for path in [
        "tmp/deploy",
        "tmp/nginx/client_body",
        "tmp/nginx/proxy",
        "tmp/nginx/fastcgi",
        "tmp/nginx/uwsgi",
        "tmp/nginx/scgi",
    ] {
        fs::create_dir_all(path).with_context(|| format!("failed to create {path}"))?;
    }
    Ok(())
}

fn start_backend(deploy_config: &DeployConfig, runtime_config_path: &Path) -> Result<Child> {
    let binary = std::env::current_exe().context("failed to resolve current executable path")?;
    Command::new(&binary)
        .arg("--runtime-config")
        .arg(runtime_config_path)
        .arg("server")
        .arg("--server-config")
        .arg(&deploy_config.api_server.api_server_config)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to spawn backend {}", binary.display()))
}

fn start_nginx(deploy_config: &DeployConfig) -> Result<Child> {
    Command::new("nginx")
        .arg("-e")
        .arg("/dev/stderr")
        .arg("-p")
        .arg(std::env::current_dir()?)
        .arg("-c")
        .arg(deploy_config.nginx_generated_config())
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
) -> Result<()> {
    let backend = server_config.bind_addr();
    let access_log = if deploy_config.frontend_http.access_log {
        "access_log /dev/stdout;"
    } else {
        "access_log off;"
    };
    let config = format!(
        "worker_processes 1;\n\
pid tmp/deploy/nginx.pid;\n\
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
    client_body_temp_path tmp/nginx/client_body;\n\
    proxy_temp_path tmp/nginx/proxy;\n\
    fastcgi_temp_path tmp/nginx/fastcgi;\n\
    uwsgi_temp_path tmp/nginx/uwsgi;\n\
    scgi_temp_path tmp/nginx/scgi;\n\
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
    );
    fs::write(deploy_config.nginx_generated_config(), config).with_context(|| {
        format!(
            "failed writing {}",
            deploy_config.nginx_generated_config().display()
        )
    })
}
