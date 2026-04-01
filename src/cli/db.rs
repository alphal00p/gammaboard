use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use gammaboard::config::{CliConfig, LocalPostgresConfig};
use std::{
    fs::{self, File},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

#[derive(Debug, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    Status,
    Start,
    Stop,
    Delete {
        #[arg(short = 'y', long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
    DumpSql,
}

pub fn run_db_command(args: DbArgs, config: &CliConfig) -> Result<()> {
    let local = &config.local_postgres;
    match args.command {
        DbCommand::Status => status_db(local, &config.database.url),
        DbCommand::Start => start_db(local, &config.database.url),
        DbCommand::Stop => stop_db(local),
        DbCommand::Delete { yes } => delete_db(local, yes),
        DbCommand::DumpSql => dump_db_sql(local, &config.database.url),
    }
}

fn status_db(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let initialized = is_cluster_initialized(local);
    let running = if initialized {
        is_db_running(local)?
    } else {
        false
    };
    let db_exists = if running {
        Some(database_exists(local, database_url)?)
    } else {
        None
    };
    let healthy = if running {
        Some(database_connection_healthy(local, database_url)?)
    } else {
        None
    };

    println!("initialized: {}", yes_no(initialized));
    println!("running: {}", yes_no(running));
    match db_exists {
        Some(value) => println!("database_exists: {}", yes_no(value)),
        None => println!("database_exists: unknown (postgres not running)"),
    }
    match healthy {
        Some(value) => println!("connection_healthy: {}", yes_no(value)),
        None => println!("connection_healthy: unknown (postgres not running)"),
    }
    Ok(())
}

fn init_db(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
    run_command(
        Command::new("initdb")
            .arg("-D")
            .arg(&local.data_dir)
            .arg("--username")
            .arg(&connection.user)
            .arg("--auth=trust"),
        "initdb",
    )?;
    fs::create_dir_all(&local.socket_dir)
        .with_context(|| format!("failed to create socket dir {}", local.socket_dir))?;
    Ok(())
}

fn start_postgres(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
    ensure_parent_dir(&local.log_file)?;
    let socket_dir = ensure_absolute_dir(&local.socket_dir)?;
    run_command(
        Command::new("pg_ctl")
            .arg("-D")
            .arg(&local.data_dir)
            .arg("-l")
            .arg(&local.log_file)
            .arg("-o")
            .arg(format!(
                "-k {} -p {} -c max_connections={}",
                socket_dir.display(),
                connection.port,
                local.max_connections
            ))
            .arg("start"),
        "pg_ctl start",
    )
}

fn ensure_database_and_migrations(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
    if !database_exists(local, database_url)? {
        let socket_dir = ensure_absolute_dir(&local.socket_dir)?;
        println!("creating database '{}'", connection.database);
        run_command(
            Command::new("createdb")
                .arg("-h")
                .arg(&socket_dir)
                .arg("-p")
                .arg(connection.port.to_string())
                .arg("-U")
                .arg(&connection.user)
                .arg(&connection.database),
            "createdb",
        )?;
    } else {
        println!("database '{}' already exists", connection.database);
    }
    println!("applying migrations");
    run_command(
        Command::new("sqlx")
            .arg("migrate")
            .arg("run")
            .arg("--database-url")
            .arg(database_url),
        "sqlx migrate run",
    )
}

fn start_db(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    if !is_cluster_initialized(local) {
        println!("postgres cluster not initialized; creating new cluster");
        init_db(local, database_url)?;
    } else {
        println!("postgres cluster already initialized");
    }

    if !is_db_running(local)? {
        println!("postgres is stopped; starting");
        start_postgres(local, database_url)?;
    } else {
        println!("postgres already running");
    }

    ensure_database_and_migrations(local, database_url)
}

fn stop_db(local: &LocalPostgresConfig) -> Result<()> {
    if !Path::new(&local.data_dir).exists() {
        println!("postgres already stopped (data directory missing)");
        return Ok(());
    }
    if !is_db_running(local)? {
        println!("postgres already stopped");
        return Ok(());
    }
    println!("stopping postgres");
    let status = Command::new("pg_ctl")
        .arg("-D")
        .arg(&local.data_dir)
        .arg("stop")
        .status()
        .context("failed to spawn pg_ctl stop")?;
    if status.success() {
        Ok(())
    } else {
        eprintln!("pg_ctl stop exited with status {status}; ignoring");
        Ok(())
    }
}

fn delete_db(local: &LocalPostgresConfig, assume_yes: bool) -> Result<()> {
    confirm_delete(local, assume_yes)?;
    stop_db(local)?;
    let removed_data = remove_path_if_exists(&local.data_dir)?;
    let removed_socket = remove_path_if_exists(&local.socket_dir)?;
    if !removed_data && !removed_socket {
        println!("nothing to delete (no local postgres state found)");
    } else {
        if removed_data {
            println!("deleted {}", local.data_dir);
        }
        if removed_socket {
            println!("deleted {}", local.socket_dir);
        }
    }
    Ok(())
}

fn confirm_delete(local: &LocalPostgresConfig, assume_yes: bool) -> Result<()> {
    if assume_yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        bail!("db delete requires --yes in non-interactive mode");
    }

    print!(
        "Delete local postgres state? This deletes '{}' and '{}'. [y/N]: ",
        local.data_dir, local.socket_dir
    );
    io::stdout().flush().context("failed to flush stdout")?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("failed reading confirmation response")?;
    let answer = line.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        Ok(())
    } else {
        bail!("aborted delete");
    }
}

fn dump_db_sql(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
    let socket_dir = ensure_absolute_dir(&local.socket_dir)?;
    fs::create_dir_all("dump").context("failed to create dump directory")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock moved backwards")?
        .as_secs();
    let output_path = PathBuf::from(format!("dump/db-{timestamp}.sql"));
    let output = File::create(&output_path)
        .with_context(|| format!("failed to create {}", output_path.display()))?;
    let status = Command::new("pg_dump")
        .arg("-h")
        .arg(&socket_dir)
        .arg("-p")
        .arg(connection.port.to_string())
        .arg("-U")
        .arg(&connection.user)
        .arg(&connection.database)
        .stdout(Stdio::from(output))
        .status()
        .context("failed to spawn pg_dump")?;
    if !status.success() {
        bail!("pg_dump failed with status {status}");
    }
    println!("{}", output_path.display());
    Ok(())
}

fn is_cluster_initialized(local: &LocalPostgresConfig) -> bool {
    Path::new(&local.data_dir).join("PG_VERSION").is_file()
}

fn is_db_running(local: &LocalPostgresConfig) -> Result<bool> {
    let output = Command::new("pg_ctl")
        .arg("-D")
        .arg(&local.data_dir)
        .arg("status")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to spawn pg_ctl status")?;
    Ok(output.success())
}

fn database_exists(local: &LocalPostgresConfig, database_url: &str) -> Result<bool> {
    let connection = LocalDbConnection::from_url(database_url)?;
    let query = format!(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = '{}');",
        escape_sql_literal(&connection.database)
    );
    let output = run_psql_query(local, &connection, "template1", &query)?;
    match output.trim() {
        "t" => Ok(true),
        "f" => Ok(false),
        other => bail!("unexpected database_exists query output: {other}"),
    }
}

fn database_connection_healthy(local: &LocalPostgresConfig, database_url: &str) -> Result<bool> {
    let connection = LocalDbConnection::from_url(database_url)?;
    let output = run_psql_query(local, &connection, &connection.database, "SELECT 1;")?;
    Ok(output.trim() == "1")
}

fn run_psql_query(
    local: &LocalPostgresConfig,
    connection: &LocalDbConnection,
    database: &str,
    sql: &str,
) -> Result<String> {
    let socket_dir = ensure_absolute_dir(&local.socket_dir)?;
    let output = Command::new("psql")
        .arg("-h")
        .arg(&socket_dir)
        .arg("-p")
        .arg(connection.port.to_string())
        .arg("-U")
        .arg(&connection.user)
        .arg("-d")
        .arg(database)
        .arg("-t")
        .arg("-A")
        .arg("-c")
        .arg(sql)
        .output()
        .context("failed to spawn psql")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("psql query failed with status {}: {stderr}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn run_command(command: &mut Command, label: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to spawn {label}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{label} failed with status {status}")
    }
}

fn ensure_parent_dir(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn ensure_absolute_dir(path: &str) -> Result<PathBuf> {
    fs::create_dir_all(path).with_context(|| format!("failed to create directory {path}"))?;
    std::env::current_dir()
        .context("failed to resolve current working directory")?
        .join(path)
        .canonicalize()
        .with_context(|| format!("failed to resolve absolute path for {path}"))
}

fn remove_path_if_exists(path: &str) -> Result<bool> {
    let path = Path::new(path);
    if !path.exists() {
        return Ok(false);
    }
    if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(true)
}

struct LocalDbConnection {
    user: String,
    database: String,
    port: u16,
}

impl LocalDbConnection {
    fn from_url(database_url: &str) -> Result<Self> {
        let url = Url::parse(database_url).with_context(|| {
            format!("failed to parse database.url from cli config: {database_url}")
        })?;
        let user = url.username().trim().to_string();
        if user.is_empty() {
            return Err(anyhow!("database.url must include a username"));
        }
        let database = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("database.url must include a database name path"))?
            .to_string();
        let port = url.port().ok_or_else(|| {
            anyhow!("database.url must include an explicit port for local postgres commands")
        })?;
        Ok(Self {
            user,
            database,
            port,
        })
    }
}
