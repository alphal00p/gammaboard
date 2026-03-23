use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use gammaboard::config::{CliConfig, LocalPostgresConfig};
use std::{
    fs::{self, File},
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
    Init,
    Start,
    Create,
    Stop,
    Reset,
    DumpSql,
}

pub fn run_db_command(args: DbArgs, config: &CliConfig) -> Result<()> {
    let local = &config.local_postgres;
    match args.command {
        DbCommand::Init => init_db(local, &config.database.url),
        DbCommand::Start => start_db(local, &config.database.url),
        DbCommand::Create => create_db(local, &config.database.url),
        DbCommand::Stop => stop_db(local),
        DbCommand::Reset => reset_db(local, &config.database.url),
        DbCommand::DumpSql => dump_db_sql(local, &config.database.url),
    }
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

fn start_db(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
    ensure_parent_dir(&local.log_file)?;
    fs::create_dir_all(&local.socket_dir)
        .with_context(|| format!("failed to create socket dir {}", local.socket_dir))?;
    run_command(
        Command::new("pg_ctl")
            .arg("-D")
            .arg(&local.data_dir)
            .arg("-l")
            .arg(&local.log_file)
            .arg("-o")
            .arg(format!("-k {} -p {}", local.socket_dir, connection.port))
            .arg("start"),
        "pg_ctl start",
    )
}

fn create_db(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
    let status = Command::new("createdb")
        .arg("-h")
        .arg(&local.socket_dir)
        .arg("-p")
        .arg(connection.port.to_string())
        .arg("-U")
        .arg(&connection.user)
        .arg(&connection.database)
        .status()
        .context("failed to spawn createdb")?;
    if !status.success() {
        eprintln!(
            "createdb exited with status {status}; continuing to migrations in case the database already exists"
        );
    }
    run_command(
        Command::new("sqlx")
            .arg("migrate")
            .arg("run")
            .arg("--database-url")
            .arg(database_url),
        "sqlx migrate run",
    )
}

fn stop_db(local: &LocalPostgresConfig) -> Result<()> {
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

fn reset_db(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    stop_db(local)?;
    remove_path_if_exists(&local.data_dir)?;
    remove_path_if_exists(&local.socket_dir)?;
    init_db(local, database_url)?;
    start_db(local, database_url)?;
    create_db(local, database_url)
}

fn dump_db_sql(local: &LocalPostgresConfig, database_url: &str) -> Result<()> {
    let connection = LocalDbConnection::from_url(database_url)?;
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
        .arg(&local.socket_dir)
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

fn remove_path_if_exists(path: &str) -> Result<()> {
    let path = Path::new(path);
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
    }
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
