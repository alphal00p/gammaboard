use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use gammaboard::api::nodes as node_api;
use gammaboard::config::{DEFAULT_SERVER_CONFIG_PATH, RuntimeConfig};
use gammaboard::runtime_context::{RuntimeContext, checked_add_port};
use gammaboard::server::ServerConfig;
use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
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
    #[arg(long = "server-config", default_value = DEFAULT_SERVER_CONFIG_PATH, value_name = "PATH")]
    server_config: PathBuf,
    #[arg(long)]
    api_port: Option<u16>,
    #[arg(long = "allowed-origin", value_name = "ORIGIN")]
    allowed_origins: Vec<String>,
}

pub async fn run_deploy_command(args: DeployArgs, runtime: &RuntimeContext) -> Result<()> {
    match args.command {
        DeployCommand::Run(args) => deploy_run(args, runtime).await,
    }
}

async fn deploy_run(args: DeployRunArgs, runtime: &RuntimeContext) -> Result<()> {
    let mut server_config = ServerConfig::load(&args.server_config)?;
    let runtime_config = runtime.runtime_config();
    apply_port_offset(&mut server_config, runtime.port_offset())?;
    let frontend_port = server_config.frontend.port;
    if let Some(api_port) = args.api_port {
        server_config.api_port = api_port;
    }
    server_config
        .allowed_origins
        .extend(args.allowed_origins.clone());
    for warning in server_config.security_warnings() {
        eprintln!("WARNING: {warning}");
    }
    preflight_deploy_ports(&server_config)?;
    validate_frontend_build(&server_config)?;

    if server_config.database.ensure_started {
        db::start_db(&runtime_config.local_postgres, &runtime_config.database.url)?;
    }

    let deploy_paths = DeployRuntimePaths::new(frontend_port, runtime);
    prepare_runtime_dirs(&deploy_paths)?;
    write_nginx_config(&server_config, frontend_port, &deploy_paths)?;

    let mut backend = start_backend(&server_config, runtime)?;
    let mut nginx = start_nginx(&deploy_paths)?;

    println!("deploy running");
    println!("server_config: {}", args.server_config.display());
    println!("frontend_build_dir: {}", server_config.frontend.build_dir);
    println!(
        "frontend_bind: {}:{}",
        server_config.frontend.host, frontend_port
    );
    for url in server_config.advertised_urls(frontend_port) {
        println!("open: {url}");
    }

    let result = supervise_children(&mut backend, &mut nginx).await;
    let cleanup_result = cleanup_deploy(&server_config, &runtime_config, &mut backend, &mut nginx)
        .await
        .context("deploy cleanup failed");

    match (result, cleanup_result) {
        (Err(err), _) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn validate_frontend_build(server_config: &ServerConfig) -> Result<()> {
    let frontend_dir = Path::new(&server_config.frontend.build_dir);
    if !frontend_dir.is_dir() {
        bail!(
            "frontend build dir does not exist or is not a directory: {}",
            frontend_dir.display()
        );
    }
    let index = frontend_dir.join("index.html");
    if !index.is_file() {
        bail!(
            "frontend build is missing index.html at {}; check frontend.build_dir and rebuild/sync frontend artifacts",
            index.display()
        );
    }
    Ok(())
}

fn preflight_deploy_ports(server_config: &ServerConfig) -> Result<()> {
    let checks = vec![
        (
            "frontend",
            server_config.frontend.host.clone(),
            server_config.frontend.port,
        ),
        (
            "api",
            server_config.api_host.to_string(),
            server_config.api_port,
        ),
    ];

    let mut failures = Vec::new();
    for (label, host, port) in checks {
        if let Err(err) = TcpListener::bind((host.as_str(), port)) {
            if err.kind() == std::io::ErrorKind::AddrInUse {
                failures.push(format_port_in_use(label, &host, port));
            } else {
                failures.push(format!(
                    "{label} bind check failed for {host}:{port}: {err}"
                ));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("deploy port preflight failed:\n\n{}", failures.join("\n\n"));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortHolder {
    pid: u32,
    command: Option<String>,
}

fn format_port_in_use(label: &str, host: &str, port: u16) -> String {
    let holders = find_port_holders(port);
    format_port_in_use_with_holders(label, host, port, holders)
}

fn format_port_in_use_with_holders(
    label: &str,
    host: &str,
    port: u16,
    holders: Vec<PortHolder>,
) -> String {
    if holders.is_empty() {
        return format!(
            "{label} port {host}:{port} is already in use.\n\
             Inspect the listener with:\n  ss -ltnp 'sport = :{port}'"
        );
    }

    let mut lines = vec![format!("{label} port {host}:{port} is already in use by:")];
    for holder in holders {
        match holder.command {
            Some(command) => {
                lines.push(format!("  {command} (pid {})", holder.pid));
            }
            None => {
                lines.push(format!("  pid {}", holder.pid));
            }
        }
        lines.push(format!("  stop it with: kill -TERM {}", holder.pid));
        lines.push(format!("  force it if needed: kill -KILL {}", holder.pid));
    }
    lines.push(format!(
        "Inspect listeners with: ss -ltnp 'sport = :{port}'"
    ));
    lines.join("\n")
}

fn find_port_holders(port: u16) -> Vec<PortHolder> {
    let filter = format!("sport = :{port}");
    let Ok(output) = Command::new("ss").args(["-ltnp", &filter]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_ss_listeners(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ss_listeners(output: &str) -> Vec<PortHolder> {
    let mut holders = BTreeMap::new();
    for line in output.lines() {
        for holder in parse_ss_listener_line(line) {
            holders.entry(holder.pid).or_insert(holder);
        }
    }
    holders.into_values().collect()
}

fn parse_ss_listener_line(line: &str) -> Vec<PortHolder> {
    line.split("(\"")
        .skip(1)
        .filter_map(|entry| {
            let quote_end = entry.find('"')?;
            let command = entry[..quote_end].to_string();
            let pid = parse_ss_pid(entry)?;
            Some(PortHolder {
                pid,
                command: Some(command),
            })
        })
        .collect()
}

fn parse_ss_pid(entry: &str) -> Option<u32> {
    let pid_start = entry.find("pid=")? + "pid=".len();
    let digits: String = entry[pid_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn apply_port_offset(server_config: &mut ServerConfig, port_offset: u16) -> Result<()> {
    if port_offset == 0 {
        return Ok(());
    }

    server_config.frontend.port =
        checked_add_port(server_config.frontend.port, port_offset, "frontend port")?;
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

    Ok(())
}

async fn cleanup_deploy(
    server_config: &ServerConfig,
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
                        server_config.cleanup.sampler_drain_timeout_seconds,
                    ),
                    node_stop_timeout: Duration::from_secs(
                        server_config.cleanup.node_stop_timeout_seconds,
                    ),
                    poll_interval: Duration::from_millis(server_config.cleanup.poll_interval_ms),
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

    if server_config.database.ensure_started {
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
    fn new(frontend_port: u16, runtime: &RuntimeContext) -> Self {
        let root = runtime.deploy_tmp_dir(frontend_port);
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

fn start_backend(server_config: &ServerConfig, runtime: &RuntimeContext) -> Result<Child> {
    let binary = std::env::current_exe().context("failed to resolve current executable path")?;
    let mut command = Command::new(&binary);
    command.args(runtime.runtime_cli_args());
    command
        .arg("server")
        .arg("--server-config")
        .arg(&server_config.server_config_path);
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

fn start_nginx(paths: &DeployRuntimePaths) -> Result<Child> {
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
    server_config: &ServerConfig,
    frontend_port: u16,
    paths: &DeployRuntimePaths,
) -> Result<()> {
    let backend = server_config.bind_addr();
    let access_log = if server_config.frontend.access_log {
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
        server_name _;\n\
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
        listen_host = server_config.frontend.host,
        listen_port = frontend_port,
        static_dir = server_config.frontend.build_dir,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ss_listener_processes() {
        let output = r#"State  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess
LISTEN 0      511        127.0.0.1:8080      0.0.0.0:*    users:(("nginx",pid=419163,fd=5),("nginx",pid=419164,fd=5))
LISTEN 0      128        127.0.0.1:4000      0.0.0.0:*    users:(("gammaboard",pid=42,fd=9))
"#;

        let holders = parse_ss_listeners(output);

        assert_eq!(
            holders,
            vec![
                PortHolder {
                    pid: 42,
                    command: Some("gammaboard".to_string()),
                },
                PortHolder {
                    pid: 419163,
                    command: Some("nginx".to_string()),
                },
                PortHolder {
                    pid: 419164,
                    command: Some("nginx".to_string()),
                },
            ]
        );
    }

    #[test]
    fn port_in_use_message_shows_manual_inspection_when_pid_is_unknown() {
        let message = format_port_in_use_with_holders("frontend", "127.0.0.1", 8080, Vec::new());

        assert!(message.contains("frontend port 127.0.0.1:8080 is already in use"));
        assert!(message.contains("ss -ltnp 'sport = :8080'"));
    }

    #[test]
    fn port_in_use_message_shows_kill_command_when_pid_is_known() {
        let message = format_port_in_use_with_holders(
            "frontend",
            "127.0.0.1",
            8080,
            vec![PortHolder {
                pid: 419163,
                command: Some("nginx".to_string()),
            }],
        );

        assert!(message.contains("nginx (pid 419163)"));
        assert!(message.contains("stop it with: kill -TERM 419163"));
        assert!(message.contains("force it if needed: kill -KILL 419163"));
    }
}
