use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use gammaboard::config::{DEFAULT_DEPLOY_CONFIG_PATH, DeployConfig, DeployState, RuntimeConfig};
use gammaboard::core::ControlPlaneStore;
use gammaboard::server::ServerConfig;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::db;
use super::shared::with_control_store;
use gammaboard::api::nodes as node_api;

#[derive(Debug, Args)]
pub struct DeployArgs {
    #[command(subcommand)]
    pub command: DeployCommand,
}

#[derive(Debug, Subcommand)]
pub enum DeployCommand {
    Up(DeployUpArgs),
    Down(DeployDownArgs),
    Status(DeployStatusArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DeployMode {
    Dev,
    Release,
}

impl DeployMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Release => "release",
        }
    }

    fn cargo_args(self) -> [&'static str; 3] {
        match self {
            Self::Dev => ["build", "--profile", "dev-optim"],
            Self::Release => ["build", "--release", ""],
        }
    }

    fn binary_path(self) -> PathBuf {
        match self {
            Self::Dev => PathBuf::from("./target/dev-optim/gammaboard"),
            Self::Release => PathBuf::from("./target/release/gammaboard"),
        }
    }
}

#[derive(Debug, Args)]
pub struct DeployUpArgs {
    #[arg(long = "deploy-config", default_value = DEFAULT_DEPLOY_CONFIG_PATH, value_name = "PATH")]
    deploy_config: PathBuf,
    #[arg(long, value_enum, default_value = "dev")]
    mode: DeployMode,
    #[arg(long)]
    frontend_port: Option<u16>,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    skip_build: bool,
}

#[derive(Debug, Args)]
pub struct DeployDownArgs {
    #[arg(long = "deploy-config", value_name = "PATH")]
    deploy_config: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct DeployStatusArgs {
    #[arg(long = "deploy-config", value_name = "PATH")]
    deploy_config: Option<PathBuf>,
}

pub async fn run_deploy_command(
    args: DeployArgs,
    runtime_config: &RuntimeConfig,
    _runtime_config_path: &Path,
) -> Result<()> {
    match args.command {
        DeployCommand::Up(args) => deploy_up(args, runtime_config).await,
        DeployCommand::Down(args) => deploy_down(args, runtime_config).await,
        DeployCommand::Status(args) => deploy_status(args),
    }
}

async fn deploy_up(args: DeployUpArgs, runtime_config: &RuntimeConfig) -> Result<()> {
    let deploy_config = DeployConfig::load(&args.deploy_config)?;
    deploy_down_internal(&deploy_config, runtime_config).await?;

    fs::create_dir_all("logs")?;
    fs::create_dir_all("tmp/deploy")?;
    fs::create_dir_all("tmp/nginx/client_body")?;
    fs::create_dir_all("tmp/nginx/proxy")?;
    fs::create_dir_all("tmp/nginx/fastcgi")?;
    fs::create_dir_all("tmp/nginx/uwsgi")?;
    fs::create_dir_all("tmp/nginx/scgi")?;

    if !args.skip_build {
        build_frontend()?;
        build_backend(args.mode)?;
    }

    if deploy_config.database.ensure_started {
        db::start_db(
            &runtime_config.local_postgres,
            &runtime_config.database.url,
            false,
        )?;
    }

    let server_config = ServerConfig::load(&deploy_config.api_server.api_server_config)?;
    let frontend_port = args
        .frontend_port
        .unwrap_or(deploy_config.frontend_http.frontend_port);
    write_nginx_config(&deploy_config, &server_config, frontend_port)?;
    start_backend(&deploy_config, args.mode)?;
    start_nginx(&deploy_config)?;
    write_deploy_state(
        &deploy_config,
        &DeployState {
            deploy_config: args.deploy_config.display().to_string(),
            mode: args.mode.as_str().to_string(),
        },
    )?;

    println!("Deploy is up");
    println!("Deploy config: {}", args.deploy_config.display());
    println!("Mode: {}", args.mode.as_str());
    println!(
        "API server config: {}",
        deploy_config.api_server.api_server_config
    );
    println!(
        "Frontend build dir: {}",
        deploy_config.static_site.frontend_build_dir
    );
    println!(
        "Frontend bind: {}:{}",
        deploy_config.frontend_http.frontend_host, frontend_port
    );
    for url in deploy_config.advertised_urls(frontend_port) {
        println!("Open: {url}");
    }
    Ok(())
}

async fn deploy_down(args: DeployDownArgs, runtime_config: &RuntimeConfig) -> Result<()> {
    let deploy_config = load_deploy_config_for_management(args.deploy_config.as_deref())?;
    deploy_down_internal(&deploy_config, runtime_config).await
}

fn deploy_status(args: DeployStatusArgs) -> Result<()> {
    let deploy_config = load_deploy_config_for_management(args.deploy_config.as_deref())?;
    let state = read_deploy_state(&deploy_config).ok();
    let backend_pid = read_pid(&deploy_config.backend_pid_file());
    let nginx_pid = read_pid(&deploy_config.nginx_pid_file());

    println!(
        "deploy_config: {}",
        state
            .as_ref()
            .map(|value| value.deploy_config.as_str())
            .unwrap_or("-")
    );
    println!(
        "mode: {}",
        state
            .as_ref()
            .map(|value| value.mode.as_str())
            .unwrap_or("-")
    );
    println!(
        "backend_running: {}",
        yes_no(
            backend_pid
                .as_ref()
                .is_some_and(|pid| process_is_running(*pid))
        )
    );
    println!(
        "backend_pid: {}",
        backend_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "nginx_running: {}",
        yes_no(
            nginx_pid
                .as_ref()
                .is_some_and(|pid| process_is_running(*pid))
        )
    );
    println!(
        "nginx_pid: {}",
        nginx_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
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
        deploy_config.frontend_http.frontend_host, deploy_config.frontend_http.frontend_port
    );
    for url in deploy_config.advertised_urls(deploy_config.frontend_http.frontend_port) {
        println!("url: {url}");
    }
    Ok(())
}

async fn deploy_down_internal(
    deploy_config: &DeployConfig,
    runtime_config: &RuntimeConfig,
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

    stop_backend(deploy_config)?;
    stop_nginx(deploy_config)?;
    let _ = fs::remove_file(deploy_config.deploy_state_file());
    Ok(())
}

fn load_deploy_config_for_management(path: Option<&Path>) -> Result<DeployConfig> {
    if let Some(path) = path {
        return DeployConfig::load(path);
    }
    let default = DeployConfig::load(DEFAULT_DEPLOY_CONFIG_PATH)?;
    match read_deploy_state(&default) {
        Ok(state) => DeployConfig::load(&state.deploy_config),
        Err(_) => Ok(default),
    }
}

fn build_frontend() -> Result<()> {
    run_command(
        Command::new("npm")
            .arg("run")
            .arg("build")
            .current_dir("dashboard"),
        "npm run build",
    )
}

fn build_backend(mode: DeployMode) -> Result<()> {
    let mut command = Command::new("cargo");
    let cargo_args = mode.cargo_args();
    command.arg(cargo_args[0]).arg(cargo_args[1]);
    if !cargo_args[2].is_empty() {
        command.arg(cargo_args[2]);
    }
    run_command(&mut command, "cargo build")
}

fn start_backend(deploy_config: &DeployConfig, mode: DeployMode) -> Result<()> {
    let binary = mode.binary_path();
    ensure_parent_dir(&deploy_config.backend_log_file())?;
    stop_backend(deploy_config)?;
    let log = fs::File::create(deploy_config.backend_log_file())?;
    let child = Command::new(&binary)
        .arg("server")
        .arg("--server-config")
        .arg(&deploy_config.api_server.api_server_config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()
        .with_context(|| format!("failed to spawn backend {}", binary.display()))?;
    fs::write(deploy_config.backend_pid_file(), child.id().to_string())?;
    Ok(())
}

fn stop_backend(deploy_config: &DeployConfig) -> Result<()> {
    if let Some(pid) = read_pid(&deploy_config.backend_pid_file()) {
        terminate_pid(pid)?;
        let _ = fs::remove_file(deploy_config.backend_pid_file());
    }
    Ok(())
}

fn start_nginx(deploy_config: &DeployConfig) -> Result<()> {
    stop_nginx(deploy_config)?;
    run_command(
        Command::new("nginx")
            .arg("-e")
            .arg(deploy_config.nginx_error_log())
            .arg("-p")
            .arg(std::env::current_dir()?)
            .arg("-c")
            .arg(deploy_config.nginx_generated_config()),
        "nginx",
    )?;
    if !deploy_config.nginx_pid_file().exists() {
        bail!(
            "nginx did not write pid file {}",
            deploy_config.nginx_pid_file().display()
        );
    }
    Ok(())
}

fn stop_nginx(deploy_config: &DeployConfig) -> Result<()> {
    if deploy_config.nginx_pid_file().exists() {
        let _ = Command::new("nginx")
            .arg("-e")
            .arg(deploy_config.nginx_error_log())
            .arg("-p")
            .arg(std::env::current_dir()?)
            .arg("-c")
            .arg(deploy_config.nginx_generated_config())
            .arg("-s")
            .arg("quit")
            .status();
        if let Some(pid) = read_pid(&deploy_config.nginx_pid_file()) {
            if process_is_running(pid) {
                terminate_pid(pid)?;
            }
        }
        let _ = fs::remove_file(deploy_config.nginx_pid_file());
    }
    Ok(())
}

fn write_nginx_config(
    deploy_config: &DeployConfig,
    server_config: &ServerConfig,
    frontend_port: u16,
) -> Result<()> {
    let backend = server_config.bind_addr();
    let config = format!(
        "worker_processes 1;\n\
pid {pid};\n\
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
    access_log {access_log};\n\
    error_log {error_log} warn;\n\
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
        pid = deploy_config.nginx_pid_file().display(),
        access_log = deploy_config.nginx_access_log().display(),
        error_log = deploy_config.nginx_error_log().display(),
        listen_host = deploy_config.frontend_http.frontend_host,
        listen_port = frontend_port,
        server_name = deploy_config.frontend_http.frontend_server_name,
        static_dir = deploy_config.static_site.frontend_build_dir,
        backend = backend,
    );
    fs::write(deploy_config.nginx_generated_config(), config).with_context(|| {
        format!(
            "failed writing {}",
            deploy_config.nginx_generated_config().display()
        )
    })
}

fn write_deploy_state(deploy_config: &DeployConfig, state: &DeployState) -> Result<()> {
    fs::write(
        deploy_config.deploy_state_file(),
        toml::to_string(state).context("failed to serialize deploy state")?,
    )
    .with_context(|| {
        format!(
            "failed writing {}",
            deploy_config.deploy_state_file().display()
        )
    })
}

fn read_deploy_state(deploy_config: &DeployConfig) -> Result<DeployState> {
    let path = deploy_config.deploy_state_file();
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed reading deploy state {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("failed parsing deploy state {}", path.display()))
}

fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()
}

fn terminate_pid(pid: u32) -> Result<()> {
    let _ = Command::new("kill").arg(pid.to_string()).status();
    if process_is_running(pid) {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
    Ok(())
}

fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_command(command: &mut Command, label: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to spawn {label}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{label} exited with status {status}")
    }
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
