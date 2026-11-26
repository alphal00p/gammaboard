use crate::{
    Client, connection,
    protocol::{ExportKind, LaunchSource, Request, TaskAction, TemplateAction, WorkerFilter},
};
use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Parser)]
#[command(name = "gb", version, about = "GammaBoard management client")]
pub struct Cli {
    /// Deployment or connection file; local operations require a full deployment file.
    #[arg(long, global = true)]
    pub instance: Option<PathBuf>,
    #[arg(long, global = true, env = "GAMMABOARD_ENDPOINT")]
    pub endpoint: Option<String>,
    #[arg(long, global = true, env = "GAMMABOARD_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub allow_local_processes: bool,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    Status,
    Worker {
        #[command(subcommand)]
        command: Worker,
    },
    /// Inspect client settings or reload selected service settings.
    Config {
        #[command(subcommand)]
        command: Config,
    },
    /// Download a database backup. Health is in status; offline maintenance is under local db.
    Db {
        #[command(subcommand)]
        command: Db,
    },
    Template {
        #[command(subcommand)]
        command: Template,
    },
    Logs(Logs),
    Auth {
        #[command(subcommand)]
        command: Auth,
    },
    Control {
        #[command(subcommand)]
        command: Control,
    },
    /// Inspect this client build; --server queries the service instead.
    BuildInfo {
        #[arg(long)]
        server: bool,
    },
    /// Offline operations on this host; requires the service executable.
    Local {
        #[command(subcommand)]
        command: Local,
    },
    Completion {
        shell: clap_complete::Shell,
    },
    Stop {
        #[arg(long)]
        wait: bool,
        #[arg(long,value_parser=parse_duration)]
        timeout: Option<Duration>,
    },
    Run {
        #[command(subcommand)]
        command: Run,
    },
    /// Bootstrap local services using the separately installed service executable.
    Start(Start),
    /// Restart the whole local deployment, not a remote endpoint or component.
    Restart(StartupOptions),
}
#[derive(Debug, Args)]
pub struct Start {
    #[arg(value_parser=["db","server"])]
    pub component: Option<String>,
    #[command(flatten)]
    pub options: StartupOptions,
}
impl std::ops::Deref for Start {
    type Target = StartupOptions;
    fn deref(&self) -> &Self::Target {
        &self.options
    }
}
#[derive(Debug, Args)]
pub struct StartupOptions {
    /// Additional exact browser origins for this invocation (repeatable).
    #[arg(long = "allowed-origin")]
    pub allowed_origins: Vec<String>,
    /// Submit requests for this many additional workers using instance policy.
    #[arg(long,value_parser=clap::value_parser!(u32).range(1..))]
    pub workers: Option<u32>,
    #[arg(long)]
    pub restore_workers: bool,
    /// Also wait for the submitted worker requests to be fulfilled.
    #[arg(long, conflicts_with = "foreground")]
    pub wait: bool,
    #[arg(long)]
    pub foreground: bool,
    #[arg(long)]
    pub api_only: bool,
    #[arg(long,default_value="60s",value_parser=parse_duration)]
    pub timeout: Duration,
}
impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            workers: None,
            restore_workers: false,
            wait: false,
            foreground: false,
            api_only: false,
            timeout: Duration::from_secs(60),
        }
    }
}
#[derive(Debug, Args)]
pub struct WaitOptions {
    #[arg(long)]
    wait: bool,
    #[arg(long,requires="wait",value_parser=parse_duration)]
    timeout: Option<Duration>,
}
impl WaitOptions {
    fn timeout(&self) -> Option<Duration> {
        self.wait
            .then(|| self.timeout.unwrap_or(Duration::from_secs(60)))
    }
}
#[derive(Debug, Subcommand)]
pub enum Run {
    Inspect {
        run: String,
        #[arg(long, default_value_t = 100)]
        log_limit: i64,
    },
    Validate {
        file: PathBuf,
        #[arg(long)]
        base_dir: Option<String>,
    },
    Duplicate {
        source: String,
        name: String,
    },
    Export {
        run: String,
        #[arg(long, value_enum)]
        kind: ExportKind,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Pause(Selection),
    Remove {
        #[command(flatten)]
        selection: Selection,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Wait {
        run: String,
        #[arg(long,default_value="completed",value_parser=["ready","idle","completed"])]
        until: String,
        #[arg(long, default_value_t = 1)]
        evaluators: usize,
        #[arg(long,default_value="60s",value_parser=parse_duration)]
        timeout: Duration,
        #[arg(long,default_value="10s",value_parser=parse_duration)]
        max_age: Duration,
    },
    Performance {
        run: String,
        #[arg(long,value_parser=parse_duration,conflicts_with="since")]
        duration: Option<Duration>,
        #[arg(long,default_value="1s",value_parser=parse_duration)]
        interval: Duration,
        #[arg(long,default_value="10s",value_parser=parse_duration)]
        max_age: Duration,
        #[arg(long)]
        since: Option<String>,
        #[arg(long, requires = "since")]
        until: Option<String>,
        #[arg(long, default_value_t = 1000)]
        limit: i64,
    },
    Task {
        #[command(subcommand)]
        command: Task,
    },
    Create {
        file: PathBuf,
        /// Reuse this key to safely recover creation after a lost response.
        #[arg(long)]
        request_id: Option<String>,
        /// Wait for compatible workers; 0 makes a single start attempt.
        #[arg(long, requires="start", default_value="10s", value_parser=parse_worker_timeout)]
        timeout: Duration,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        start: bool,
        #[arg(long, requires = "start")]
        max_evaluators: Option<usize>,
        #[arg(long, help = "Absolute export path on the deployment")]
        export_path: Option<String>,
        #[arg(long, help = "Deployment directory for relative resource references")]
        base_dir: Option<String>,
    },
    List {
        name: Option<String>,
    },
    #[command(alias = "resume")]
    Start {
        run: String,
        /// Wait for compatible workers; 0 makes a single start attempt.
        #[arg(long, default_value="10s", value_parser=parse_worker_timeout)]
        timeout: Duration,
        #[arg(long)]
        max_evaluators: Option<usize>,
    },
    /// Import a result bundle as an archived run that cannot execute.
    ImportResults {
        file: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    Results {
        run: String,
        #[arg(long)]
        task: Option<i64>,
        #[arg(long, hide = true)]
        output: Option<PathBuf>,
    },
}
#[derive(Debug, Args)]
pub struct Selection {
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    pub runs: Vec<String>,
    #[arg(long)]
    pub all: bool,
}
#[derive(Debug, Subcommand)]
pub enum Task {
    List {
        run: String,
    },
    #[command(hide = true)]
    Inspect {
        run: String,
        task_id: i64,
    },
    Export {
        run: String,
        task_id: i64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Append {
        run: String,
        file: PathBuf,
    },
    Duplicate {
        run: String,
        task_id: i64,
    },
    Edit {
        run: String,
        task_id: i64,
        file: PathBuf,
    },
    Tune {
        run: String,
        task_id: i64,
        file: PathBuf,
    },
    Remove {
        run: String,
        task_id: i64,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum Worker {
    List {
        name: Option<String>,
        #[arg(long, value_enum, default_value = "live")]
        state: WorkerFilter,
        #[arg(long, hide = true, conflicts_with = "state")]
        offline: bool,
    },
    Inspect {
        name: String,
    },
    /// Submit a launch request using the instance's backend policy.
    Launch {
        #[arg(long,conflicts_with="config",required_unless_present="config",value_parser=clap::value_parser!(u32).range(1..))]
        count: Option<u32>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(flatten)]
        waiting: WaitOptions,
    },
    /// Submit a request for an offline worker's saved recipe.
    Start {
        name: String,
        #[command(flatten)]
        waiting: WaitOptions,
    },
    /// Submit requests for workers saved during shutdown.
    Restore {
        #[command(flatten)]
        waiting: WaitOptions,
    },
    CancelRestoration {
        name: String,
    },
    Stop {
        #[arg(required_unless_present_any=["all","idle"],conflicts_with_all=["all","idle"])]
        names: Vec<String>,
        #[arg(long, conflicts_with = "idle")]
        all: bool,
        #[arg(long)]
        idle: bool,
    },
    Assign {
        name: String,
        run: String,
        #[arg(long,value_parser=["evaluator","sampler_aggregator"])]
        role: String,
    },
    Unassign {
        name: String,
    },
    #[command(name = "requests", alias = "launch-request")]
    LaunchRequest {
        #[command(subcommand)]
        command: LaunchRequest,
    },
}
#[derive(Debug, Subcommand)]
pub enum LaunchRequest {
    List,
    Inspect {
        id: i64,
    },
    Wait {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long,default_value="60s",value_parser=parse_duration)]
        timeout: Duration,
    },
    /// Cancel a queued request before a controller claims it.
    Cancel {
        id: i64,
    },
}
#[derive(Debug, Subcommand)]
pub enum Config {
    Show,
    Validate,
    /// Reload browser origins from the service's deployment file without restarting.
    Reload {
        #[arg(long, required = true)]
        origins: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum Db {
    Backup {
        #[arg(long, short)]
        output: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
pub enum Local {
    Db {
        #[command(subcommand)]
        command: LocalDb,
    },
    Config {
        #[command(subcommand)]
        command: LocalConfig,
    },
    Run {
        #[command(subcommand)]
        command: LocalRun,
    },
    #[command(hide = true)]
    CheckConfig,
    /// Validate a run on this host; descriptor discovery can execute evaluators.
    #[command(hide = true)]
    CheckRun {
        file: PathBuf,
        #[arg(long)]
        probe: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum LocalConfig {
    Validate,
}
#[derive(Debug, Subcommand)]
pub enum LocalRun {
    /// Validate on this host. Descriptor discovery may initialize external providers.
    Validate {
        file: PathBuf,
        #[arg(long)]
        probe: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum LocalDb {
    Status,
    Backup {
        #[arg(long, short)]
        output: PathBuf,
    },
    Create,
    Migrate,
    Stop,
    Delete {
        #[arg(long)]
        yes: bool,
    },
    Purge {
        #[arg(long)]
        yes: bool,
    },
    Reset {
        #[arg(long)]
        yes: bool,
    },
    Restore {
        backup: PathBuf,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum Template {
    List {
        #[arg(value_parser=["run","task","worker"])]
        kind: String,
    },
    Show {
        #[arg(value_parser=["run","task","worker"])]
        kind: String,
        name: String,
    },
    Save {
        #[arg(value_parser=["run","task","worker"])]
        kind: String,
        name: String,
        file: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    Remove {
        #[arg(value_parser=["run","task","worker"])]
        kind: String,
        name: String,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Debug, Args)]
pub struct Logs {
    #[arg(long,conflicts_with_all=["worker","service"])]
    run: Option<String>,
    #[arg(long, conflicts_with = "service")]
    worker: Option<String>,
    #[arg(long)]
    service: Option<String>,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long)]
    before_id: Option<i64>,
    #[arg(long)]
    follow: bool,
    #[arg(long, help = "One JSON object per log entry")]
    jsonl: bool,
}
#[derive(Debug, Subcommand)]
pub enum Auth {
    Login {
        #[arg(long)]
        password_stdin: bool,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
pub enum Control {
    AcknowledgeCleanup { request_id: String },
}

pub fn parse_duration(raw: &str) -> Result<Duration, String> {
    parse_duration_with_min(raw, 0.001)
}

fn parse_worker_timeout(raw: &str) -> Result<Duration, String> {
    parse_duration_with_min(raw, 0.0)
}

fn parse_duration_with_min(raw: &str, minimum: f64) -> Result<Duration, String> {
    let (number, scale) = if let Some(n) = raw.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = raw.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = raw.strip_suffix('m') {
        (n, 60.0)
    } else {
        (raw, 1.0)
    };
    let seconds = number
        .parse::<f64>()
        .map_err(|_| "expected a duration such as 250ms, 30s, or 2m")?
        * scale;
    if !seconds.is_finite() || !(minimum..=86400.0).contains(&seconds) {
        return Err(format!(
            "duration must be between {}ms and 24h",
            minimum * 1000.0
        ));
    }
    Ok(Duration::from_secs_f64(seconds))
}
impl Cli {
    pub fn connection(&self) -> Result<crate::ConnectionConfig> {
        let mut config = if let Some(path) = &self.instance {
            connection(path)?
        } else if self.endpoint.is_none() {
            connection(Path::new("gammaboard.toml"))?
        } else {
            crate::ConnectionConfig::default()
        };
        if let Some(endpoint) = &self.endpoint {
            config.endpoint = endpoint.clone();
        }
        if let Some(path) = &self.token_file {
            config.token_file = Some(path.clone());
        }
        Ok(config)
    }
}
pub fn is_local(command: &Command) -> bool {
    matches!(
        command,
        Command::Start(_) | Command::Restart(_) | Command::Local { .. }
    )
}
pub async fn execute(cli: Cli) -> Result<()> {
    if is_local(&cli.command) {
        return bootstrap(&cli);
    }
    if let Command::Completion { shell } = &cli.command {
        ensure!(!cli.json, "completion emits shell source; omit --json");
        use clap::CommandFactory;
        clap_complete::generate(*shell, &mut Cli::command(), "gb", &mut std::io::stdout());
        return Ok(());
    }
    ensure!(
        !cli.allow_local_processes,
        "--allow-local-processes applies only to start, restart, and local administration on this host"
    );
    if let Command::BuildInfo { server: false } = cli.command {
        return print_value(
            &serde_json::json!({"binary":"gb","version":env!("CARGO_PKG_VERSION"),"management_api":1}),
            cli.json,
        );
    }
    let connection = cli.connection()?;
    if let Command::Config { command } = &cli.command
        && !matches!(command, Config::Reload { .. })
    {
        Client::new(&connection.endpoint, None)?;
        return match command {
            Config::Reload { .. } => unreachable!(),
            Config::Show => print_value(&serde_json::to_value(&connection)?, cli.json),
            Config::Validate => {
                connection.validate()?;
                print_value(
                    &serde_json::json!({"valid":true,"endpoint":connection.endpoint}),
                    cli.json,
                )
            }
        };
    }
    let client = connection.client(!matches!(cli.command, Command::Auth { .. }))?;
    let mut output = None;
    let mut raw = false;
    let mut launch_timeout = None;
    let mut worker_wait = None;
    let request = match cli.command {
        Command::Status => Request::Status {},
        Command::Config {
            command: Config::Reload { .. },
        } => Request::ReloadOrigins {},
        Command::BuildInfo { server: true } => Request::BuildInfo {},
        Command::Control {
            command: Control::AcknowledgeCleanup { request_id },
        } => Request::AcknowledgeCleanup { request_id },
        Command::Db {
            command: Db::Backup { output },
        } => {
            client.download("/api/v1/database/backup", &output).await?;
            return print_value(&serde_json::json!({"output":output}), cli.json);
        }
        Command::Auth {
            command:
                Auth::Login {
                    password_stdin,
                    output,
                },
        } => {
            ensure!(
                password_stdin,
                "supply --password-stdin; passwords are never command arguments"
            );
            use std::io::Read;
            let mut password = String::new();
            std::io::stdin().read_to_string(&mut password)?;
            let token = client
                .login(password.trim_end_matches(['\r', '\n']))
                .await?;
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            use std::io::Write;
            writeln!(file, "{token}")?;
            file.persist_noclobber(&output).map_err(|e| e.error)?;
            return print_value(&serde_json::json!({"token_file":output}), cli.json);
        }
        Command::Logs(args) => return logs(&client, args, cli.json).await,
        Command::Stop { wait, timeout } => {
            let started = std::time::Instant::now();
            let accepted = client
                .request_with_timeout(
                    &Request::Stop {
                        wait,
                        timeout_ms: timeout.map(ms),
                    },
                    timeout
                        .unwrap_or(Duration::from_secs(30))
                        .min(Duration::from_secs(30)),
                )
                .await?;
            let id = accepted["stop_id"]
                .as_str()
                .context("service returned no shutdown ID")?;
            return print_value(
                &client
                    .wait_for_stop(id, timeout.map(|t| t.saturating_sub(started.elapsed())))
                    .await?,
                cli.json,
            );
        }
        Command::Run { command } => match command {
            Run::Create {
                file,
                request_id,
                timeout,
                name,
                start,
                max_evaluators,
                export_path,
                base_dir,
            } => {
                if let Some(path) = &export_path {
                    ensure!(
                        Path::new(path).is_absolute(),
                        "--export-path must be absolute on the deployment"
                    );
                }
                worker_wait = (start && !timeout.is_zero()).then_some(timeout);
                Request::RunCreate {
                    request_id,
                    duplicate: false,
                    toml: std::fs::read_to_string(file)?,
                    base_dir,
                    name,
                    start,
                    max_evaluators,
                    export_path,
                }
            }
            Run::List { name } => Request::RunList { name },
            Run::Start {
                run,
                max_evaluators,
                timeout,
            } => {
                worker_wait = (!timeout.is_zero()).then_some(timeout);
                Request::RunStart {
                    run,
                    max_evaluators,
                }
            }
            Run::ImportResults { file, name } => Request::RunImportResults {
                bundle: serde_json::from_slice(&std::fs::read(file)?)?,
                name,
            },
            Run::Results {
                run,
                task,
                output: path,
            } => {
                output = path;
                Request::RunResults { run, task }
            }
            Run::Inspect { run, log_limit } => Request::RunInspect { run, log_limit },
            Run::Validate { file, base_dir } => Request::RunValidate {
                toml: std::fs::read_to_string(file)?,
                base_dir,
            },
            Run::Duplicate { source, name } => Request::RunDuplicate { source, name },
            Run::Export {
                run,
                kind,
                output: path,
            } => {
                output = path;
                raw = kind == ExportKind::Definition;
                Request::RunExport { run, kind }
            }
            Run::Pause(selection) => Request::RunPause {
                runs: selection.runs,
                all: selection.all,
            },
            Run::Remove {
                selection,
                yes,
                dry_run,
            } => {
                ensure!(yes || dry_run, "run remove requires --yes (or --dry-run)");
                Request::RunRemove {
                    runs: selection.runs,
                    all: selection.all,
                    dry_run,
                }
            }
            Run::Wait {
                run,
                until,
                evaluators,
                timeout,
                max_age,
            } => {
                let request = Request::RunReadiness {
                    run,
                    until,
                    evaluators,
                    max_age_ms: ms(max_age),
                };
                let deadline = std::time::Instant::now() + timeout;
                loop {
                    let remaining = deadline
                        .checked_duration_since(std::time::Instant::now())
                        .context("timed out waiting for run; work is unchanged")?;
                    let value = client
                        .request_with_timeout(&request, remaining.min(Duration::from_secs(30)))
                        .await?;
                    if value["ready"] == true {
                        return print_value(&value["snapshot"], cli.json);
                    }
                    ensure!(
                        std::time::Instant::now() < deadline,
                        "timed out waiting for run; work is unchanged"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            Run::Performance {
                run,
                duration,
                interval,
                max_age,
                since,
                until,
                limit,
            } => Request::RunPerformance {
                run,
                duration_ms: duration.map(ms),
                interval_ms: ms(interval),
                max_age_ms: ms(max_age),
                since,
                until,
                limit,
            },
            Run::Task {
                command: Task::Inspect { run, task_id },
            } => Request::RunResults {
                run,
                task: Some(task_id),
            },
            Run::Task { command } => {
                let (run, action) = match command {
                    Task::List { run } => (run, TaskAction::List {}),
                    Task::Inspect { .. } => unreachable!(),
                    Task::Export {
                        run,
                        task_id,
                        output: path,
                    } => {
                        output = path;
                        raw = true;
                        (run, TaskAction::Export { id: task_id })
                    }
                    Task::Append { run, file } => (
                        run,
                        TaskAction::Append {
                            duplicate: false,
                            toml: std::fs::read_to_string(file)?,
                        },
                    ),
                    Task::Duplicate { run, task_id } => {
                        (run, TaskAction::Duplicate { id: task_id })
                    }
                    Task::Edit { run, task_id, file } => (
                        run,
                        TaskAction::Edit {
                            expected_toml: None,
                            id: task_id,
                            toml: std::fs::read_to_string(file)?,
                        },
                    ),
                    Task::Tune { run, task_id, file } => (
                        run,
                        TaskAction::Tune {
                            id: task_id,
                            queue_tuning: Some(serde_json::to_value(
                                toml::from_str::<toml::Value>(&std::fs::read_to_string(file)?)?,
                            )?),
                        },
                    ),
                    Task::Remove { run, task_id, yes } => {
                        ensure!(yes, "task removal requires --yes");
                        (run, TaskAction::Remove { id: task_id })
                    }
                };
                Request::Task { run, action }
            }
        },
        Command::Worker { command } => match command {
            Worker::List {
                name,
                state,
                offline,
            } => Request::WorkerList {
                name,
                state: if offline {
                    WorkerFilter::Offline
                } else {
                    state
                },
                run: None,
            },
            Worker::Inspect { name } => Request::WorkerInspect { name },
            Worker::Launch {
                count,
                config,
                waiting,
            } => {
                launch_timeout = waiting.timeout();
                Request::WorkerLaunch {
                    source: LaunchSource::New {
                        count: count.map(|n| n as usize),
                        toml: config.map(std::fs::read_to_string).transpose()?,
                        max_start_failures: None,
                        name_prefix: None,
                        args: serde_json::json!({}),
                    },
                }
            }
            Worker::Start { name, waiting } => {
                launch_timeout = waiting.timeout();
                Request::WorkerLaunch {
                    source: LaunchSource::Saved {
                        name,
                        expected_uuid: None,
                        restore_only: false,
                    },
                }
            }
            Worker::Restore { waiting } => {
                launch_timeout = waiting.timeout();
                Request::WorkerLaunch {
                    source: LaunchSource::RestoreSaved {},
                }
            }
            Worker::CancelRestoration { name } => Request::WorkerCancelRestoration {
                name,
                expected_uuid: None,
            },
            Worker::Stop { names, all, idle } => Request::WorkerStop { names, all, idle },
            Worker::Assign { name, run, role } => Request::WorkerAssign { name, run, role },
            Worker::Unassign { name } => Request::WorkerUnassign {
                name: Some(name),
                all: false,
                expected_uuid: None,
            },
            Worker::LaunchRequest { command } => match command {
                LaunchRequest::List => Request::WorkerRequests { id: None },
                LaunchRequest::Inspect { id } => Request::WorkerRequests { id: Some(id) },
                LaunchRequest::Cancel { id } => Request::WorkerRequestCancel { id },
                LaunchRequest::Wait { ids, timeout } => {
                    return print_value(&client.wait_for_requests(&ids, timeout).await?, cli.json);
                }
            },
        },
        Command::Template { command } => {
            let (kind, action) = match command {
                Template::List { kind } => (kind, TemplateAction::List {}),
                Template::Show { kind, name } => {
                    raw = true;
                    (kind, TemplateAction::Show { name })
                }
                Template::Save {
                    kind,
                    name,
                    file,
                    yes,
                } => (
                    kind,
                    TemplateAction::Save {
                        name,
                        toml: std::fs::read_to_string(file)?,
                        overwrite: yes,
                    },
                ),
                Template::Remove { kind, name, yes } => {
                    ensure!(yes, "template removal requires --yes");
                    (kind, TemplateAction::Remove { name })
                }
            };
            Request::Template { kind, action }
        }
        Command::Start(_)
        | Command::Restart(_)
        | Command::Local { .. }
        | Command::Config {
            command: Config::Show | Config::Validate,
        }
        | Command::BuildInfo { server: false }
        | Command::Completion { .. } => unreachable!(),
    };
    let started = std::time::Instant::now();
    let mut value = if let Some(timeout) = launch_timeout {
        client
            .request_with_timeout(&request, timeout.min(Duration::from_secs(30)))
            .await?
    } else {
        client.request(&request).await?
    };
    if matches!(request, Request::RunRemove { dry_run: false, .. }) {
        for operation in value["operations"]
            .as_array_mut()
            .context("missing removal operations")?
        {
            while operation["status"] == "running" {
                tokio::time::sleep(Duration::from_millis(
                    operation["poll_after_ms"].as_u64().unwrap_or(1000),
                ))
                .await;
                let id = operation["operation_id"]
                    .as_str()
                    .context("missing removal ID")?
                    .to_owned();
                *operation = client
                    .request(&Request::RunRemovalStatus {
                        operation_id: id.clone(),
                    })
                    .await
                    .with_context(|| {
                        format!(
                            "removal {id} remains submitted; inspect its status before retrying"
                        )
                    })?;
            }
        }
        if value["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| operation["status"] != "completed")
        {
            print_value(&value, cli.json)?;
            return Err(crate::ChildExit(1).into());
        }
    }
    if matches!(
        request,
        Request::RunCreate { .. } | Request::RunStart { .. }
    ) {
        if let Some(timeout) = worker_wait {
            let id = value
                .get("created")
                .and_then(|v| v.get("run_id"))
                .or_else(|| value.get("run_id"))
                .context("start receipt has no run ID")?
                .to_string()
                .trim_matches('"')
                .to_owned();
            let cap = match &request {
                Request::RunCreate { max_evaluators, .. }
                | Request::RunStart { max_evaluators, .. } => *max_evaluators,
                _ => None,
            };
            while value["start"]["state"] == "blocked" {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    value["start"]["state"] = "timed_out".into();
                    break;
                }
                tokio::time::sleep(remaining.min(Duration::from_millis(250))).await;
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    value["start"]["state"] = "timed_out".into();
                    break;
                }
                match client
                    .request_with_timeout(
                        &Request::RunStart {
                            run: id.clone(),
                            max_evaluators: cap,
                        },
                        remaining.min(Duration::from_secs(30)),
                    )
                    .await
                {
                    Ok(receipt) => value["start"] = receipt["start"].clone(),
                    Err(error) => {
                        value["start"] = serde_json::json!({"state":"unknown","error":{"code":"start_outcome_unknown","retryable":false,"message":format!("{error:#}"),"reconcile":"Inspect this run before retrying; an in-flight start may have committed."}});
                        break;
                    }
                }
            }
        }
        let failed = !matches!(
            value["start"]["state"].as_str(),
            Some("started" | "not_requested")
        ) || value.get("export").is_some_and(|v| v["state"] == "failed");
        print_value(&value, cli.json)?;
        return if failed {
            Err(crate::ChildExit(1).into())
        } else {
            Ok(())
        };
    }
    if let Some(timeout) = launch_timeout {
        let ids: Vec<String> = serde_json::from_value(value["request_ids"].clone())?;
        client
            .wait_for_requests(&ids, timeout.saturating_sub(started.elapsed()))
            .await?;
    }
    if let Some(path) = output {
        let bytes = if raw {
            value["toml"]
                .as_str()
                .context("missing definition")?
                .as_bytes()
                .to_vec()
        } else {
            serde_json::to_vec_pretty(&value)?
        };
        write_output(&path, &bytes)?;
        print_value(&serde_json::json!({"output":path}), cli.json)?;
    } else if raw && !cli.json {
        print!("{}", value["toml"].as_str().context("missing definition")?);
    } else {
        print_value(&value, cli.json)?;
    }
    Ok(())
}
fn ms(value: Duration) -> u64 {
    value.as_millis() as u64
}
async fn logs(client: &Client, args: Logs, json: bool) -> Result<()> {
    ensure!(
        !args.follow || !json || args.jsonl,
        "use --jsonl for streaming logs, or omit --follow for one JSON document"
    );
    let service = if args.run.is_none() && args.worker.is_none() {
        Some(args.service.unwrap_or_else(|| "server".into()))
    } else {
        None
    };
    let request = Request::Logs {
        run: args.run,
        worker: args.worker,
        service,
        limit: args.limit,
        before_id: args.before_id,
    };
    let mut seen = std::collections::HashSet::new();
    loop {
        let value = client.request(&request).await?;
        if !args.follow && !args.jsonl {
            return print_value(&value, json);
        }
        for entry in value["items"].as_array().into_iter().flatten() {
            if seen.insert(entry["id"].to_string()) {
                if args.jsonl {
                    print_value(entry, true)?;
                } else {
                    println!("{}", entry["message"].as_str().unwrap_or(""));
                }
            }
        }
        if !args.follow {
            return Ok(());
        }
        // Only the most recent page needs duplicate suppression.
        seen = value["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| e["id"].to_string())
            .collect();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
pub fn print_value(value: &Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(value)?);
    } else if let Some(rows) = value.as_array() {
        if rows.is_empty() {
            println!("No entries.");
        } else {
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print_value(row, false)?;
            }
        }
    } else if let Some(object) = value.as_object() {
        for (key, value) in object {
            if key == "dashboard_url" {
                use std::io::IsTerminal;
                let link = value
                    .as_str()
                    .map(|url| {
                        terminal_link(
                            url,
                            std::io::stdout().is_terminal()
                                && std::env::var("TERM").as_deref() != Ok("dumb"),
                        )
                    })
                    .unwrap_or_else(|| "not served".into());
                println!("Dashboard: {link}");
                continue;
            }
            println!(
                "{key}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            );
        }
    } else {
        println!(
            "{}",
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        );
    }

    Ok(())
}
/// OSC 8 in interactive terminals; readable URLs in redirected output.
fn terminal_link(value: &str, interactive: bool) -> String {
    let Ok(url) = reqwest::Url::parse(value) else {
        return value.into();
    };
    if interactive && matches!(url.scheme(), "http" | "https") {
        format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\")
    } else {
        value.into()
    }
}

pub fn write_output(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|e| e.error)?;
    Ok(())
}
fn bootstrap(cli: &Cli) -> Result<()> {
    ensure!(
        cli.endpoint.is_none(),
        "this command operates on the local host; omit --endpoint and use a deployment configuration"
    );
    let path = cli
        .instance
        .as_deref()
        .unwrap_or(Path::new("gammaboard.toml"));
    let config: toml::Value =
        toml::from_str(&std::fs::read_to_string(path).with_context(|| {
            format!(
                "local operation needs deployment configuration {}",
                path.display()
            )
        })?)?;
    ensure!(
        config.get("database").is_some() && config.get("paths").is_some(),
        "local operations require a deployment configuration with [database] and [paths], not a client connection file"
    );
    let executable = service_executable()?;
    let mut cmd = std::process::Command::new(executable);
    cmd.arg("--instance")
        .arg(
            cli.instance
                .as_deref()
                .unwrap_or(Path::new("gammaboard.toml")),
        )
        .arg("--json");
    if cli.allow_local_processes {
        cmd.arg("--allow-local-processes");
    }
    if let Command::Local { command } = &cli.command {
        match command {
            Local::CheckConfig
            | Local::Config {
                command: LocalConfig::Validate,
            } => {
                cmd.arg("check-config");
            }
            Local::CheckRun { file, probe }
            | Local::Run {
                command: LocalRun::Validate { file, probe },
            } => {
                cmd.arg("check-run").arg(file);
                if *probe {
                    cmd.arg("--probe");
                }
            }
            Local::Db { command } => {
                cmd.arg("db");
                match command {
                    LocalDb::Status => {
                        cmd.arg("status");
                    }
                    LocalDb::Stop => {
                        cmd.arg("stop");
                    }
                    LocalDb::Create => {
                        cmd.arg("create");
                    }
                    LocalDb::Migrate => {
                        cmd.arg("migrate");
                    }
                    LocalDb::Backup { output } => {
                        cmd.arg("backup").arg("--output").arg(output);
                    }
                    LocalDb::Delete { yes } | LocalDb::Purge { yes } | LocalDb::Reset { yes } => {
                        ensure!(*yes, "destructive database maintenance requires --yes");
                        cmd.arg(match command {
                            LocalDb::Delete { .. } => "delete",
                            LocalDb::Purge { .. } => "purge",
                            _ => "reset",
                        })
                        .arg("--yes");
                    }
                    LocalDb::Restore { backup, yes } => {
                        ensure!(*yes, "database restore requires --yes");
                        cmd.arg("restore").arg(backup).arg("--yes");
                    }
                }
            }
        }
        return child_output(&mut cmd, cli.json);
    }
    let (name, component, args) = match &cli.command {
        Command::Start(a) => ("start", a.component.as_deref(), &a.options),
        Command::Restart(a) => ("restart", None, a),
        _ => unreachable!(),
    };
    cmd.arg(name);
    for origin in &args.allowed_origins {
        cmd.args(["--allowed-origin", origin]);
    }
    if let Some(c) = component {
        cmd.arg(c);
    }
    if let Some(n) = args.workers {
        cmd.args(["--workers", &n.to_string()]);
    }
    if args.wait {
        cmd.arg("--wait");
    }
    if args.restore_workers {
        cmd.arg("--restore-workers");
    }
    if args.foreground {
        cmd.arg("--foreground");
    }
    if args.api_only {
        cmd.arg("--api-only");
    }
    cmd.args(["--timeout", &format!("{}ms", args.timeout.as_millis())]);
    if args.foreground {
        ensure!(
            !cli.json,
            "foreground service logs are not a finite JSON response; omit --json"
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            Err(cmd.exec().into())
        }
        #[cfg(not(unix))]
        {
            let status = cmd.status()?;
            if !status.success() {
                return Err(crate::ChildExit(status.code().unwrap_or(1) as u8).into());
            }
            Ok(())
        }
    } else {
        child_output(&mut cmd, cli.json)
    }
}

fn child_output(cmd: &mut std::process::Command, json: bool) -> Result<()> {
    let output = cmd.output()?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        let message = error
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<Value>(line).ok())
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| error.trim().to_owned());
        ensure!(!message.is_empty(), "service exited with {}", output.status);
        anyhow::bail!("{message}");
    }
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    let value: Value =
        serde_json::from_slice(&output.stdout).context("service returned invalid JSON")?;
    print_value(&value, json)
}

/// An explicit override is authoritative; never silently fall back from it.
fn service_executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GAMMABOARD_SERVICE_BINARY") {
        let path = PathBuf::from(path);
        ensure!(
            is_executable(&path),
            "GAMMABOARD_SERVICE_BINARY points to a missing or non-executable file {}",
            path.display()
        );
        return Ok(path);
    }
    let sibling = std::env::current_exe()?.with_file_name("gammaboard");
    if is_executable(&sibling) {
        return Ok(sibling);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join("gammaboard");
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "service executable not found: tried {} and PATH. Install gammaboard beside gb or set GAMMABOARD_SERVICE_BINARY to its explicit path. Remote management commands only require gb.",
        sibling.display()
    )
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| {
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}
